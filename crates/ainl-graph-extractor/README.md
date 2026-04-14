# ainl-graph-extractor

> **Experimental alpha crate** from the AINL / ArmaraOS workspace. API may change rapidly before 1.0.

**Graph-aware extraction pipeline for AINL** — bumps semantic `recurrence_count` from retrieval-style deltas and runs persona evolution via **ainl-persona** on a shared **ainl-memory** SQLite store.

## What it does

- [`run_extraction_pass`](https://docs.rs/ainl-graph-extractor/latest/ainl_graph_extractor/fn.run_extraction_pass.html) — convenience one-shot: builds a fresh [`GraphExtractorTask`](https://docs.rs/ainl-graph-extractor/latest/ainl_graph_extractor/struct.GraphExtractorTask.html) per call (streak-based heuristics do not carry across invocations). Returns [`ExtractionReport`](https://docs.rs/ainl-graph-extractor/latest/ainl_graph_extractor/struct.ExtractionReport.html) directly (not `Result`).
- Stateful agents should construct [`GraphExtractorTask`](https://docs.rs/ainl-graph-extractor/latest/ainl_graph_extractor/struct.GraphExtractorTask.html) once and call [`run_pass`](https://docs.rs/ainl-graph-extractor/latest/ainl_graph_extractor/struct.GraphExtractorTask.html#method.run_pass) each tick (same as **ainl-runtime** and **openfang-runtime**).
- Re-exports [`EVOLUTION_TRAIT_NAME`](https://docs.rs/ainl-graph-extractor/latest/ainl_graph_extractor/constant.EVOLUTION_TRAIT_NAME.html) from **ainl-persona** so callers do not duplicate the evolution trait string.

### `ExtractionReport` (per-phase errors)

[`GraphExtractorTask::run_pass`](https://docs.rs/ainl-graph-extractor/latest/ainl_graph_extractor/struct.GraphExtractorTask.html#method.run_pass) always returns a report. Failures are **orthogonal slots** (the pass keeps going so you can observe more than one):

| Field | Meaning |
|--------|--------|
| **`merged_signals`** | Merged graph + heuristic [`RawSignal`](https://docs.rs/ainl-persona/latest/ainl_persona/struct.RawSignal.html) batch ingested this pass (empty on a cold graph). |
| **`facts_written`** | Semantic recurrence rows touched (`None` if recurrence bookkeeping did not complete cleanly). |
| **`extract_error`** | Graph extract, heuristic collect, or persona-row probe failures (signal merge / “read” side before persona write). |
| **`pattern_error`** | Semantic recurrence update failure or episode tag flush (**pattern persistence**). |
| **`persona_error`** | Evolution persona row write failure (or test injection). |
| **`has_errors()`** | `true` if any of the three `*_error` options are set. |

Hidden **`test_inject_*`** fields on [`GraphExtractorTask`](https://docs.rs/ainl-graph-extractor/latest/ainl_graph_extractor/struct.GraphExtractorTask.html) exist for deterministic tests; do not use in production.

## Where it fits

Sits between **ainl-memory** (persistence) and agent loops that periodically consolidate signals (ArmaraOS `openfang-runtime` uses related wiring in-tree). Deterministic, offline, no external services.

## Usage sketch

```rust
use ainl_graph_extractor::run_extraction_pass;
use ainl_memory::{GraphStore, SqliteGraphStore};

fn main() -> Result<(), String> {
    let store = SqliteGraphStore::open("agent_memory.db")?;
    let report = run_extraction_pass(&store, "my-agent");
    if report.has_errors() {
        eprintln!("extractor: {report:?}");
    }
    Ok(())
}
```

## Status

**Alpha — current crate version `0.1.5` on crates.io** (workspace may match). Registry releases depend on **`ainl-memory` 0.1.8-alpha** and **`ainl-persona` 0.1.4** so they resolve together with **`ainl-runtime` 0.3.5-alpha**. API surface is intentionally small; internal modules may move as extraction rules evolve.

**Tests:** `cargo test -p ainl-graph-extractor` (includes **`tests/test_extraction_report.rs`** for per-phase fields and **`tests/test_graph_extractor.rs`** for integration).

## License

Licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at your option.
