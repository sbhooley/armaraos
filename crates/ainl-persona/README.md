# ainl-persona

> **Experimental alpha crate** from the AINL / ArmaraOS workspace. API may change rapidly before 1.0.

**Persona graph and evolution engine for AINL agents** — additive base/delta/injection persona layering, soft axes, and metadata-only signals read from [`ainl-memory`](https://crates.io/crates/ainl-memory) graph stores.

## What it does

- Reads episodic, semantic, procedural, and persona nodes for an agent id and turns them into weighted [`RawSignal`](https://docs.rs/ainl-persona/latest/ainl_persona/struct.RawSignal.html) values on configurable [`PersonaAxis`](https://docs.rs/ainl-persona/latest/ainl_persona/enum.PersonaAxis.html) axes.
- [`EvolutionEngine`](https://docs.rs/ainl-persona/latest/ainl_persona/struct.EvolutionEngine.html) runs extract → ingest (EMA-style updates) → snapshot → optional write of an evolution bundle persona row (trait name [`EVOLUTION_TRAIT_NAME`](https://docs.rs/ainl-persona/latest/ainl_persona/constant.EVOLUTION_TRAIT_NAME.html)). Public methods include **`extract_signals`**, **`ingest_signals`**, **`correction_tick`**, **`snapshot`**, **`write_persona_node`**, and **`evolve`** (full graph-backed pass).

## Where it fits

Used together with **ainl-memory** (SQLite graph) and **ainl-graph-extractor** in the ArmaraOS / OpenFang stack. It does not talk to the network; it only interprets graph state.

**ainl-graph-extractor** is one producer of signals (graph-backed `extract_signals` plus pattern heuristics); **ainl-runtime** can also call [`EvolutionEngine`](https://docs.rs/ainl-persona/latest/ainl_persona/struct.EvolutionEngine.html) directly (`ingest_signals`, `correction_tick`, `evolve`, …) on the same engine instance so evolution is not extractor-gated.

## Usage sketch

```rust
use ainl_memory::SqliteGraphStore;
use ainl_persona::EvolutionEngine;

fn main() -> Result<(), String> {
    let store = SqliteGraphStore::open("agent_memory.db")?;
    let mut engine = EvolutionEngine::new("my-agent");
    let signals = engine.extract_signals(&store)?;
    engine.ingest_signals(signals);
    let snapshot = engine.snapshot();
    engine.write_persona_node(&store, &snapshot)?;
    Ok(())
}
```

## Status

**Alpha — current crate version `0.1.4` on crates.io** (workspace may match). Depends on **`ainl-memory` 0.1.8-alpha** when pulled from the registry so it can sit in one resolver graph with **`ainl-graph-extractor` 0.1.5** and **`ainl-runtime` 0.3.5-alpha**. Scope is the evolution pipeline and shared constants for downstream extractors; it is not a full persona DSL.

## License

Licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at your option.
