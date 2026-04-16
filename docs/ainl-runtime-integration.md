# ainl-runtime integration (OpenFang / ArmaraOS)

This guide covers the embedded path that runs a Rust **`ainl-runtime`** orchestration **prelude** (`run_turn` / `run_turn_async`) before the default OpenFang LLM + tool loop. It complements:

- **`docs/ainl-runtime.md`** — hub: what the crate does, delegation depth / **`AinlRuntimeError`**, **`async`** feature, **`std::sync::Mutex`** vs **`tokio::sync::Mutex`**, verification commands.
- **`docs/ainl-runtime-graph-patch.md`** — procedural patch adapters, `MemoryContext`, and how **`GraphPatchAdapter`** fits the graph.
- **`crates/ainl-runtime/README.md`** — crate-level behavior, delegation depth, async SQLite notes.

In current builds, `openfang-runtime` compiles with `ainl-runtime-engine` enabled by default and `AgentManifest.ainl_runtime_engine` defaults to `true`. Effective routing still depends on runtime/env state (`AINL_RUNTIME_ENGINE`, `ARMARAOS_DISABLE_AINL_RUNTIME_ENGINE`) and graph-memory availability.

---

## Quick reference

| Topic | Detail |
|--------|--------|
| **Cargo feature** | `ainl-runtime-engine` on crate **`openfang-runtime`** |
| **Rust bridge** | `crates/openfang-runtime/src/ainl_runtime_bridge.rs` (`AinlRuntimeBridge`) |
| **Manifest** | Top-level `ainl_runtime_engine = true` (TOML / JSON on `AgentManifest`) |
| **Environment** | `AINL_RUNTIME_ENGINE=1` (process-wide; OR with manifest flag) |
| **Emergency off switch** | `ARMARAOS_DISABLE_AINL_RUNTIME_ENGINE=1|true|yes|on` forces the path off globally (wins over manifest + `AINL_RUNTIME_ENGINE=1`) |
| **Graph DB** | Same per-agent `ainl_memory.db` as `GraphMemoryWriter` (second SQLite connection; WAL-safe). **`AinlRuntime`** may upsert a **`runtime_state`** node (session counters + optional persona snapshot JSON) on each completed turn — see **`crates/ainl-runtime/README.md`** (*Session persistence*). |
| **Default agent loop graph toggles** | Unrelated to **`ainl-runtime-engine`** — the built-in loop uses **`AINL_EXTRACTOR_ENABLED`** (opt-out; default on), **`AINL_TAGGER_ENABLED`** (opt-in; must be `1`), **`AINL_PERSONA_EVOLUTION`** (opt-out; default on), and export path **`AINL_GRAPH_MEMORY_ARMARAOS_EXPORT`** as documented in **[graph-memory.md](graph-memory.md)** and **`crates/openfang-runtime/README.md`**. |
| **Evolution writes** | Bridge enforces `AinlRuntime::with_evolution_writes_enabled(false)` so OpenFang remains the sole evolution-row writer (constructor fails if this invariant is violated) |
| **Delegation cap** | `AinlRuntimeBridge::with_delegation_cap(..., runtime_limits.max_agent_call_depth)` wires **`RuntimeConfig::max_delegation_depth`**. Enforcement is **internal** on each nested **`run_turn`** / **`run_turn_async`**; **`TurnInput::depth`** does not raise the cap. Over limit → **`Err(AinlRuntimeError::DelegationDepthExceeded)`** (see **`crates/ainl-runtime/README.md`**, **[ainl-runtime.md](ainl-runtime.md)**). |
| **Tests** | `cargo test -p openfang-runtime --features ainl-runtime-engine test_agent_loop_uses_openfang_by_default` (single `cargo test` filter; alternatively `… ainl_runtime` to match bridge tests) |

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

**Daemon (`openfang-kernel` → `openfang-runtime`):** `openfang-runtime` includes **`ainl-runtime-engine`** in its default feature set, so standard production builds include the shim. Prelude routing is active when `(manifest.ainl_runtime_engine || AINL_RUNTIME_ENGINE=1) && !ARMARAOS_DISABLE_AINL_RUNTIME_ENGINE` and graph memory opens.

