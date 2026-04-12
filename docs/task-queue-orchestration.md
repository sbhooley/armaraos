# Task queue and orchestration

The shared **task queue** (memory substrate + `task_post` / `task_claim` / `task_complete` tools) integrates with **orchestration traces** so work items can stay associated with the same distributed `trace_id` across agents.

## What is implemented

### Sticky `trace_id` on `task_post`

When an agent calls `task_post` **during** an active orchestration turn, the runtime merges orchestration metadata into the task payload:

- `orchestration.trace_id` — current trace id
- `orchestration.orchestrator_id` — root orchestrator agent id

Implementation: `tool_task_post` in `crates/openfang-runtime/src/tool_runner.rs` (reads `OrchestrationLive`).

**Workflow runs** use the same stickiness: step agents see **`trace_id`** values like `wf:{workflow_uuid}:run:{run_uuid}` (see **`docs/workflows.md`** *Orchestration and traces*), so tasks posted from inside a workflow step carry that id unless the payload overrides it.

### `task_claim` preference

`task_claim` accepts:

- `prefer_orchestration_trace_id` — optional string; if omitted, the tool uses the **current** live trace id when the caller is in an orchestrated turn.
- `strategy` — `default` | `prefer_unassigned` | `sticky_only` (see `openfang_types::task_queue::TaskClaimStrategy`).

The memory layer prefers pending tasks whose JSON payload contains `orchestration.trace_id` matching the preferred id (see `crates/openfang-memory/src/substrate.rs`).

## What is implemented (claim → next turn)

When `task_claim` returns a task whose `payload` includes `orchestration.trace_id` (as merged by `task_post` during an orchestrated turn, or supplied manually in `task_post.payload`):

1. The runtime builds an `OrchestrationContext` via `openfang_types::orchestration::orchestration_context_from_claimed_task` (trace id, orchestrator id, claimant in `call_chain`, task id/title/description in `shared_vars`).
2. It calls `KernelHandle::set_pending_orchestration_ctx` so the **next** LLM turn for that agent picks up the same context (same mechanism as `spawn_agent_with_context` / `pending_orchestration_ctx` in `kernel.rs`).
3. If the agent already has a live orchestration handle for this turn (`OrchestrationLive`), that lock is **updated** so tools later in the **same** iteration see the reconstructed context.

## What is not implemented

- **Smart routing** beyond sticky trace + priority/assignment — no global “best worker” scheduler in the queue itself (product decision: assignment + sticky traces remain the routing surface).

## Claiming when you are already in a different trace

If the worker already has an **`OrchestrationLive`** context (another `trace_id`) and claims a task whose payload carries **different** orchestration metadata, the runtime **replaces** the live lock with the reconstructed context from the task. The **next** user turn still receives the pending context from `set_pending_orchestration_ctx` (same values). Use this when a worker should adopt the trace embedded in the queue item; avoid claiming cross-trace work mid-turn if you need to preserve the prior trace in the same tool iteration.

## Related: triggers (not the task queue)

**Triggers** (`crates/openfang-kernel/src/triggers.rs`): `OrchestrationTrace` event wakes include full trace-aware context. Other patterns wake with a minimal `AdHoc` context (`trigger_id`, `trigger_pattern`, `trigger_event_preview` in `shared_vars`) and a **stable** `trace_id` per trigger (`trigger-wake-<uuid>`) so the trace list does not grow a unique id on every firing. That path is separate from task queue posting.

**Note:** Time-based debouncing of trigger wakes is **not** applied (would risk dropping legitimate bursts); stable `trace_id` is the primary noise-control lever for generic patterns.

## Quick example (conceptual)

1. Agent A runs with trace `abc-123` and posts a task (`task_post`) — payload includes `orchestration.trace_id = "abc-123"`.
2. Agent B calls `task_claim` with `prefer_orchestration_trace_id: "abc-123"` (or runs under the same trace) — claims sticky work first.

## See also

- **`docs/orchestration-walkthrough.md`**
- **`docs/orchestration-guide.md`**
- **`docs/agent-orchestration-design.md`** — §6 / §7 and subsystem notes
