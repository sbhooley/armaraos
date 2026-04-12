# AINL Runtime - Graph-Based Agent Programming Runtime

**Execution runtime for AI Native Language (AINL) with integrated graph-memory.**

AINL Runtime provides the execution context and memory integration for graph-based agent programming. It connects agent execution to the AINL Memory substrate, automatically recording delegations, tool calls, and learned facts as graph nodes.

## Why AINL Runtime?

Traditional agent runtimes separate execution from memory storage:
- Execute tools → Store text → Retrieve when needed
- Memory is an afterthought, not part of the execution model

AINL Runtime integrates memory into the execution loop:
- Every delegation is a graph node
- Every tool call is recorded automatically
- Memory IS the execution trace

## Quick Start

```toml
[dependencies]
ainl-runtime = "0.1.0-alpha"
ainl-memory = "0.1.0-alpha"
```

```rust
use ainl_runtime::{RuntimeConfig, RuntimeContext};
use ainl_memory::SqliteGraphStore;

// Create a runtime with graph-backed memory
let config = RuntimeConfig::default();
let store = SqliteGraphStore::open("memory.db").unwrap();
let runtime = RuntimeContext::new(config, Some(store));

// Record a delegation
runtime.record_delegation(
    "agent-A".to_string(),
    "agent-B".to_string(),
    "trace-123".to_string(),
    1, // depth
).unwrap();

// Record a tool execution
runtime.record_tool_execution(
    "agent-A".to_string(),
    "file_read".to_string(),
).unwrap();
```

## Features

- **Graph-native memory**: Execution traces stored as graph nodes
- **Delegation tracking**: Automatic recording of agent delegation chains
- **Tool execution history**: Every tool call becomes a graph node
- **Configurable depth limits**: Prevent infinite delegation loops
- **Optional memory backend**: Can run with or without persistence

## Integration with AINL Memory

AINL Runtime depends on [ainl-memory](https://crates.io/crates/ainl-memory) for graph-based storage. The runtime automatically creates typed nodes for:

- **Episode nodes**: Agent turns with tool calls and delegations
- **Delegation chains**: Connected via graph edges
- **Tool execution sequences**: Ordered by timestamp

## Configuration

```rust
use ainl_runtime::RuntimeConfig;

let config = RuntimeConfig {
    max_delegation_depth: 10,
    enable_graph_memory: true,
};
```

- `max_delegation_depth`: Maximum depth for delegation chains (default: 10)
- `enable_graph_memory`: Enable persistent graph storage (default: true)

## Status

**Alpha (0.1.0-alpha)**

API is subject to change. Production-ready for experimentation.

Currently, this crate provides the core integration layer between agent runtimes and the AINL Memory graph substrate. Future versions will include:
- Full AINL language parser
- Graph-based workflow compilation
- Built-in agent orchestration primitives

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contributing

Contributions are welcome! This is early-stage infrastructure—your feedback shapes the API.

## See Also

- [ainl-memory](https://crates.io/crates/ainl-memory) - AINL graph-memory substrate
- [AINL Specification](https://github.com/sbhooley/ainativelang) - AI Native Language design
