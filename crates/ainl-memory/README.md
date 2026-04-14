# ainl-memory

> ⚠️ Alpha — API subject to change

**The unified graph substrate for AINL agents.**  
Execution IS memory. Memory IS the graph.

## What it is

ainl-memory implements the AINL unified graph: a single typed, executable, auditable artifact that simultaneously encodes an agent's memory, persona, tools, and execution history.

Unlike systems that treat memory as a separate retrieval layer (RAG, vector stores, external graph DBs), ainl-memory makes the execution graph itself the memory substrate — no retrieval boundary, no translation step, no sync problem.

## Documentation map

| Topic | Where |
|-------|--------|
| Schema, FK migration, `import_graph` flag | **[CHANGELOG.md](CHANGELOG.md)** (`0.1.4-alpha`+) |
| SQL-level integrity layers (FK, repair import, `validate_graph` scope) | **`src/store.rs`** module docs |
| `GraphQuery` + free helpers (`recall_*`, `walk_from`, …) | **`src/query.rs`** module docs |
| Snapshot / validation types | **`src/snapshot.rs`** |

## The five memory families

| Type | Node / category | What it stores |
|------|-----------------|----------------|
| Episodic | EpisodicNode | Agent turns, tool calls, outcomes; optional **`tags`** (e.g. ArmaraOS **`ainl-semantic-tagger`** strings on tool sequences) |
| Semantic | SemanticNode | Facts, beliefs, topic clusters; optional **`tags`** (correlation + tagger hints) |
| Procedural | ProceduralNode | Compiled patterns, GraphPatch labels |
| Persona | PersonaNode | Identity, evolved axis scores, dominant traits |
| Session (runtime) | `RuntimeStateNode` (`node_type = runtime_state`) | Per-agent **persisted counters**: `turn_count`, `last_extraction_at_turn`, optional **`persona_snapshot_json`** (JSON-encoded compiled persona string), `updated_at` (unix seconds). Upserted by **ainl-runtime** so daemon restarts do not reset extraction cadence or force a cold persona compile on the first post-restart turn. |

`AinlNodeKind::RuntimeState` matches the `runtime_state` SQL / JSON tag; `MemoryCategory::RuntimeState` is the same slice for exports and analytics.

## Referential integrity & edges

SQLite enforces **basic referential integrity** on the graph edge table:

