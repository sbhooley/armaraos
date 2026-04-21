# Public baseline evaluation thresholds (policy / MCP)

Regression tests and dashboards should treat these as **guardrails** (tune per release).

| Metric | Pass | Notes |
|--------|------|--------|
| `recommended_next_tools` present on validate/compile (MCP) | 100% of sampled responses | `scripts/ainl_mcp_server.py` |
| `policy_contract.context_freshness` | Always `"unknown"` in MCP-only hosts unless extended | Expected default |
| `readiness.checks.repo_intelligence.ready` | `true` when GitNexus-class query+impact tools connected | `GET /api/mcp/servers` |
| Unsafe tool routing (no prior validate in strict mode) | 0 in golden harness | Planned; deterministic `Run` deferral + dashboard policy block reduce bypass risk today |

**Telemetry field names** (stable): see `ainl_contracts::telemetry` and `tooling/ainl_policy_contract.json`.
