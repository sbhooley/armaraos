# Graph memory explainability (V1)

This document records the **V1** cross-crate design for operator-facing graph memory clarity: live SSE events, `/api/graph-memory` payloads, and the dashboard Graph Memory page.

## Crate boundaries

| Layer | Responsibility |
|-------|----------------|
| **`ainl-memory`** | Node payloads (episode, semantic, procedural, persona, runtime_state) in SQLite; unchanged in V1 except consumers reading richer **API** projections. |
| **`openfang-types`** | `SystemEvent::GraphMemoryWrite` + optional **`GraphMemoryWriteProvenance`** (additive, backward-compatible JSON). |
| **`openfang-runtime`** | `GraphMemoryWriter` fires hooks with `(agent_id, kind, provenance)` after `record_*`, `run_persona_evolution_pass`, `PersonaEvolutionHook`, inbox import, and `emit_write_observed` (e.g. `ainl-runtime` bridge). |
| **`openfang-kernel`** | Publishes SSE events for the dashboard event stream. |
| **`openfang-api`** | `GET /api/graph-memory` adds per-node **`explain`** `{ what_happened, why_happened, evidence, relations, node_kind }` alongside existing **`meta`**. |

## Runtime event contract (`GraphMemoryWrite`)

- **`kind`**: `episode`, `delegation`, `fact`, `procedural`, `persona`, or observed aliases (`episode` for `ainl-runtime` observed writes).
- **`provenance`** (optional): `node_ids`, `node_kind`, `reason`, `summary`, `trace_id`.
- Older daemons / JSON without `provenance` still deserialize (`None`).

## HTTP graph shape

Each node includes:

- **`meta`**: raw fields from the store (existing behavior, extended for persona with evolution/cycle hints).
- **`explain`**: human-oriented strings + structured `evidence` + `relations` (typed edges with peer labels).

## crates.io publish order (when releasing types independently)

**OpenFang-only changes:**

1. `openfang-types`
2. `openfang-runtime`
3. `openfang-kernel`
4. `openfang-api`
5. `openfang-cli` (if version-bumped with the stack)

**If `ainl-*` crates also change** (V2 model-level rationale — not required for V1):

1. `ainl-memory`
2. `ainl-persona`
3. `ainl-graph-extractor`
4. then the OpenFang order above.

## References

- Implementation: `crates/openfang-runtime/src/graph_memory_writer.rs`, `crates/openfang-api/src/graph_memory.rs`, `crates/openfang-api/static/js/pages/graph-memory.js`.
