# Persona evolution (ArmaraOS dashboard agents)

| Concern | Doc / code |
|---------|----------------|
| SQLite paths + export file | **[graph-memory.md](graph-memory.md)** |
| Cargo features + **`AINL_TAGGER_ENABLED`** / **`AINL_EXTRACTOR_ENABLED`** | **`crates/openfang-runtime/README.md`** |
| This page | Axis snapshot hook (**`AINL_PERSONA_EVOLUTION`**) |

ArmaraOS stores per-agent graph memory in `~/.armaraos/agents/<agent_id>/ainl_memory.db`. Persona-related rows include:

- **Trait rows** — human-facing labels (e.g. `prefers_brevity`) with a scalar `strength`.
- **Axis evolution bundle** — a single row whose `trait_name` is `axis_evolution_snapshot` (constant `EVOLUTION_TRAIT_NAME` in the `ainl-persona` crate). It holds soft axis scores (`Instrumentality`, `Curiosity`, `Persistence`, `Systematicity`, `Verbosity`) used for prompt injection and long-horizon adaptation.

## Two write paths

1. **`GraphMemoryWriter::run_persona_evolution_pass`** (spawned after each turn) runs **`ainl-graph-extractor`**: semantic recurrence bumps, graph signal extraction, heuristic persona signals, then persists the evolution snapshot when appropriate. It returns an **`ExtractionReport`** (`extract_error` / `pattern_error` / `persona_error` + **`has_errors()`**); partial failures are **`warn!`**’d, not thrown.
2. **`PersonaEvolutionHook::evolve_from_turn`** (optional) runs **after** that pass. It re-reads the latest evolution snapshot, applies **explicit** signals derived from this turn’s tool list and optional `delegation_to`, then writes the snapshot again. This matters because dashboard episodes often omit `trace_event.outcome: "success"`, so `ainl-persona`’s episodic extractor skips tool-based hints unless this hook runs.

## Runtime toggle: `AINL_PERSONA_EVOLUTION`

The turn hook is **off by default** (no env → no extra writes).

Set one of:

- `AINL_PERSONA_EVOLUTION=1`
- `AINL_PERSONA_EVOLUTION=true` (also `yes`, `on`, case-insensitive)

to enable `PersonaEvolutionHook::evolve_from_turn` from the agent loop. When unset or falsey, the hook returns immediately without touching SQLite.

## How axis scores move (grow / decay)

- **Growth:** Each matching `RawSignal` nudges an axis score with a **weighted EMA** (see `ainl_persona::AxisState::update_weighted` and `EMA_ALPHA` in the `ainl-persona` crate). Repeated similar tools (e.g. `shell_exec` → instrumentality hints) push the score toward the signal’s `reward`, bounded in `[0, 1]`.
- **Decay / neutral pull:** When `run_persona_evolution_pass` sees **no merged signals** for a pass but the agent already had at least one persona row, it applies `EvolutionEngine::correction_tick` on every axis toward `0.5` and persists — a slow re-centering when the graph goes quiet.

Trait-level `strength` on non-evolution persona rows is not rewritten by this hook; only the axis snapshot row is updated via `write_persona_node`.

## Crate feature: `ainl-persona-evolution`

`openfang-runtime` gates direct `ainl-persona` usage behind the Cargo feature **`ainl-persona-evolution`** (enabled in default features). Minimal builds can disable it; **`PersonaEvolutionHook`** becomes a no-op and the **`EvolutionEngine::correction_tick`** cold-graph branch inside **`run_persona_evolution_pass`** is skipped.

## Crate feature: `ainl-extractor`

The full **`GraphMemoryWriter::run_persona_evolution_pass`** implementation (semantic recurrence, **`GraphExtractorTask`**, **`EvolutionEngine`** ingest) is compiled only when Cargo feature **`ainl-extractor`** is enabled (default **on**). Without it, the method returns a lightweight stub **`ExtractionReport`** so the daemon still links; persona **reading** from SQLite continues to work, but **no** extractor-driven evolution writes occur from that pass.

## Tests

`openfang-runtime` includes `test_persona_strength_increases_after_repeated_tool`: with `AINL_PERSONA_EVOLUTION=1`, two successive `PersonaEvolutionHook::evolve_from_turn` calls (same `shell_exec` tool list) each bump **`evolution_cycle`** on the axis snapshot row. Axis scores use a weighted EMA toward `reward * weight`, so a given axis is not guaranteed to move monotonically upward on every repeated tool; the cycle counter is the stable “learning persisted” signal.

## Related docs

- `docs/graph-memory.md` — SQLite layout, export path, scheduled `ainl run` bundles.
- `crates/openfang-runtime/README.md` — **`AINL_EXTRACTOR_ENABLED`**, **`AINL_TAGGER_ENABLED`**, **`AINL_PERSONA_EVOLUTION`**, default Cargo features.
- `crates/ainl-persona/README.md` — axis model and evolution engine API.
- `crates/ainl-runtime/README.md` — coordination note if another host writes the same DB.
