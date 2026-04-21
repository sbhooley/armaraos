# Policy boundaries: ArmaraOS, AINL MCP, optional inference-server

| Layer | Responsibility | Crates / surfaces |
|-------|----------------|-------------------|
| **AINL policy crates** | Portable contracts and pure policy (no OpenFang deps). | `ainl-contracts`, `ainl-repo-intel`, `ainl-context-freshness`, `ainl-impact-policy` |
| **OpenFang / ArmaraOS** | MCP transport, connection lifecycle, API/UI, tool execution. Adapters only. | `openfang-runtime::ainl_policy`, `openfang-api` `/api/mcp/servers` |
| **AI_Native_Lang MCP** | Authoring loop: validate → compile → IR diff → run; MCP resources. | `scripts/ainl_mcp_server.py`, `tooling/mcp_exposure_profiles.json` |
| **ainl-inference-server** (optional) | Planner telemetry and request shaping when deployed. | Not required for public baseline behavior. |

**Repo intelligence MCP** (e.g. GitNexus): classified by `ainl-repo-intel`; readiness appears in `GET /api/mcp/servers` under `readiness.checks.repo_intelligence` and `repo_intelligence.workspace_profile`.

**Related:** [`mcp-a2a.md`](mcp-a2a.md) — MCP setup; [AI_Native_Lang docs/AINL_GRAPH_VOCABULARY.md](../../AI_Native_Lang/docs/AINL_GRAPH_VOCABULARY.md) — vocabulary.
