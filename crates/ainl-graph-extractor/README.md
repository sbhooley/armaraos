# ainl-graph-extractor

> **Experimental alpha crate** from the AINL / ArmaraOS workspace. API may change rapidly before 1.0.

**Graph-aware extraction pipeline for AINL** — bumps semantic `recurrence_count` from retrieval-style deltas and runs persona evolution via **ainl-persona** on a shared **ainl-memory** SQLite store.

## What it does

- [`run_extraction_pass`](https://docs.rs/ainl-graph-extractor/latest/ainl_graph_extractor/fn.run_extraction_pass.html) is the main entry: one pass over the graph for an `agent_id`.
- Re-exports [`EVOLUTION_TRAIT_NAME`](https://docs.rs/ainl-graph-extractor/latest/ainl_graph_extractor/constant.EVOLUTION_TRAIT_NAME.html) from **ainl-persona** so callers do not duplicate the evolution trait string.

## Where it fits

Sits between **ainl-memory** (persistence) and agent loops that periodically consolidate signals (ArmaraOS `openfang-runtime` uses related wiring in-tree). Deterministic, offline, no external services.

## Usage sketch

```rust
use ainl_graph_extractor::run_extraction_pass;
use ainl_memory::SqliteGraphStore;

fn main() -> Result<(), String> {
    let store = SqliteGraphStore::open("agent_memory.db")?;
    let _report = run_extraction_pass(&store, "my-agent")?;
    Ok(())
}
```

## Status

**Alpha (`0.1.0-alpha`).** API surface is intentionally small; internal modules may move as extraction rules evolve.

## License

Licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at your option.