- Table **`ainl_graph_edges`** columns: `from_id`, `to_id`, `label`, `weight`, `metadata` (primary key `(from_id, to_id, label)`).
- **`FOREIGN KEY (from_id)`** and **`FOREIGN KEY (to_id)`** reference **`ainl_graph_nodes(id)`** with **`ON DELETE CASCADE`**.
- **`PRAGMA foreign_keys = ON`** is applied when opening a [`SqliteGraphStore`](https://docs.rs/ainl-memory/latest/ainl_memory/struct.SqliteGraphStore.html) (see `open` / `from_connection`).

**Legacy databases** (edges table created before FK metadata): on first open, a **one-time migration** rebuilds `ainl_graph_edges`. Only rows whose **both** endpoints exist in `ainl_graph_nodes` are copied; historical dangling rows are **dropped** (they cannot exist under FK rules). Details: [CHANGELOG.md](CHANGELOG.md) § 0.1.4-alpha.

**Write paths**

| API | Role |
|-----|------|
| [`GraphStore::write_node`](https://docs.rs/ainl-memory/latest/ainl_memory/trait.GraphStore.html) | Upserts node JSON, then persists embedded `AinlEdge` rows (node must exist before edges; order satisfies FK). |
| [`SqliteGraphStore::write_node_with_edges`](https://docs.rs/ainl-memory/latest/ainl_memory/struct.SqliteGraphStore.html#method.write_node_with_edges) | Single transaction; **fails** if any embedded edge target is missing (application check + FK). |
| [`SqliteGraphStore::insert_graph_edge`](https://docs.rs/ainl-memory/latest/ainl_memory/struct.SqliteGraphStore.html#method.insert_graph_edge) | Inserts one row; SQLite rejects invalid endpoints when FKs are on. |
| [`SqliteGraphStore::insert_graph_edge_checked`](https://docs.rs/ainl-memory/latest/ainl_memory/struct.SqliteGraphStore.html#method.insert_graph_edge_checked) | Pre-checks both node rows exist, then inserts (clear errors without relying on SQLite message text). |

**Repair / forensic import**

- [`import_graph(snapshot, allow_dangling_edges)`](https://docs.rs/ainl-memory/latest/ainl_memory/struct.SqliteGraphStore.html#method.import_graph): use **`false`** everywhere in production (strict). Use **`true`** only to load snapshots that **violate** referential integrity: FK checks are disabled **only for that import**, then restored. Always follow with **`validate_graph`** and fix data before resuming normal writes on the same connection.

**Higher-level validation** (orthogonal to FK row existence)

- [`validate_graph(agent_id)`](https://docs.rs/ainl-memory/latest/ainl_memory/struct.SqliteGraphStore.html#method.validate_graph) reports agent-scoped edges, **dangling** endpoint pairs, optional **`DanglingEdgeDetail`** (includes **edge label**), **`cross_agent_boundary_edges`**, orphans, and `is_valid`. Use this for semantics, exports alignment, and post-repair audits.

**Rust snapshot type vs SQL**

- [`SnapshotEdge`](https://docs.rs/ainl-memory/latest/ainl_memory/struct.SnapshotEdge.html) uses `source_id` / `target_id` / `edge_type`; the database uses `from_id` / `to_id` / `label`. Import/export maps between them.

## Core API

### Store

```rust
use ainl_memory::{AinlMemoryNode, GraphStore, SqliteGraphStore};
use std::path::Path;

let store = SqliteGraphStore::open(Path::new("memory.db"))?;
let mut node = AinlMemoryNode::new_episode(
    uuid::Uuid::new_v4(),
    chrono::Utc::now().timestamp(),
    vec!["file_read".into()],
    None,
    None,
);
node.agent_id = "my-agent".into();
store.write_node(&node)?;
```

### GraphQuery (`SqliteGraphStore::query`)

Builder scoped to one agent: `COALESCE(json_extract(payload, '$.agent_id'), '') = <agent_id>`. Edge traversal uses SQL columns `from_id`, `to_id`, `label`.

| Method | Purpose |
|--------|---------|
| `episodes` / `semantic_nodes` / `procedural_nodes` / `persona_nodes` | All nodes of that kind for the agent |
| `recent_episodes(limit)` | Episodes ordered by `timestamp` DESC |
| `since(ts, node_type)` | Nodes with `timestamp >= ts`, type normalized to SQL `node_type`, ascending |
| `subgraph_edges` | Edges with **both** endpoints in this agent’s node id set (aligned with `export_graph`) |
| `neighbors(node_id, edge_type)` | Targets of outgoing edges (`label = edge_type`) |
| `lineage(node_id)` | BFS on `DERIVED_FROM` / `CAUSED_PATCH`, max depth **20**, excludes start node from results |
| `by_tag(tag)` | `json_each` on episode `persona_signals_emitted` or semantic `tags` |
| `by_topic_cluster(cluster)` | Semantic `topic_cluster` LIKE |
| `pattern_by_name(name)` | Procedural `pattern_name` or `label` |
| `active_patches` | Procedural rows where `retired` is null / false |
| `successful_episodes(limit)` | Episodes with `node_type.outcome == "success"` in JSON |
| `episodes_with_tool(tool, limit)` | Episodes where `tools_invoked` or `tool_calls` contains the tool |
| `evolved_persona` | Latest persona with `trait_name == "axis_evolution_snapshot"` |
| `read_runtime_state` | Delegates to store; same `agent_id` as the query |

Legacy helpers (`recall_recent`, `find_patterns`, `walk_from`, …) live in the same crate and take [`GraphStore`](https://docs.rs/ainl-memory/latest/ainl_memory/trait.GraphStore.html) plus explicit `agent_id` where needed.

```rust
use ainl_memory::SqliteGraphStore;

let store = SqliteGraphStore::open(std::path::Path::new("memory.db"))?;
let recent = store.query("my-agent").recent_episodes(10)?;
let lineage = store.query("my-agent").lineage(some_node_id)?;
let internal = store.query("my-agent").subgraph_edges()?;
```

### Export / import snapshots

```rust
use ainl_memory::{SqliteGraphStore, SNAPSHOT_SCHEMA_VERSION};

let store = SqliteGraphStore::open(std::path::Path::new("memory.db"))?;
let snapshot = store.export_graph("my-agent")?;
assert_eq!(snapshot.schema_version, SNAPSHOT_SCHEMA_VERSION);

let mut fresh = SqliteGraphStore::open(std::path::Path::new("copy.db"))?;
fresh.import_graph(&snapshot, false)?; // strict: FK on
```

### Graph validation

```rust
use ainl_memory::SqliteGraphStore;

let store = SqliteGraphStore::open(std::path::Path::new("memory.db"))?;
let report = store.validate_graph("my-agent")?;
assert!(report.is_valid);
// `dangling_edge_details`: source_id, target_id, edge_type (label)
// `cross_agent_boundary_edges`: touches agent on one side only (informational)
```

### Session state (`read_runtime_state` / `write_runtime_state`)

Stable **one row per `agent_id`** (deterministic UUIDv5 over the agent id) — use the helpers instead of hand-rolling nodes:

```rust
use ainl_memory::{GraphMemory, RuntimeStateNode, SqliteGraphStore};
use std::path::Path;

let store = SqliteGraphStore::open(Path::new("memory.db"))?;
let memory = GraphMemory::from_sqlite_store(store);

let state = RuntimeStateNode {
    agent_id: "my-agent".into(),
    turn_count: 12,
    last_extraction_at_turn: 10,
    persona_snapshot_json: serde_json::to_string("compiled persona lines…").ok(),
    updated_at: chrono::Utc::now().timestamp(),
};
memory.write_runtime_state(&state)?;
let _loaded = memory.read_runtime_state("my-agent")?;

let q = memory.sqlite_store().query("my-agent");
let _same = q.read_runtime_state()?;
```

Legacy rows may still carry JSON keys `last_extraction_turn`, `last_persona_prompt`, or RFC3339 `updated_at` strings; **`RuntimeStateNode`** deserializes them via serde aliases / a tolerant timestamp parser.

### `GraphMemory` forwards

[`GraphMemory`](https://docs.rs/ainl-memory) exposes `validate_graph`, `export_graph`, `import_graph`, `agent_subgraph_edges`, `write_node_with_edges`, `insert_graph_edge_checked`, **`read_runtime_state`**, and **`write_runtime_state`** so hosts like **ainl-runtime** can checkpoint or boot-gate without reaching past the high-level API.

## Integration tests (pointers)

| File | What it covers |
|------|------------------|
| `tests/test_query.rs` | `GraphQuery` filters, lineage, neighbors, outcomes |
| `tests/test_snapshot.rs` | Export/import roundtrip, idempotency, `agent_subgraph_edges` vs export |
| `tests/test_validate.rs` | `validate_graph`, strict vs `import_graph(..., true)`, `insert_graph_edge_checked` |
| `tests/test_integrity.rs` | `write_node_with_edges` |
| `tests/test_edge_migration.rs` | Legacy edges table → FK migration drops invalid rows |
| `tests/graph_integration.rs` | Broader graph memory flows |

## Crate ecosystem

- **ainl-memory** — this crate (storage + query); published version is **`0.1.8-alpha`** on the workspace (aligns with **ainl-runtime** / **ainl-graph-extractor** pins — see crates.io and sibling `Cargo.toml` files).
- **ainl-runtime** — agent turn execution, depends on ainl-memory (+ persona, extractor, semantic-tagger)
- **ainl-persona** — persona evolution engine, depends on ainl-memory
- **ainl-graph-extractor** — periodic signal extraction, depends on ainl-memory + ainl-persona
- **ainl-semantic-tagger** — deterministic text tagging, no ainl-memory dependency

## Why this is different

Traditional stacks bolt a vector index or key-value “memory” onto an LLM and hope embeddings stay aligned with what actually ran. AINL instead treats every tool call, fact, patch, and persona shift as first-class graph data you can traverse, validate, and export as one artifact — closer to a provenance-rich program trace than a fuzzy recall cache.

## License

MIT OR Apache-2.0
