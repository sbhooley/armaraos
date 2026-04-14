# ainl-runtime integration (OpenFang / ArmaraOS)

This guide covers the **optional** path that routes a **chat turn** through the Rust **`ainl-runtime`** orchestration layer (`run_turn` / `run_turn_async`) instead of the default OpenFang LLM + tool loop. It complements:

- **`docs/ainl-runtime.md`** — hub: what the crate does, **`async`** feature, **`std::sync::Mutex`** vs **`tokio::sync::Mutex`**, verification commands.
- **`docs/ainl-runtime-graph-patch.md`** — procedural patch adapters, `MemoryContext`, and how **`GraphPatchAdapter`** fits the graph.
- **`crates/ainl-runtime/README.md`** — crate-level behavior, delegation depth, async SQLite notes.

The built-in OpenFang agent loop remains the **default**. Unless the Cargo feature **and** a runtime switch are enabled, behavior is unchanged.

---

## Quick reference

| Topic | Detail |
|--------|--------|
| **Cargo feature** | `ainl-runtime-engine` on crate **`openfang-runtime`** |
| **Rust bridge** | `crates/openfang-runtime/src/ainl_runtime_bridge.rs` (`AinlRuntimeBridge`) |
| **Manifest** | Top-level `ainl_runtime_engine = true` (TOML / JSON on `AgentManifest`) |
| **Environment** | `AINL_RUNTIME_ENGINE=1` (process-wide; OR with manifest flag) |
| **Graph DB** | Same per-agent `ainl_memory.db` as `GraphMemoryWriter` (second SQLite connection; WAL-safe). **`AinlRuntime`** may upsert a **`runtime_state`** node (session counters + optional persona snapshot JSON) on each completed turn — see **`crates/ainl-runtime/README.md`** (*Session persistence*). |
| **Default agent loop graph toggles** | Unrelated to **`ainl-runtime-engine`** — the built-in loop uses **`AINL_EXTRACTOR_ENABLED`**, **`AINL_TAGGER_ENABLED`**, **`AINL_PERSONA_EVOLUTION`**, and export path **`AINL_GRAPH_MEMORY_ARMARAOS_EXPORT`** as documented in **[graph-memory.md](graph-memory.md)** and **`crates/openfang-runtime/README.md`**. |
| **Evolution writes** | Bridge uses `AinlRuntime::with_evolution_writes_enabled(false)` so OpenFang keeps owning evolution row writes |
| **Delegation cap** | `AinlRuntimeBridge::with_delegation_cap(..., runtime_limits.max_agent_call_depth)` wires **`RuntimeConfig::max_delegation_depth`**. Enforcement is **internal** on each nested **`run_turn`** / **`run_turn_async`**; **`TurnInput::depth`** does not raise the cap. Over limit → **`Err(AinlRuntimeError::DelegationDepthExceeded)`** (see **`crates/ainl-runtime/README.md`**, **[ainl-runtime.md](ainl-runtime.md)**). |
| **Tests** | `cargo test -p openfang-runtime --features ainl-runtime-engine ainl_runtime test_agent_loop_uses_openfang_by_default` |

---

## Why the workspace uses a path dependency

**`openfang-runtime`** depends on **`ainl-runtime`** with `path = "../ainl-runtime"` and a **`version =`** field for crates.io metadata. The ArmaraOS repo already builds **`ainl-memory`**, **`ainl-persona`**, and **`ainl-graph-extractor`** from **workspace paths**. Pulling **`ainl-runtime`** from crates.io **only** would pull **registry** copies of those crates, so types like `SqliteGraphStore` would no longer match — the build fails with “multiple versions of crate `ainl_memory`”. The path keeps **one** dependency graph.

For **downstream** forks that do not vendor `ainl-runtime`, align versions with the published **`ainl-runtime`** release so `ainl-memory` / `ainl-persona` / `ainl-graph-extractor` / `ainl-semantic-tagger` resolve cleanly. For **0.3.5-alpha**, use the version table in **`docs/ainl-runtime-graph-patch.md`** (**crates.io dependency alignment**).

