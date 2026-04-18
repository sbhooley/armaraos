# ainl-agent-snapshot

Bounded **AgentSnapshot** and **DeterministicPlan** types for the AINL planner protocol (ArmaraOS / inference-server).

- Builds snapshots via typed graph queries (`recall_by_type`, time windows), not unbounded `export_graph`.
- Provides `apply_graph_writes` for semantic / persona / procedural node materialization from planner output.

**Repository:** <https://github.com/sbhooley/armaraos> (crate lives under `crates/ainl-agent-snapshot`).

## License

Apache-2.0 OR MIT.
