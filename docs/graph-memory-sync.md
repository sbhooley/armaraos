# Graph memory sync (Python ↔ Rust)

This topic lives in **[graph-memory.md](graph-memory.md)**:

- **Python inbox (write-back)** — `ainl_graph_memory_inbox.json`, **`ARMARAOS_AGENT_ID`**, **`AinlMemorySyncWriter`** (implemented in **ainativelang** `armaraos/bridge/ainl_memory_sync.py`).
- **Rust drain** — `GraphMemoryWriter::drain_python_graph_memory_inbox` at agent-loop start.
- **Optional extraction / tagging** — env **`AINL_EXTRACTOR_ENABLED`**, **`AINL_TAGGER_ENABLED`**; see **`crates/openfang-runtime/README.md`**.

Cross-repo Python contract: **ainativelang** [`docs/adapters/AINL_GRAPH_MEMORY.md`](https://github.com/sbhooley/ainativelang/blob/main/docs/adapters/AINL_GRAPH_MEMORY.md) (*Python inbox*).