---

## Build and ship

**Library / tests:**

```bash
cargo build -p openfang-runtime --features ainl-runtime-engine
cargo test -p openfang-runtime --features ainl-runtime-engine ainl_runtime
```

**Daemon (`openfang-kernel` → `openfang-runtime`):** the kernel depends on `openfang-runtime` **without** listing `ainl-runtime-engine` today, so **release binaries do not** include the shim unless you change that dependency (e.g. `openfang-runtime = { path = "...", features = ["ainl-runtime-engine"] }`) or use a workspace feature that forwards it. Treat **`ainl-runtime-engine`** as **opt-in** for custom builds until product decides to enable it by default.

**Clippy:**

```bash
cargo clippy -p openfang-runtime --features ainl-runtime-engine --all-targets -- -D warnings
```

---

## Activation rules

Either condition can turn the path on (when the feature is compiled in):

1. **`manifest.ainl_runtime_engine == true`**  
   Example TOML fragment:

   ```toml
   name = "my-agent"
   ainl_runtime_engine = true
   ```

2. **`AINL_RUNTIME_ENGINE=1`** in the daemon environment (applies to agents that would otherwise use the normal loop; still requires graph memory — see below).

**AND** graph memory must open successfully (`GraphMemoryWriter` for `~/.armaraos/agents/<id>/ainl_memory.db` or equivalent under `ARMARAOS_HOME` / `OPENFANG_HOME`). If the writer is missing, the loop logs a warning and **continues with the normal OpenFang LLM path**.

---

## What happens on a routed turn

Rough order (non-streaming and streaming share the same early exit):

1. The usual preamble runs: memories, hooks, system prompt, efficient-mode compression, user message appended to the session, `llm_messages` / repair, trim, loop guard setup, **`loop_t0`**.
2. **Before** the first `for iteration in 0..max_iterations`, if the switch is active and `graph_memory` is `Some`, **`try_consume_turn_via_ainl_runtime`** runs.
3. It builds **`AinlRuntimeBridge::with_delegation_cap(Arc<Mutex<GraphMemoryWriter>>, max_agent_call_depth)`**, calls **`run_turn`** with `TurnInput` built from the (possibly compressed) user text and optional trace JSON from orchestration — so **`compile_memory_context_for`** inside **`run_turn`** receives that same text and **`MemoryContext::relevant_semantic`** is **topic-ranked** for this turn (see semantic ranking migration in **`docs/ainl-runtime-graph-patch.md`**; avoid assuming episode inheritance when calling **`compile_memory_context_for(None)`** elsewhere).
4. **`map_ainl_turn_outcome`** produces host **`TurnOutcome`**: `output`, `tool_calls`, `delegation_to`, `cost_estimate` (see below). **`log_mapped_end_turn_fields`** emits a structured **info** line.
5. Assistant message is appended, session saved, **`HookEvent::AgentLoopEnd`** fired, **`AgentLoopResult`** returned with **no LLM token usage** and **`iterations: 1`**.

```mermaid
flowchart TD
  subgraph default [Default path]
    A[User message] --> B[OpenFang LLM loop]
    B --> C[Tools / approvals]
    C --> D[Assistant reply]
  end
  subgraph shim [ainl-runtime-engine path]
    E[User message] --> F{Switch + graph_memory?}
    F -->|no| B
    F -->|yes| G[AinlRuntimeBridge.run_turn]
    G --> H[Map TurnOutcome]
    H --> I[Assistant text + save session]
  end
```

---

## EndTurn-shaped mapping and logs

The bridge does **not** emit the same internal events as an LLM **`StopReason::EndTurn`**. It **does** map **ainl-runtime** `TurnOutcome` into a small struct for logging and future dashboard parity:

| Field | Source (today) |
|--------|----------------|
| **`output`** | Persona prompt contribution text, or a synthesized status line (`episode_id`, `TurnStatus`, `steps_executed`) |
| **`tool_calls`** | `TurnInput.tools_invoked` plus patch adapter names from `patch_dispatch_results` |
| **`delegation_to`** | From host **`TurnContext`** (episode rows from **ainl-runtime** still use `delegation_to: None` internally) |
| **`cost_estimate`** | Always **`None`** — no token meter on this path |

