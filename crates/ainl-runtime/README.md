# ainl-runtime

**Alpha (0.2.0-alpha) — API subject to change.**

`ainl-runtime` is the **Rust orchestration layer** for the unified AINL **graph memory** stack: it coordinates [`ainl-memory`](https://crates.io/crates/ainl-memory), [`ainl-persona`](https://crates.io/crates/ainl-persona)’s [`EvolutionEngine`](https://docs.rs/ainl-persona/latest/ainl_persona/struct.EvolutionEngine.html) (shared with [`ainl-graph-extractor`](https://crates.io/crates/ainl-graph-extractor)’s [`GraphExtractorTask`](https://docs.rs/ainl-graph-extractor/latest/ainl_graph_extractor/struct.GraphExtractorTask.html)), and optional **post-turn extraction**, with a [`TurnHooks`] seam for hosts (e.g. OpenFang).

It is **not** the Python `RuntimeEngine`, **not** the MCP server, **not** the AINL CLI, and **not** an LLM or IR parser.

## What v0.2 provides

- **[`AinlRuntime`]** — owns a [`ainl_memory::GraphMemory`] over a [`SqliteGraphStore`], a stateful [`GraphExtractorTask`], and [`RuntimeConfig`].
- **Persona evolution (direct)** — [`AinlRuntime::evolution_engine`] / [`AinlRuntime::evolution_engine_mut`], [`AinlRuntime::apply_evolution_signals`], [`AinlRuntime::evolution_correction_tick`], [`AinlRuntime::persist_evolution_snapshot`], [`AinlRuntime::evolve_persona_from_graph_signals`] (`EvolutionEngine` lives in **ainl-persona**; the extractor is an additional signal source, not a hard gate).
- **Boot** — [`AinlRuntime::load_artifact`] → [`AinlGraphArtifact`] (`export_graph` + `validate_graph`; fails on dangling edges).
- **Turn pipeline** — [`AinlRuntime::run_turn`]: validate subgraph, compile persona lines from persona nodes, [`compile_memory_context`], capped BFS walk (`next` / `follows` / `DERIVED_FROM`), record an episodic node (user message + tools), run extractor every `extraction_interval` turns.
- **Legacy API** — [`RuntimeContext`] + `record_*` + [`RuntimeContext::run_graph_extraction_pass`] unchanged for light callers.

It still does **not** execute arbitrary AINL IR, call adapters, or route emit edges; hosts wire LLM/tools on top of [`TurnOutput`] / [`MemoryContext`].

## Quick start (`AinlRuntime`)

```toml
[dependencies]
ainl-runtime = "0.2.0-alpha"
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
let _artifact = rt.load_artifact()?;
let out = rt.run_turn(TurnInput {
    user_message: "Hello".into(),
    tools_invoked: vec!["file_read".into()],
    ..Default::default()
})?;
```

## `RuntimeConfig`

- **`agent_id`**: `String` (empty disables graph extraction on [`RuntimeContext`]; required for [`AinlRuntime`] turns).
- **`max_steps`**: cap for the exploratory BFS in `run_turn` (default `1000`).
- **`extraction_interval`**: run `GraphExtractorTask::run_pass` every N turns (`0` = never).

## Persona evolution and ArmaraOS (OpenFang)

**Target convergence:** `AinlRuntime`’s evolution engine (`EvolutionEngine` + scheduled `GraphExtractorTask::run_pass`) is the intended long-term convergence point for graph-driven persona persistence in the Rust stack.

**Today:** Until ArmaraOS migrates to **ainl-runtime** as its primary execution engine, **openfang-runtime**’s `GraphMemoryWriter::run_persona_evolution_pass` is the **active** evolution write path for dashboard agents (`~/.armaraos/agents/<id>/ainl_memory.db`). Do not call `AinlRuntime::persist_evolution_snapshot` or `AinlRuntime::evolve_persona_from_graph_signals` on that same database concurrently with that pass. If you embed `AinlRuntime` next to openfang while openfang still owns evolution, chain `AinlRuntime::with_evolution_writes_enabled(false)` so those two methods return an error instead of writing.

## License

MIT OR Apache-2.0
