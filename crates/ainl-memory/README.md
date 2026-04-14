# ainl-memory

> ⚠️ Alpha — API subject to change

**The unified graph substrate for AINL agents.**  
Execution IS memory. Memory IS the graph.

## What it is

ainl-memory implements the AINL unified graph: a single typed, executable, auditable artifact that simultaneously encodes an agent's memory, persona, tools, and execution history.

Unlike systems that treat memory as a separate retrieval layer (RAG, vector stores, external graph DBs), ainl-memory makes the execution graph itself the memory substrate — no retrieval boundary, no translation step, no sync problem.

## The five memory families

| Type | Node / category | What it stores |
|------|-----------------|----------------|
| Episodic | EpisodicNode | Agent turns, tool calls, outcomes |
| Semantic | SemanticNode | Facts, beliefs, topic clusters |
| Procedural | ProceduralNode | Compiled patterns, GraphPatch labels |
| Persona | PersonaNode | Identity, evolved axis scores, dominant traits |
| Session (runtime) | `RuntimeStateNode` (`node_type = runtime_state`) | Per-agent **persisted counters**: `turn_count`, `last_extraction_at_turn`, optional **`persona_snapshot_json`** (JSON-encoded compiled persona string), `updated_at` (unix seconds). Upserted by **ainl-runtime** so daemon restarts do not reset extraction cadence or force a cold persona compile on the first post-restart turn. |

`AinlNodeKind::RuntimeState` matches the `runtime_state` SQL / JSON tag; `MemoryCategory::RuntimeState` is the same slice for exports and analytics.

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

### Query (new in v0.1.4)

```rust
use ainl_memory::SqliteGraphStore;

let store = SqliteGraphStore::open(std::path::Path::new("memory.db"))?;
let recent = store.query("my-agent").recent_episodes(10)?;
let lineage = store.query("my-agent").lineage(some_node_id)?;
let internal = store.query("my-agent").subgraph_edges()?; // both endpoints owned by the agent
```

### Export / Import (new in v0.1.4)

```rust
use ainl_memory::SqliteGraphStore;

let store = SqliteGraphStore::open(std::path::Path::new("memory.db"))?;
let snapshot = store.export_graph("my-agent")?;
let mut fresh = SqliteGraphStore::open(std::path::Path::new("copy.db"))?;
fresh.import_graph(&snapshot, false)?;
```

Use `import_graph(snapshot, true)` only for controlled repair loads (FK checks disabled for that import); run `validate_graph` afterward and fix data before resuming normal writes.

Edges reference `ainl_graph_nodes(id)` at the database level. Upgrades from pre-FK databases run a one-time migration (see `CHANGELOG.md`): valid edges are copied into a new table; orphaned edge rows are dropped.

### Graph Validation (new in v0.1.4)

```rust
use ainl_memory::SqliteGraphStore;

let store = SqliteGraphStore::open(std::path::Path::new("memory.db"))?;
let report = store.validate_graph("my-agent")?;
assert!(report.is_valid);
// `report.dangling_edge_details` includes edge labels; `report.cross_agent_boundary_edges`
// counts edges that touch this agent on one side only (informational).
```

### Session state (`read_runtime_state` / `write_runtime_state`, v0.1.8+)

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

// Scoped query (same connection):
let q = memory.sqlite_store().query("my-agent");
let _same = q.read_runtime_state()?;
```

Legacy rows may still carry JSON keys `last_extraction_turn`, `last_persona_prompt`, or RFC3339 `updated_at` strings; **`RuntimeStateNode`** deserializes them via serde aliases / a tolerant timestamp parser.

### `GraphMemory` forwards (runtime alignment)

[`GraphMemory`](https://docs.rs/ainl-memory) also exposes `validate_graph`, `export_graph`, `import_graph`, `agent_subgraph_edges`, `write_node_with_edges`, `insert_graph_edge_checked`, **`read_runtime_state`**, and **`write_runtime_state`** so hosts like **ainl-runtime** can checkpoint or boot-gate without reaching past the high-level API.

## Crate ecosystem

- **ainl-memory** — this crate (storage + query); published **`0.1.8-alpha`** aligns with **`ainl-runtime` 0.3.5-alpha** / **`ainl-graph-extractor` 0.1.5** on crates.io
- **ainl-runtime** — agent turn execution, depends on ainl-memory (+ persona, extractor, semantic-tagger)
- **ainl-persona** — persona evolution engine, depends on ainl-memory (**use `0.1.4+`** from crates.io with memory **0.1.8-alpha**)
- **ainl-graph-extractor** — periodic signal extraction, depends on ainl-memory + ainl-persona
- **ainl-semantic-tagger** — deterministic text tagging, no ainl-memory dependency

## Why this is different

Traditional stacks bolt a vector index or key-value “memory” onto an LLM and hope embeddings stay aligned with what actually ran. AINL instead treats every tool call, fact, patch, and persona shift as first-class graph data you can traverse, validate, and export as one artifact — closer to a provenance-rich program trace than a fuzzy recall cache.

## License

MIT OR Apache-2.0
