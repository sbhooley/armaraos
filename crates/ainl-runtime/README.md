# ainl-runtime

**Alpha (0.3.5-alpha) — API subject to change.**

**Documentation map (ArmaraOS repo):** **[`docs/ainl-runtime.md`](../../docs/ainl-runtime.md)** (hub: features, testing, **`std::sync::Mutex`** rationale), **[`docs/ainl-runtime-graph-patch.md`](../../docs/ainl-runtime-graph-patch.md)** (GraphPatch / patch adapters), **[`docs/graph-memory.md`](../../docs/graph-memory.md)** (live daemon **`GraphMemoryWriter`** vs this crate), root **[`ARCHITECTURE.md`](../../ARCHITECTURE.md)** (layering).

`ainl-runtime` is the **Rust orchestration layer** for the unified AINL **graph memory** stack: it coordinates [`ainl-memory`](https://crates.io/crates/ainl-memory), [`ainl-persona`](https://crates.io/crates/ainl-persona)’s [`EvolutionEngine`](https://docs.rs/ainl-persona/latest/ainl_persona/struct.EvolutionEngine.html) (shared with [`ainl-graph-extractor`](https://crates.io/crates/ainl-graph-extractor)’s [`GraphExtractorTask`](https://docs.rs/ainl-graph-extractor/latest/ainl_graph_extractor/struct.GraphExtractorTask.html)), and optional **post-turn extraction**, with a [`TurnHooks`] seam for hosts (e.g. OpenFang).

It is **not** the Python `RuntimeEngine`, **not** the MCP server, **not** the AINL CLI, and **not** an LLM or IR parser.

## What v0.3 provides (beyond v0.2)

### Turn outcomes, warnings, and phases

- **`run_turn` / `run_turn_async`** return **`Result<TurnOutcome, AinlRuntimeError>`** — not a bare `TurnResult`. Use **`TurnOutcome::Complete`** vs **`PartialSuccess`** (non-fatal write failures still return a full **`TurnResult`** plus **`Vec<TurnWarning>`** tagged with **`TurnPhase`**).
- **`TurnPhase`** (exported at crate root; see rustdoc) covers episode edges, fitness write-back, **granular graph extraction**, export refresh, and session row persist:
  - **`EpisodeWrite`**, **`FitnessWriteBack`**, **`ExtractionPass`**, **`PatternPersistence`**, **`PersonaEvolution`**, **`ExportRefresh`**, **`RuntimeStatePersist`**
- When the scheduled **`GraphExtractorTask::run_pass`** runs (`extraction_interval` cadence), **`ainl_graph_extractor::ExtractionReport`** fields map to warnings as:
  - **`extract_error`** → **`TurnPhase::ExtractionPass`**
  - **`pattern_error`** → **`TurnPhase::PatternPersistence`**
  - **`persona_error`** → **`TurnPhase::PersonaEvolution`**
  so **`TurnOutcome::PartialSuccess`** can carry **multiple** extraction-related warnings in one turn. The report is still attached to **`TurnResult::extraction_report`** when the pass ran (see tests **`test_turn_phase_granularity`**).

### Delegation depth

- **Internal depth guard** — nested **`run_turn`** calls increment a counter; beyond **`RuntimeConfig::max_delegation_depth`** you get **`AinlRuntimeError::DelegationDepthExceeded`** (hard error). **`TurnInput::depth`** remains metadata for logging only.

### Session persistence (`RuntimeStateNode`)

- **Where it lives** — one upserted graph row per agent in **`ainl_memory.db`** (`node_type = runtime_state`), written through **`GraphMemory::write_runtime_state`** (backed by **`SqliteGraphStore::write_runtime_state`**). **`AinlRuntime::new`** calls **`GraphMemory::read_runtime_state`** before the first turn.
- **Fields** — **`turn_count`** and **`last_extraction_at_turn`** (`u64`) keep scheduled **`GraphExtractorTask::run_pass`** aligned across process restarts. **`persona_snapshot_json`** holds **`serde_json::to_string`** output of the compiled persona contribution string (restore with **`serde_json::from_str::<String>`**) so the first post-restart turn can reuse the in-memory cache without re-querying persona nodes.
- **Failures** — SQLite persist errors are **non-fatal**: the turn still completes, but you get **`TurnOutcome::PartialSuccess`** with a **`TurnWarning`** whose **`TurnPhase::RuntimeStatePersist`** explains the error (cadence resets on next cold start if the row never landed).
- **Tests** — `cargo test -p ainl-runtime --test test_session_persistence` (restart simulation on a shared temp DB).

### Topic relevance (`MemoryContext::relevant_semantic`)

- **Ranking** — when you pass a non-empty message into **`compile_memory_context_for(Some(...))`** or use **`run_turn`** (which always passes the current user text), **`relevant_semantic`** is ordered with **`ainl_semantic_tagger::infer_topic_tags`** overlap on each node’s **`topic_cluster` / `topic:` tags**, with **`recurrence_count`** as a tiebreaker; empty text or no inferred topic tags falls back to high-recurrence semantic selection. Crate re-exports **`infer_topic_tags`** for tests and tooling.
- **Migration** — see **Memory context / semantic ranking** below: **`compile_memory_context_for(None)`** does **not** reuse the latest episode body for ranking.

### Procedural patches (`PatchAdapter` + `GraphPatchAdapter`)

- **[`PatchAdapter`] + [`AdapterRegistry`]** — label-keyed **`execute_patch(&PatchDispatchContext)`**; register hosts with **`AinlRuntime::register_adapter`**. **`PatchDispatchResult`** includes **`adapter_name`** / **`adapter_output`** when an adapter succeeds.
- **Reference [`GraphPatchAdapter`]** (`"graph_patch"`) — built-in fallback; returns a small JSON summary **`{ "label", "patch_version", "frame_keys" }`** (with declared-read safety checks). Does **not** compile or run AINL IR in Rust.
- **[`PatchDispatchContext`]** — node + frame passed into **`execute_patch`**.
- **Fallback dispatch** — if no adapter matches the procedural **label**, `run_turn` uses the registered **`graph_patch`** adapter when present (install with [`AinlRuntime::register_default_patch_adapters`]).
- **Optional host hook** — [`GraphPatchAdapter::with_host`] + [`GraphPatchHostDispatch`] forwards that same summary JSON to another runtime (e.g. Python GraphPatch).

**Limits (honest):** Rust GraphPatch support is **host-dispatch / extraction only**. Python-side GraphPatch (full `memory.patch`, IR promotion, overwrite guards, engine integration) remains the rich path until a future convergence milestone.

ArmaraOS integration docs (repo root): **[`docs/ainl-runtime-graph-patch.md`](../../docs/ainl-runtime-graph-patch.md)** (patch adapters + `MemoryContext`), **[`docs/ainl-runtime-integration.md`](../../docs/ainl-runtime-integration.md)** (optional **`openfang-runtime`** `ainl-runtime-engine` chat shim: manifest / env, build, limits).

## What v0.2 still provides

- **[`AinlRuntime`]** — owns a [`ainl_memory::GraphMemory`] over a [`SqliteGraphStore`], a stateful [`GraphExtractorTask`], and [`RuntimeConfig`].
- **Persona evolution (direct)** — [`AinlRuntime::evolution_engine`] / [`AinlRuntime::evolution_engine_mut`], [`AinlRuntime::apply_evolution_signals`], [`AinlRuntime::evolution_correction_tick`], [`AinlRuntime::persist_evolution_snapshot`], [`AinlRuntime::evolve_persona_from_graph_signals`] (`EvolutionEngine` lives in **ainl-persona**; the extractor is an additional signal source, not a hard gate).
- **Boot** — [`AinlRuntime::load_artifact`] → [`AinlGraphArtifact`] (`export_graph` + `validate_graph`; fails on dangling edges).
- **Turn pipeline** — [`AinlRuntime::run_turn`]: validate subgraph, compile persona lines from persona nodes, [`compile_memory_context`], **procedural patch dispatch** (declared-read gating + fitness EMA), record an episodic node (user message + tools), [`TurnHooks::on_emit`] for `EMIT_TO` edges, run extractor every `extraction_interval` turns.
- **Legacy API** — [`RuntimeContext`] + `record_*` + [`RuntimeContext::run_graph_extraction_pass`] returns **`Result<ExtractionReport, String>`** for config/memory errors only; the inner extractor still returns a report (per-phase errors live on **`ExtractionReport`**, use **`has_errors()`**).

It still does **not** execute arbitrary AINL IR in Rust; hosts wire LLM/tools on top of [`TurnOutcome`] / [`MemoryContext`] / patch adapter JSON.

## Memory context / semantic ranking (migration)

**`compile_memory_context_for(None)`** no longer inherits previous episode text for semantic ranking; pass **`Some(user_message)`** if you want topic-aware ranking.

[`compile_memory_context`](https://docs.rs/ainl-runtime/latest/ainl_runtime/struct.AinlRuntime.html#method.compile_memory_context) still calls `compile_memory_context_for(None)` — that path now behaves like an **empty** user message (high-recurrence fallback for [`MemoryContext::relevant_semantic`](https://docs.rs/ainl-runtime/latest/ainl_runtime/struct.MemoryContext.html#structfield.relevant_semantic)), not “reuse the last episode body.” [`run_turn`](https://docs.rs/ainl-runtime/latest/ainl_runtime/struct.AinlRuntime.html#method.run_turn) always passes the current turn’s `user_message` into memory compilation, so embedded turn pipelines keep topic-aware semantics without extra calls.

## Optional Tokio API (`async` feature)

Enable **`features = ["async"]`** for [`AinlRuntime::run_turn_async`], [`TurnHooksAsync`], and Tokio (`spawn_blocking` for SQLite / graph work).

**Why `std::sync::Mutex`, not `tokio::sync::Mutex`, for graph memory?** With an async mutex, calling [`AinlRuntime::new`] or [`AinlRuntime::sqlite_store`] from a Tokio worker (including `#[tokio::test]`) would push you toward `blocking_lock` or cross-thread deadlocks when the “short lock” path blocks the executor. The async path instead keeps the graph in `Arc<std::sync::Mutex<GraphMemory>>` and confines **heavy** SQLite and graph mutation to `tokio::task::spawn_blocking`, which matches how `openfang-runtime` callers already isolate blocking work.

**Minimal async example** (body of an `async fn`; Cargo: `ainl-runtime = { version = "…", features = ["async"] }`):

```rust
use std::sync::Arc;
use ainl_memory::SqliteGraphStore;
use ainl_runtime::{AinlRuntime, NoOpAsyncHooks, RuntimeConfig, TurnHooksAsync, TurnInput};

let store = SqliteGraphStore::open(std::path::Path::new("memory.db"))?;
let cfg = RuntimeConfig {
    agent_id: "agent-1".into(),
    ..Default::default()
};
let hooks: Arc<dyn TurnHooksAsync> = Arc::new(NoOpAsyncHooks);
let mut rt = AinlRuntime::new(cfg, store).with_hooks_async(hooks);
let _out = rt.run_turn_async(TurnInput::default()).await?;
```

## Quick start (`AinlRuntime`)

```toml
[dependencies]
ainl-runtime = "0.3.5-alpha"
```

```rust
use ainl_runtime::{AinlRuntime, RuntimeConfig, TurnInput};
use ainl_memory::SqliteGraphStore;

let store = SqliteGraphStore::open(std::path::Path::new("memory.db"))?;
let cfg = RuntimeConfig {
    agent_id: "my-agent".into(),
    extraction_interval: 10,
    ..Default::default()
};
let mut rt = AinlRuntime::new(cfg, store);
rt.register_default_patch_adapters(); // GraphPatch fallback for procedural patches
let _artifact = rt.load_artifact()?;
// Topic-aware semantic slice: pass Some(...). None = empty ranking input (not last episode).
let _ctx = rt.compile_memory_context_for(Some("What did we decide about Rust?"))?;
let out = rt.run_turn(TurnInput {
    user_message: "Hello".into(),
    tools_invoked: vec!["file_read".into()],
    ..Default::default()
})?;
```

## `RuntimeConfig`

- **`agent_id`**: `String` (empty disables graph extraction on [`RuntimeContext`]; required for [`AinlRuntime`] turns).
- **`max_delegation_depth`**: max nested [`AinlRuntime::run_turn`] entries tracked internally (default `8`); exceeded depth returns [`AinlRuntimeError::DelegationDepthExceeded`] (not [`TurnInput::depth`], which is metadata only).
- **`max_steps`**: cap for the exploratory BFS in `run_turn` (default `1000`).
- **`extraction_interval`**: run `GraphExtractorTask::run_pass` every N turns (`0` = never).

## `AinlRuntimeError` (hard failures from `run_turn`)

- **`Message(String)`** — store / validation / config failures; use **`message_str()`** for a borrowed view, or **`From<String>`** / **`?`** when chaining.
- **`DelegationDepthExceeded { depth, max }`** — nested `run_turn` past **`max_delegation_depth`**; use **`is_delegation_depth_exceeded()`** or **`delegation_depth_exceeded()`** instead of matching on `TurnStatus` (there is no soft depth outcome).
- **`AsyncJoinError` / `AsyncStoreError`** — only with the **`async`** feature, from **`run_turn_async`**: blocking-pool join failure or SQLite error inside **`spawn_blocking`** (graph mutex remains **`std::sync::Mutex`**; see above).

## Persona evolution and ArmaraOS (OpenFang)

**Target convergence:** `AinlRuntime`’s evolution engine (`EvolutionEngine` + scheduled `GraphExtractorTask::run_pass`) is the intended long-term convergence point for graph-driven persona persistence in the Rust stack.

**Today:** Until ArmaraOS migrates to **ainl-runtime** as its primary execution engine, **openfang-runtime**’s `GraphMemoryWriter::run_persona_evolution_pass` is the **active** evolution write path for dashboard agents (`~/.armaraos/agents/<id>/ainl_memory.db`). Do not call `AinlRuntime::persist_evolution_snapshot` or `AinlRuntime::evolve_persona_from_graph_signals` on that same database concurrently with that pass. If you embed `AinlRuntime` next to openfang while openfang still owns evolution, chain `AinlRuntime::with_evolution_writes_enabled(false)` so those two methods return an error instead of writing.

## crates.io stack (registry consumers)

If you depend on **`ainl-runtime` = "0.3.5-alpha"`** from crates.io, let Cargo pick matching releases: **`ainl-memory` 0.1.8-alpha**, **`ainl-persona` 0.1.4**, **`ainl-graph-extractor` 0.1.5**, **`ainl-semantic-tagger` 0.1.2-alpha**. Older **`ainl-persona` 0.1.3** cannot pair with **`ainl-memory` 0.1.8-alpha** in the same graph (resolver conflict). See **`docs/ainl-runtime-graph-patch.md`** (dependency table).

## License

MIT OR Apache-2.0
