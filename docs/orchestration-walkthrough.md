# Orchestration walkthrough

This hands-on guide walks through **observing** multi-agent orchestration in ArmaraOS, then **optional** follow-ups (workflows, quotas, task queue stickiness). It complements **`docs/orchestration-guide.md`** (CLI/API index) and **`docs/agent-orchestration-design.md`** (design).

## Prerequisites

- Daemon running (`openfang start`), default API `http://127.0.0.1:4200`.
- At least one agent with tools that delegate (`agent_delegate`, `agent_spawn`, etc.), **or** any registered workflow (even all-**sequential** steps produce `wf:…:run:…` traces — adaptive steps are only required if you want a multi-iteration tool loop inside one step; see **`docs/workflows.md`**).
- Optional: `api_key` in `~/.armaraos/config.toml` for non-loopback auth.

## Step 1 — Confirm the API sees traces

1. Cause a multi-agent interaction (e.g. ask an agent to delegate a subtask to another agent, or **`POST /api/workflows/{id}/run`** with a small pipeline).
2. List recent traces:

```bash
curl -s http://127.0.0.1:4200/api/orchestration/traces?limit=25 | jq .
```

3. Pick a `trace_id` and fetch full detail:

```bash
T=<paste trace_id>
curl -s "http://127.0.0.1:4200/api/orchestration/traces/$T" | jq .
curl -s "http://127.0.0.1:4200/api/orchestration/traces/$T/tree" | jq .
curl -s "http://127.0.0.1:4200/api/orchestration/traces/$T/cost" | jq .
```

You should see `event_type` objects with snake_case `type` fields (`agent_delegated`, `agent_completed`, …).

## Step 2 — Use the dashboard

1. Open the UI (e.g. `http://127.0.0.1:4200/#orchestration-traces`).
2. **Trace list:** filter by id substring; use **From / To** datetime to narrow by `last_event_at`.
3. Select a trace. You should see:
   - **Delegation graph** — indented call tree; click a row to copy the agent id.
   - **Event type filter** — comma-separated substrings (e.g. `agent_completed,agent_failed`) to narrow Gantt and the events JSON.
   - **Timeline (Gantt)** and **token heatmap** from cost rollup.
   - Collapsible **JSON** for cost, tree, and events.

## Step 3 — CLI export

```bash
openfang orchestration export "$T" -o /tmp/trace.json
jq '.events | length' /tmp/trace.json
```

Use `watch` for a live poll while a long run is still active:

```bash
openfang orchestration watch --trace "$T" --interval-secs 2
```

## Step 4 — Quota tree (presentation)

1. Resolve an agent UUID (`GET /api/agents` or the dashboard).
2. On **Orchestration traces**, enter the UUID under **Quota tree** and load.
3. You should see **Quota usage** bars (used vs `max_llm_tokens_per_hour`) plus JSON. Enforcement is server-side; the chart is **presentation** only.

## Step 5 — Workflows that emit traces

Workflow steps run through the same **`send_message_with_handle_and_blocks`** path as chat, with **`OrchestrationPattern::Workflow`** and a per-run **`trace_id`** (`wf:{workflow_uuid}:run:{run_uuid}`). **Adaptive** steps additionally run a **multi-iteration** agent loop (tools, sub-agents) under that context. Register a workflow (`POST /api/workflows`) using one of the JSON bodies in **`docs/workflow-examples.md`**, then `POST /api/workflows/{id}/run` with `{"input":"..."}` and look up the new **`wf:`** trace id in the orchestration list.

## Step 6 — Task queue + same trace (optional)

When an agent posts a task **while** an orchestration context is active, `task_post` merges `orchestration.trace_id` (and `orchestrator_id`) into the task payload. `task_claim` can prefer that trace with `prefer_orchestration_trace_id` or the current live trace. See **`docs/task-queue-orchestration.md`**.

## Step 7 — Triggers on orchestration events (optional)

Agents can register triggers on **`OrchestrationTrace`** patterns (see `crates/openfang-kernel/src/triggers.rs`). When a trace event matches, the dispatched message can include an **`OrchestrationContext`** so the waking agent participates in the same trace. This is **not** the same as the general task queue; it is event-driven.

## Troubleshooting

- **Empty trace list:** no delegation occurred yet, or the ring buffer was trimmed (process-local, capped).
- **404 on `/cost` or `/tree`:** trace id typo or no events for that id.
- **Date filters look wrong:** summaries use `last_event_at` in UTC; local `datetime-local` inputs compare to that instant in the browser.

## See also

- **`docs/orchestration-guide.md`** — CLI table and API list
- **`docs/task-queue-orchestration.md`** — sticky task routing
- **`docs/workflows.md`** — workflow modes and aggregation
- **`docs/agent-orchestration-design.md`** — architecture and status
