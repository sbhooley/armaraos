# Orchestration user guide

This guide covers **multi-agent orchestration** in ArmaraOS: traces, costs, quota trees, workflows, and how to inspect them via the **CLI**, **dashboard**, and **HTTP API**.

## Prerequisites

- **Daemon running** (`openfang start`) for CLI commands that call `http://127.0.0.1:4200` (or your configured listen address). The CLI discovers the daemon via `~/.armaraos/daemon.json` (same mechanism as `openfang workflow list`).
- **`api_key` in `~/.armaraos/config.toml`** — if set, the CLI sends `Authorization: Bearer …` (optional on loopback; matches API behavior).

## CLI (`openfang orchestration`)

All subcommands require a **running daemon**.

| Command | Description |
|--------|-------------|
| `openfang orchestration list [--limit N] [--json]` | Recent trace summaries (`GET /api/orchestration/traces`). |
| `openfang orchestration trace <trace_id> [--json]` | Full event list for one trace. |
| `openfang orchestration cost <trace_id> [--json]` | Token/cost rollup by agent. |
| `openfang orchestration tree <trace_id> [--json]` | Reconstructed delegation tree. |
| `openfang orchestration live <trace_id> [--json]` | Live snapshot (shared_vars / budget) if the trace is still active. |
| `openfang orchestration quota <agent_id> [--json]` | Quota tree for an agent and descendants. |
| `openfang orchestration export <trace_id> [-o file.json]` | Single JSON document: `events`, `tree`, and `cost`. Default: stdout. |
| `openfang orchestration watch [--trace ID] [--interval-secs 3]` | Poll summaries or a trace’s live snapshot until Ctrl+C. |

**Examples**

```bash
openfang orchestration list --limit 20
openfang orchestration trace "$(openfang orchestration list --json | jq -r '.[0].trace_id')"
openfang orchestration export abc-trace-id -o /tmp/trace.json
openfang orchestration watch --trace abc-trace-id --interval-secs 2
```

## Dashboard

- Open the web UI (default `http://127.0.0.1:4200/`), then **Agents → Orchestration** (below **Graph Memory**) or navigate to hash **`#orchestration-traces`**.
- **Trace list:** id substring filter; **From / To** datetime filters on `last_event_at`.
- **Detail (per trace):** **Delegation graph** (indented call tree; click row to copy agent id); **event type filter** (comma-separated, narrows Gantt + events JSON); **Gantt-style timeline**; **token in/out heatmap**; collapsible JSON for cost, tree, and events (`static/js/pages/orchestration-traces.js`, styles in `static/css/layout.css`).
- **Quota tree:** after loading by agent UUID, **quota usage bars** (used vs hourly token cap) plus JSON.

## Tutorials and deep dives

- **`docs/orchestration-walkthrough.md`** — step-by-step: API, dashboard, CLI export, workflows, optional task queue / triggers.
- **`docs/task-queue-orchestration.md`** — how `task_post` / `task_claim` carry `trace_id` for sticky routing.
- **`docs/graph-memory.md`** — **AINL graph memory** (per-agent `ainl_memory.db`): separate SQLite substrate from this trace ring; delegate episodes may embed orchestration JSON for correlation.

## HTTP API

Authoritative route list and response shapes: **`docs/api-reference.md`** (section *Orchestration traces & quota*).

Notable endpoints:

- `GET /api/orchestration/traces?limit=50`
- `GET /api/orchestration/traces/{trace_id}`
- `GET /api/orchestration/traces/{trace_id}/tree`
- `GET /api/orchestration/traces/{trace_id}/cost`
- `GET /api/orchestration/traces/{trace_id}/live`
- `GET /api/orchestration/quota-tree/{agent_id}`

## Workflows and aggregation

Workflow definitions (JSON), adaptive steps, and **collect aggregation** (BestOf, Summarize, etc.) are documented in **`docs/workflows.md`**. Short copy-paste recipes: **`docs/workflow-examples.md`**. Design background: **`docs/agent-orchestration-design.md`**.

**Traces:** Each workflow **run** uses a stable **`trace_id`** of the form `wf:{workflow_uuid}:run:{run_uuid}` across all steps. After **`POST /api/workflows/{id}/run`**, use **`openfang orchestration list`** or the dashboard **`#orchestration-traces`** and filter by that prefix to inspect the run alongside delegate/map-reduce traces.

## Resource limits (§6)

Spawn limits and `inherit_parent_quota` are described in **`docs/agent-orchestration-design.md`** (section 6) and reflected in **`GET /api/orchestration/quota-tree/{agent_id}`** and `openfang orchestration quota`.