**Warnings:** `map_ainl_turn_outcome` **`warn!`**s for **MemoryContext** slices, **`extraction_report`**, each **`TurnWarning`** (including **granular** extractor phases: **`TurnPhase::ExtractionPass`**, **`PatternPersistence`**, **`PersonaEvolution`** — one warning per populated **`ExtractionReport`** slot), non-OK **`TurnStatus`** (e.g. **`StepLimitExceeded`**, **`GraphMemoryDisabled`** — **not** delegation depth; depth over the cap fails **`run_turn`** earlier as **`AinlRuntimeError::DelegationDepthExceeded`**), and patch **`adapter_output`** blobs that do not yet have OpenFang equivalents.

---

## Known limitations

| Limitation | Notes |
|-------------|--------|
| **No LLM** | No model call; reply text is graph / persona summary only. |
| **No tool / approval loop** | No `shell_exec`, approvals, or kernel tool policy on this path. |
| **No streaming tokens** | Early return does not drive `StreamEvent` token deltas; clients see a single completed turn after save. |
| **Second SQLite handle** | WAL-safe; avoid conflicting long transactions with `GraphMemoryWriter` on the same file. Both paths may write **`runtime_state`** (bridge / **`AinlRuntime`** only) and episode/persona rows (**OpenFang**); keep turns short. |
| **No extra multi-tenancy** | Same per-agent `agent_id` scoping as existing graph memory. |

---

## Convergence roadmap

1. **Single evolution writer** — explicit hand-off or read-only **ainl-runtime** on SQLite for `EVOLUTION_TRAIT_NAME`.
2. **Tool + approval parity** — optional phase: host hooks drive OpenFang tool runner from patch / emit results.
3. **Orchestration / dashboard** — feed mapped EndTurn fields into the same telemetry as LLM EndTurn.
4. **Streaming** — if **ainl-runtime** gains incremental summaries, wire `StreamEvent`s.

Until then, use this path only for **experimental graph-first** agents that do not need a normal model reply.

---

## Related source files

| File | Role |
|------|------|
| `crates/openfang-runtime/src/ainl_runtime_bridge.rs` | Bridge, `TurnContext`, `TurnOutcome`, `run_turn` / `run_turn_async` |
| `crates/openfang-runtime/src/agent_loop.rs` | `try_consume_turn_via_ainl_runtime`, call sites before main iteration loop |
| `crates/openfang-runtime/src/graph_memory_writer.rs` | `sqlite_database_path_for_agent` |
| `crates/openfang-types/src/agent.rs` | `AgentManifest.ainl_runtime_engine` |
| `crates/ainl-runtime/` | `AinlRuntime`, `TurnInput`, `TurnOutcome`, `GraphPatchAdapter` |

---

## Troubleshooting

| Symptom | Likely cause |
|---------|----------------|
| Path never triggers | Feature not enabled on **`openfang-runtime`**, or neither manifest flag nor `AINL_RUNTIME_ENGINE=1`, or graph memory failed to open. |
| Falls back after “failed to construct bridge” | `try_lock` on `GraphMemoryWriter` failed (rare), or `SqliteGraphStore::open` failed — check paths and permissions. |
| Falls back after `run_turn failed` | Graph validation failed (e.g. dangling edges), empty `agent_id`, **`AinlRuntimeError::DelegationDepthExceeded`** (too many nested **`run_turn`** entries vs **`max_delegation_depth`**), or other **`AinlRuntimeError::Message`** — see logs with `error =`. |
| Duplicate / confusing persona rows | Do not enable **`evolution_writes_enabled`** on embedded **ainl-runtime** without coordinating with OpenFang’s persona pass (bridge keeps it **false**). |

---

## Changelog discipline

When you change activation rules, mapping, or defaults, update this doc and **`docs/ainl-runtime-graph-patch.md`** (if the patch story moves) in the same PR so operators and embedders stay aligned.
