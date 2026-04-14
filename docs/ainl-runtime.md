# ainl-runtime — documentation hub

The **`ainl-runtime`** crate is a **standalone Rust orchestration layer** over the same SQLite graph as the daemon (**`ainl-memory`** / **`ainl_memory.db`**). It is **not** the Python AINL `RuntimeEngine`, **not** the MCP server, and **not** the default ArmaraOS chat path today — **`openfang-runtime`** owns live execution. Use **`ainl-runtime`** for tests, tooling, or optional embedding behind feature **`ainl-runtime-engine`** (see **[ainl-runtime-integration.md](ainl-runtime-integration.md)**).

## Where to read next

| Topic | Doc |
|--------|-----|
| API, **`run_turn`** / **`run_turn_async`**, **`TurnOutcome`**, session **`runtime_state`**, mutex vs Tokio | **`crates/ainl-runtime/README.md`** |
| **`PatchAdapter`**, procedural rows, GraphPatch summaries, semantic ranking, crates.io alignment | **[ainl-runtime-graph-patch.md](ainl-runtime-graph-patch.md)** |
| Optional **`AinlRuntimeBridge`** in **`openfang-runtime`**, manifest / env | **[ainl-runtime-integration.md](ainl-runtime-integration.md)** |
| Live daemon **`GraphMemoryWriter`**, node kinds, on-disk layout | **[graph-memory.md](graph-memory.md)** |
| Post-turn persona evolution / **`AINL_PERSONA_EVOLUTION`** | **[persona-evolution.md](persona-evolution.md)** |

---

## What it does

| API | When to use |
|-----|----------------|
| **`AinlRuntime::run_turn`** | Synchronous single-turn pipeline: validate subgraph, compile **`MemoryContext`**, dispatch procedural patches via **`PatchAdapter`**, record episodes, optional **`GraphExtractorTask::run_pass`**, sync **`TurnHooks`**. |
| **`AinlRuntime::run_turn_async`** | Same semantics off the Tokio async executor: enable crate feature **`async`**, run SQLite-heavy work on **`tokio::task::spawn_blocking`**, optional **`TurnHooksAsync`**. |

Both return **`Result<TurnOutcome, AinlRuntimeError>`** (**`Complete`** vs **`PartialSuccess`** with **`TurnWarning`** + **`TurnPhase`**). **`RuntimeConfig::max_delegation_depth`** applies to nested **`run_turn`** / **`run_turn_async`** the same way.

**Semantic ranking:** **`compile_memory_context_for(None)`** does not inherit the latest episode body for **`MemoryContext::relevant_semantic`**; pass **`Some(user_message)`** for topic-aware ranking. **`run_turn`** / **`run_turn_async`** pass the current turn text. Details: **[ainl-runtime-graph-patch.md](ainl-runtime-graph-patch.md)** (*Memory context / semantic ranking*).

---

## Session persistence (`RuntimeStateNode`)

**`AinlRuntime`** restores and persists one **`runtime_state`** graph row per agent: **`turn_count`**, **`last_extraction_at_turn`**, optional **`persona_snapshot_json`**, **`updated_at`**. Read in **`AinlRuntime::new`**; written at end of **`run_turn`** / **`run_turn_async`**. SQLite failures are non-fatal (**`TurnOutcome::PartialSuccess`**, **`TurnPhase::RuntimeStatePersist`**). See **`crates/ainl-runtime/README.md`** (*Session persistence*) and **[graph-memory.md](graph-memory.md)**.

---

## Persona evolution pass (`ExtractionReport`)

**`ainl-graph-extractor`** **`GraphExtractorTask::run_pass`** returns **`ExtractionReport`** (per-phase errors, **`has_errors`**). **`openfang-runtime`** **`GraphMemoryWriter::run_persona_evolution_pass`** surfaces the same shape for logging and tests. See **`crates/ainl-graph-extractor`** crate docs and **[persona-evolution.md](persona-evolution.md)**.

---

## Cargo feature `async`

```toml
[dependencies]
ainl-runtime = { version = "…", features = ["async"] }
```

