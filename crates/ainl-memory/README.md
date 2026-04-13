# ainl-memory

> ⚠️ Alpha — API subject to change

**The unified graph substrate for AINL agents.**  
Execution IS memory. Memory IS the graph.

## What it is

ainl-memory implements the AINL unified graph: a single typed, executable, auditable artifact that simultaneously encodes an agent's memory, persona, tools, and execution history.

Unlike systems that treat memory as a separate retrieval layer (RAG, vector stores, external graph DBs), ainl-memory makes the execution graph itself the memory substrate — no retrieval boundary, no translation step, no sync problem.

## The four memory types

| Type | Node | What it stores |
|------|------|----------------|
| Episodic | EpisodicNode | Agent turns, tool calls, outcomes |
| Semantic | SemanticNode | Facts, beliefs, topic clusters |
| Procedural | ProceduralNode | Compiled patterns, GraphPatch labels |
| Persona | PersonaNode | Identity, evolved axis scores, dominant traits |

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

### `GraphMemory` forwards (runtime alignment)

[`GraphMemory`](https://docs.rs/ainl-memory) also exposes `validate_graph`, `export_graph`, `import_graph`, `agent_subgraph_edges`, `write_node_with_edges`, and `insert_graph_edge_checked` so hosts like **ainl-runtime** can checkpoint or boot-gate without reaching past the high-level API.

## Crate ecosystem

- **ainl-memory** — this crate (storage + query)
- **ainl-runtime** — agent turn execution, depends on ainl-memory
- **ainl-persona** — persona evolution engine, depends on ainl-memory
- **ainl-graph-extractor** — periodic signal extraction, depends on ainl-memory + ainl-persona
- **ainl-semantic-tagger** — deterministic text tagging, no ainl-memory dependency

## Why this is different

Traditional stacks bolt a vector index or key-value “memory” onto an LLM and hope embeddings stay aligned with what actually ran. AINL instead treats every tool call, fact, patch, and persona shift as first-class graph data you can traverse, validate, and export as one artifact — closer to a provenance-rich program trace than a fuzzy recall cache.

## License

MIT OR Apache-2.0