**Clippy:**

```bash
cargo clippy -p openfang-runtime --features ainl-runtime-engine --all-targets -- -D warnings
```

---

## Activation rules

Activation (when the feature is compiled in):

1. **`manifest.ainl_runtime_engine == true`**  
   Example TOML fragment:

   ```toml
   name = "my-agent"
   ainl_runtime_engine = true
   ```

2. **`AINL_RUNTIME_ENGINE=1`** in the daemon environment (applies process-wide; still requires graph memory — see below).

**Kill switch precedence:** if **`ARMARAOS_DISABLE_AINL_RUNTIME_ENGINE`** is truthy (`1` / `true` / `yes` / `on`), the path is forced off regardless of manifest/env opt-ins.

**AND** graph memory must open successfully (`GraphMemoryWriter` for `~/.armaraos/agents/<id>/ainl_memory.db` or equivalent under `ARMARAOS_HOME` / `OPENFANG_HOME`). If the writer is missing, the loop logs a warning and **continues with the normal OpenFang LLM path**.

---

## What happens on a routed turn

Rough order (non-streaming and streaming share the same pre-loop hook):

1. The usual preamble runs: memories, hooks, system prompt, efficient-mode compression, user message appended to the session, `llm_messages` / repair, trim, loop guard setup, **`loop_t0`**.
2. **Before** the first `for iteration in 0..max_iterations`, if the switch is active and `graph_memory` is `Some`, **`run_ainl_runtime_engine_prelude`** runs.
3. The prelude builds **`AinlRuntimeBridge::with_delegation_cap(Arc<Mutex<GraphMemoryWriter>>, max_agent_call_depth)`**, calls **`run_turn`** with `TurnInput` built from the (possibly compressed) user text and optional trace JSON from orchestration — so **`compile_memory_context_for`** inside **`run_turn`** receives that same text and **`MemoryContext::relevant_semantic`** is **topic-ranked** for this turn (see semantic ranking migration in **`docs/ainl-runtime-graph-patch.md`**; avoid assuming episode inheritance when calling **`compile_memory_context_for(None)`** elsewhere).
   The host reuses a cached bridge per `(agent_id, max_agent_call_depth)` to avoid per-turn SQLite bridge construction churn.
4. **`map_ainl_turn_outcome`** produces host **`TurnOutcome`**: `output`, `tool_calls`, `delegation_to`, `cost_estimate` (see below). **`log_mapped_end_turn_fields`** emits a structured **info** line.
5. The loop continues into the normal OpenFang LLM/tool path; assistant text comes from the model’s `EndTurn` response, not bridge output.

```mermaid
flowchart TD
  subgraph default [Default path]
    A[User message] --> B[OpenFang LLM loop]
    B --> C[Tools / approvals]
    C --> D[Assistant reply]
  end
  subgraph shim [ainl-runtime-engine prelude]
    E[User message] --> F{Switch + graph_memory?}
    F -->|no| B
    F -->|yes| G[AinlRuntimeBridge.run_turn]
    G --> H[Map + log telemetry]
    H --> B
  end
```

---

## EndTurn-shaped mapping and logs

The bridge does **not** emit the same internal events as an LLM **`StopReason::EndTurn`**. It maps **ainl-runtime** `TurnOutcome` into host output plus structured telemetry included in hook payloads and logs:

| Field | Source (today) |
|--------|----------------|
| **`output`** | Synthesized status line (`episode_id`, `TurnStatus`, `steps_executed`) for telemetry/log mapping (not user-facing chat text) |
| **`tool_calls`** | `TurnInput.tools_invoked` plus patch adapter names from `patch_dispatch_results` |
| **`delegation_to`** | From host **`TurnContext`** (episode rows from **ainl-runtime** still use `delegation_to: None` internally) |
| **`cost_estimate`** | Synthetic host estimate (`steps_executed` as `f64`) to keep telemetry shape stable on this path |
| **Telemetry fields** | `turn_status`, `partial_success`, warning count, extraction-report presence, memory-context counts, patch-dispatch counts, `steps_executed` |