Pulls in **`async-trait`** and **`tokio`** (`rt`, `macros`, `fs`, `sync`). Default features are **empty** so sync-only dependents pay no Tokio cost.

---

## Why graph memory uses `std::sync::Mutex`, not `tokio::sync::Mutex`

With the **`async`** feature, graph state is **`Arc<std::sync::Mutex<ainl_memory::GraphMemory>>`** (see **`crates/ainl-runtime/src/graph_cell.rs`**).

- **`AinlRuntime::new`** and **`sqlite_store()`** may run on **any** OS thread, including a Tokio **worker** running **`#[tokio::test]`**. Using **`tokio::sync::Mutex`** for that inner lock pushes embedders toward **`blocking_lock`** or “cannot block inside async context” failure modes when the runtime detects blocking work on the executor.
- **Heavy** SQLite reads/writes for **`run_turn_async`** run inside **`tokio::task::spawn_blocking`** closures, matching how **`openfang-runtime`** isolates blocking **`GraphMemory`** work from async tasks.

Full rationale: **`crates/ainl-runtime/README.md`** (*Optional Tokio API*).

---

## Hooks: sync vs async

| Trait | Used by | Notes |
|-------|---------|--------|
| **`TurnHooks`** | **`run_turn`** | **`Send + Sync`**, synchronous callbacks. |
| **`TurnHooksAsync`** | **`run_turn_async`** | **`#[async_trait]`**; install with **`AinlRuntime::with_hooks_async`**. Sync hooks remain available in parallel. |

---

## Related ArmaraOS docs

| Doc | Topic |
|-----|--------|
| **[ainl-runtime-graph-patch.md](ainl-runtime-graph-patch.md)** | **`PatchAdapter`** registry, **`GraphPatchAdapter`** fallback, **`PatchDispatchResult`**, host summaries; **crates.io** version matrix. |
| **[ainl-runtime-integration.md](ainl-runtime-integration.md)** | Feature **`ainl-runtime-engine`**, **`AinlRuntimeBridge`**, **`TurnOutcome`** mapping. |
| **[graph-memory.md](graph-memory.md)** | **`GraphMemoryWriter`**, **`ainl_memory.db`**, Python inbox, **`runtime_state`**. |
| **[persona-evolution.md](persona-evolution.md)** | **`AINL_PERSONA_EVOLUTION`**, **`run_persona_evolution_pass`** / **`ExtractionReport`**. |
| **[architecture.md](architecture.md)** | Workspace crate table (**`ainl-runtime`** row mentions **`async`**). |
| Root **[ARCHITECTURE.md](../ARCHITECTURE.md)** | Layer 3 graph substrate; execution engine vs **`ainl-runtime`** roadmap. |

---

## Verification (developers)

From the repo root:

```bash
# Default features (no Tokio inside ainl-runtime)
cargo test -p ainl-runtime
cargo clippy -p ainl-runtime --all-targets -- -D warnings

# Async API + tests/test_async_runtime.rs
cargo test -p ainl-runtime --features async
cargo clippy -p ainl-runtime --all-targets --features async -- -D warnings

# Session persistence integration test (explicit test target)
cargo test -p ainl-runtime --test test_session_persistence
```

The **`test_async_runtime`** target uses **`required-features = ["async"]`** in **`crates/ainl-runtime/Cargo.toml`**, so **`cargo test --workspace`** skips it unless the workspace enables **`ainl-runtime/async`**. CI that must cover async turns should run **`cargo test -p ainl-runtime --features async`**.

Optional OpenFang bridge:

```bash
cargo test -p openfang-runtime --features ainl-runtime-engine ainl_runtime
```

---

## Future: openfang-runtime embedding

When **`openfang-runtime`** embeds **`AinlRuntime`**, respect **single-writer** rules for persona evolution rows (see **`GraphMemoryWriter::run_persona_evolution_pass`** rustdoc and **`crates/ainl-runtime/README.md`** *Persona evolution and ArmaraOS*). The **`async`** path does not change SQLite file locking; it only moves **where** blocking graph work runs relative to the Tokio executor.
