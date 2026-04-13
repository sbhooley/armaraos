# AINL Memory - Graph-Based Agent Memory Substrate

**Graph-as-memory for AI agents. Execution IS the memory.**

AINL Memory is a Rust library that implements agent memory as an execution graph. Every agent turn, tool call, and delegation becomes a typed graph node. No separate retrieval layer—the graph itself is the memory.

## Why AINL Memory?

Most agent frameworks treat memory as separate from execution:
- Execute → Store text → Retrieve → Execute again
- Separate episodic, semantic, and procedural stores
- Memory retrieval adds latency and complexity

AINL Memory unifies execution and memory:
- **Episodic memory**: What happened during agent turns (tool calls, delegations)
- **Semantic memory**: Facts learned with confidence scores
- **Procedural memory**: Reusable compiled workflow patterns
- **Persona memory**: Agent traits learned over time

All stored as typed graph nodes with edges for traversal.

## Quick Start

```toml
[dependencies]
ainl-memory = "0.1.3-alpha"
```

```rust
use ainl_memory::{AinlMemoryNode, SqliteGraphStore, GraphStore};

// Open a graph store
let store = SqliteGraphStore::open("memory.db").unwrap();

// Record an agent delegation
let node = AinlMemoryNode::new_delegation_episode(
    "agent-A".to_string(),
    "agent-B".to_string(),
    "trace-123".to_string(),
    1, // depth
);

store.write_node(&node).unwrap();

// Query recent episodes
let episodes = store.query_recent_episodes("agent-A", 10).unwrap();
```

## Features

- **Typed graph nodes**: Episode, Semantic, Procedural, Persona
- **SQLite backend**: Persistent graph storage with indexes
- **Graph traversal**: Walk edges between nodes
- **Swappable backends**: Implement `GraphStore` trait for custom storage
- **Zero unsafe code**: Pure Rust implementation
- **Offline-first**: No network required, local SQLite storage

### High-level `GraphMemory` API (used by ArmaraOS `openfang-runtime`)

The **`GraphMemory`** type (see **`src/lib.rs`**) wraps **`SqliteGraphStore`** with helpers including **`write_episode`**, **`write_fact`**, **`store_pattern`**, **`write_persona`**, **`recall_recent`**, and **`recall_by_type`** (filter by **`AinlNodeKind`**, e.g. **`Persona`**, within a time window). **`openfang-runtime`** exposes this through **`GraphMemoryWriter`** at **`~/.armaraos/agents/<agent_id>/ainl_memory.db`** for delegation episodes, facts, and **persona** traits that feed the chat **system prompt** hook. Scheduled **`ainl run`** uses a separate Python JSON bridge + **`.ainlbundle`** file; see ArmaraOS **`docs/scheduled-ainl.md`**, **`docs/graph-memory.md`** (how **`openfang-runtime`** uses this crate), and **ainativelang** **`docs/adapters/AINL_GRAPH_MEMORY.md`**.

## Node Types

### Episode
Records what happened during an agent turn:
- Tool calls executed
- Delegation to other agents
- Trace ID for correlation
- Delegation depth

### Semantic
Facts learned with confidence:
- Fact text
- Confidence score (0.0-1.0)
- Source turn ID

### Procedural
Reusable workflow patterns:
- Pattern name
- Compiled graph (binary format)

### Persona
Agent traits learned over time:
- Trait name
- Strength (0.0-1.0)
- Source turn IDs

## Architecture

AINL Memory is designed as infrastructure that any agent framework can adopt:
- No dependencies on specific agent runtimes
- Simple trait-based API
- Bring your own storage backend

## ArmaraOS / `openfang-runtime` integration

**ArmaraOS** opens **`GraphMemory`** at **`~/.armaraos/agents/<agent_id>/ainl_memory.db`** via **`openfang_runtime::graph_memory_writer::GraphMemoryWriter`** (async-friendly **`Arc<Mutex<GraphMemory>>`**). The agent loop records episodes and facts; **`GraphMemory::recall_by_type`**, **`write_persona`**, and **`AinlNodeKind`** support **persona** recall for the chat **system prompt** hook. Scheduled **`ainl run`** uses a different persistence path (**`AINLBundle`** / **`ainl_graph_memory`** JSON); see **armaraos** **`docs/graph-memory.md`** and **`docs/scheduled-ainl.md`**.

## Status

**Alpha (`0.1.3-alpha` on crates.io when published; supersedes `0.1.2-alpha`).**

API is subject to change. Production-ready for experimentation.

See [`CHANGELOG.md`](CHANGELOG.md) for schema and API deltas vs the previous crates.io release.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contributing

Contributions are welcome! This is early-stage infrastructure—your feedback shapes the API.

## See Also

- [ainl-runtime](https://crates.io/crates/ainl-runtime) - AINL execution runtime (coming soon)
- [AINL Specification](https://github.com/sbhooley/ainativelang) - AI Native Language design
