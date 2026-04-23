# ainl-context-freshness

Evaluates whether **repo / tool context** is fresh enough for conservative agent policy (staleness gates before risky tool use).

- **Repository:** <https://github.com/sbhooley/armaraos>
- **API reference:** <https://docs.rs/ainl-context-freshness>

Uses `ainl-contracts` for shared freshness / policy structs. Complements `ainl-context-compiler`, which assembles prompts rather than gating execution.
