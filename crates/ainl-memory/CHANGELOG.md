# Changelog

All notable changes to **ainl-memory** are documented here. This crate follows semantic intent for alphas: minor bumps signal schema or API additions consumers should pin.

## 0.1.8-alpha

### Documentation

- **README**: documentation map; **referential integrity** (FK + migration + `import_graph` repair flag + `validate_graph`); full **GraphQuery** method table; SQL vs `SnapshotEdge` naming; integration test pointers (`test_edge_migration`, `test_validate`, …).
- **`src/store.rs`**: module-level docs already describe SQLite FK layers vs application checks vs repair import (see source).

## 0.1.4-alpha

### Schema

- **`ainl_graph_edges`**: `FOREIGN KEY (from_id)` and `FOREIGN KEY (to_id)` referencing `ainl_graph_nodes(id)` with **`ON DELETE CASCADE`**, plus existing `weight` / `metadata` columns.
- **Migration (existing SQLite files)**: On open, if `pragma_foreign_key_list('ainl_graph_edges')` is empty, the table is renamed to `ainl_graph_edges__old`, recreated with FKs, repopulated with **`INSERT … SELECT` only for rows where both endpoints exist** in `ainl_graph_nodes`, then `ainl_graph_edges__old` is dropped. **Dangling historical edges are discarded** (they cannot be stored under FK rules).

### Added

- **`GraphQuery`** / **`SqliteGraphStore::query`**, **`export_graph`**, **`validate_graph`**, **`write_node_with_edges`**, snapshot types (`AgentGraphSnapshot`, `SnapshotEdge`, …) — graph builder, export/import, and validation (see README).
- **`GraphQuery::subgraph_edges`** / **`SqliteGraphStore::agent_subgraph_edges`**: export-compatible internal edge list for one agent (both endpoints in that agent’s node id set).
- **`GraphValidationReport`**: **`dangling_edge_details`** (source/target/**label**) and **`cross_agent_boundary_edges`** (edges that touch the agent on exactly one side while both node rows exist).
- **`SqliteGraphStore::insert_graph_edge_checked`**: fail fast with a clear error if either endpoint row is missing (strict runtime wiring).
- **`GraphMemory`**: forwards **`validate_graph`**, **`export_graph`**, **`import_graph`**, **`agent_subgraph_edges`**, **`write_node_with_edges`**, **`insert_graph_edge_checked`** for host crates (e.g. **ainl-runtime**).
- **`DanglingEdgeDetail`** snapshot helper type.

### Changed

- **`import_graph`**: signature is now `import_graph(snapshot, allow_dangling_edges: bool)`. Pass **`false`** for normal operation (foreign keys remain enabled). Pass **`true`** only for controlled repair/forensic imports (FK checks disabled for the duration of that import); follow with **`validate_graph`** and fix data before resuming normal writes.

### Notes for downstream

- **Publish order unchanged:** **ainl-memory** → **ainl-persona** → **ainl-graph-extractor** → **ainl-runtime** (see `scripts/publish-prep-ainl-crates.sh`).
- **Pre-release bumps:** if you raise **`ainl-memory`**’s pre-release version, older **`^0.1.x-alpha`** requirements on **already-published** persona/extractor crates may block **`cargo publish`** for **ainl-runtime** until you ship **new** persona/extractor versions with aligned pins — see **`docs/ainl-runtime-graph-patch.md`** (*Pre-release versions and `cargo publish`*).
- Call sites must pass the new `allow_dangling_edges` argument (typically `false`).

## 0.1.3-alpha

### Added

- **`EpisodicNode`**: optional `user_message` and `assistant_response` (`Option<String>`) for offline extractors and richer persona / graph tooling; omitted from JSON when unset (`skip_serializing_if`).
- **`new_episode`**: initializes the new optional fields to `None`.

### Notes for downstream

- Crates.io currently lists **0.1.2-alpha** as latest; any crate that reads episode payloads or constructs `EpisodicNode` literals should bump to **0.1.3-alpha** before publishing dependents that rely on these fields.
- Publish order: **ainl-memory** → **ainl-persona** → **ainl-graph-extractor** → **ainl-runtime** (see `scripts/publish-prep-ainl-crates.sh`).

## 0.1.2-alpha

Prior published baseline on crates.io (semantic recurrence / graph store evolution).
