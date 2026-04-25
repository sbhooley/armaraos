# ainl-failure-learning

Failure recall, **FTS-backed search**, and prevention snippets for AINL hosts—bounded to `ainl-*` crates and `ainl-contracts` at the public API edge.

- **Repository:** <https://github.com/sbhooley/armaraos>
- **API reference:** <https://docs.rs/ainl-failure-learning>

Pairs with `ainl-memory` for durable recall of prior mistakes and mitigations.

**Structured provenance:** `FailureRecallHit` carries optional `source_namespace` / `source_tool` from `FailureNode` (e.g. AINL MCP tool failures). `format_failure_prevention_block` appends them to the `_ (source: …)_` line when present so prompts are not dependent on parsing the free-text `message`.