**Warnings:** `map_ainl_turn_outcome` **`warn!`**s for **MemoryContext** slices, **`extraction_report`**, each **`TurnWarning`** (including **granular** extractor phases: **`TurnPhase::ExtractionPass`**, **`PatternPersistence`**, **`PersonaEvolution`** — one warning per populated **`ExtractionReport`** slot), non-OK **`TurnStatus`** (e.g. **`StepLimitExceeded`**, **`GraphMemoryDisabled`** — **not** delegation depth; depth over the cap fails **`run_turn`** earlier as **`AinlRuntimeError::DelegationDepthExceeded`**), and patch **`adapter_output`** blobs that do not yet have OpenFang equivalents.

---

## Known limitations

| Limitation | Notes |
|-------------|--------|
| **No tool / approval parity inside prelude** | Prelude itself is graph-memory runtime work only; OpenFang still runs the regular tool/approval loop afterward. |
| **Bridge output is non-user text** | `TurnOutcome.output` is for logs/telemetry mapping, not chat reply rendering. |
| **Second SQLite handle** | WAL-safe; avoid conflicting long transactions with `GraphMemoryWriter` on the same file. Both paths may write **`runtime_state`** (bridge / **`AinlRuntime`** only) and episode/persona rows (**OpenFang**); keep turns short. |
| **No extra multi-tenancy** | Same per-agent `agent_id` scoping as existing graph memory. |

---

## Convergence roadmap

1. **Single evolution writer** — explicit hand-off or read-only **ainl-runtime** on SQLite for `EVOLUTION_TRAIT_NAME`.
2. **Tool + approval parity** — optional phase: host hooks drive OpenFang tool runner from patch / emit results.
3. **Orchestration / dashboard** — feed mapped EndTurn fields into the same telemetry as LLM EndTurn.
4. **Streaming parity** — if **ainl-runtime** gains incremental summaries, replace synthetic delta emission with native incremental events.

Until then, prefer this path for **graph-first** agents where pre-loop graph telemetry helps memory quality before model inference.

---

## Related source files

| File | Role |
|------|------|
| `crates/openfang-runtime/src/ainl_runtime_bridge.rs` | Bridge, `TurnContext`, `TurnOutcome`, `run_turn` / `run_turn_async` |
| `crates/openfang-runtime/src/agent_loop.rs` | `run_ainl_runtime_engine_prelude`, call sites before main iteration loop |
| `crates/openfang-runtime/src/graph_memory_writer.rs` | `sqlite_database_path_for_agent` |
| `crates/openfang-types/src/agent.rs` | `AgentManifest.ainl_runtime_engine` |
| `crates/ainl-runtime/` | `AinlRuntime`, `TurnInput`, `TurnOutcome`, `GraphPatchAdapter` |

---

## Troubleshooting

| Symptom | Likely cause |
|---------|----------------|
| Path never triggers | Feature not enabled on **`openfang-runtime`**, or neither manifest flag nor `AINL_RUNTIME_ENGINE=1`, or graph memory failed to open. |
| Prelude skipped after “failed to construct bridge” | `try_lock` on `GraphMemoryWriter` failed (rare), or `SqliteGraphStore::open` failed — check paths and permissions. |
| Falls back after `run_turn failed` | Graph validation failed (e.g. dangling edges), empty `agent_id`, **`AinlRuntimeError::DelegationDepthExceeded`** (too many nested **`run_turn`** entries vs **`max_delegation_depth`**), or other **`AinlRuntimeError::Message`** — see logs with `error =`. |
| Duplicate / confusing persona rows | Do not enable **`evolution_writes_enabled`** on embedded **ainl-runtime** without coordinating with OpenFang’s persona pass (bridge keeps it **false**). |

---

## Changelog discipline

When you change activation rules, mapping, or defaults, update this doc and **`docs/ainl-runtime-graph-patch.md`** (if the patch story moves) in the same PR so operators and embedders stay aligned.
