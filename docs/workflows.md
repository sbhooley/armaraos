# Workflow Engine Guide

**Quick copy-paste:** see **`docs/workflow-examples.md`** for compact JSON and a curl register/run recipe. **Hands-on:** **`docs/orchestration-walkthrough.md`**.

## Overview

The ArmaraOS workflow engine enables multi-step agent pipelines -- orchestrated sequences of tasks where each step routes work to a specific agent, and output from one step flows as input to the next. Workflows let you compose complex behaviors from simple, single-purpose agents without writing any Rust code.

Use workflows when you need to:

- Chain multiple agents together in a processing pipeline (e.g., research then write then review).
- Fan work out to several agents in parallel and collect their results.
- Conditionally branch execution based on an earlier step's output.
- Iterate a step in a loop until a quality gate is met.
- Run an **adaptive** step: a full multi-iteration agent loop (tools, sub-agents, orchestration context) inside one workflow step.
- Build reproducible, auditable multi-agent processes that can be triggered via API or CLI.

The implementation lives in `openfang-kernel/src/workflow.rs`. The workflow engine is decoupled from the kernel through closures -- it never directly owns or references the kernel, making it testable in isolation.

---

## Orchestration and traces (§1)

When the **kernel** runs a workflow (`OpenFangKernel::run_workflow`) or the **HTTP channel bridge** runs workflow text (`run_workflow_text` in `openfang-api`), each step that enters the normal **agent / LLM** path does so with **`Some(OrchestrationContext)`**:

| Field / behavior | Value |
|------------------|--------|
| **`pattern`** | `OrchestrationPattern::Workflow { workflow_id, step_index, step_name }` — identifies the definition and the step inside the run. |
| **`trace_id`** | One id per **run**, shared by every step: `wf:{workflow_uuid}:run:{run_uuid}`. Filter this in **Monitor → Orchestration traces** (`#orchestration-traces`) or `openfang orchestration …`. |
| **`orchestrator_id` / `call_chain`** | Rooted at the **step’s target agent** for that turn; nested **`agent_send`** / **`agent_spawn`** extend the chain as usual. |
| **`remaining_budget_ms`** | If unset when the context is built, the kernel may set it from **`[runtime_limits] orchestration_default_budget_ms`** in `config.toml` (`RuntimeLimitsConfig` in `openfang-types`). |

**Adaptive** mode is special only in **length of the turn**: it runs a **multi-iteration** agent loop with optional tool allowlists and overrides. **Sequential**, **loop** iterations, **fan_out** branches, **conditional** steps, and agent-driven **collect** (BestOf / Summarize / Custom) still receive the same **Workflow** pattern for each **engine** invocation of the step callback.

**Engine wiring:** `WorkflowEngine::execute_run` passes a **`step_index: usize`** into the injected async sender so the **`step_index`** inside **`OrchestrationPattern::Workflow`** matches the engine’s ordering (including fan-out list indices and loop iteration counts).

---

## Core Types

| Rust type | Description |
|---|---|
| `WorkflowId(Uuid)` | Unique identifier for a workflow definition. |
| `WorkflowRunId(Uuid)` | Unique identifier for a running workflow instance. |
| `Workflow` | A named definition containing a list of `WorkflowStep` entries. |
| `WorkflowStep` | A single step: agent reference, prompt template, mode, timeout, error handling. |
| `WorkflowRun` | A running instance: tracks state, step results, final output, timestamps. |
| `WorkflowRunState` | Enum: `Pending`, `Running`, `Completed`, `Failed`. |
| `StepResult` | Result from one step: agent info, output text, token counts, duration. |
| `WorkflowEngine` | The engine itself: stores definitions and runs in `Arc<RwLock<HashMap>>`. |

---

## Workflow Definition

Workflows are registered via the REST API as JSON. The top-level structure is:

```json
{
  "name": "my-pipeline",
  "description": "Describe what the workflow does",
  "steps": [ ... ]
}
```

The corresponding Rust struct is:

```rust
pub struct Workflow {
    pub id: WorkflowId,            // Auto-assigned on creation
    pub name: String,              // Human-readable name
    pub description: String,       // What this workflow does
    pub steps: Vec<WorkflowStep>,  // Ordered list of steps
    pub created_at: DateTime<Utc>, // Auto-assigned on creation
}
```

---

## Step Configuration

Each step in the `steps` array has the following fields:

| JSON field | Rust field | Type | Default | Description |
|---|---|---|---|---|
| `name` | `name` | `String` | `"step"` | Step name for logging and display. |
| `agent_name` | `agent` | `StepAgent::ByName` | -- | Reference an agent by its name (first match). Mutually exclusive with `agent_id`. |
| `agent_id` | `agent` | `StepAgent::ById` | -- | Reference an agent by its UUID. Mutually exclusive with `agent_name`. |
| `prompt` | `prompt_template` | `String` | `"{{input}}"` | Prompt template with variable placeholders. |
| `mode` | `mode` | `StepMode` | `"sequential"` | Execution mode (see below). |
| `timeout_secs` | `timeout_secs` | `u64` | `120` | Maximum time in seconds before the step times out. |
| `error_mode` | `error_mode` | `ErrorMode` | `"fail"` | How to handle errors (see below). |
| `max_retries` | (inside `ErrorMode::Retry`) | `u32` | `3` | Number of retries when `error_mode` is `"retry"`. |
| `output_var` | `output_var` | `Option<String>` | `null` | If set, stores this step's output in a named variable for later reference. |
| `condition` | (inside `StepMode::Conditional`) | `String` | `""` | Substring to match in previous output (case-insensitive). |
| `max_iterations` | (`StepMode::Loop` or `StepMode::Adaptive`) | `u32` | `5` (loop), `20` (adaptive) | Loop: max iterations before exit. Adaptive: max agent-loop iterations for the step (`POST /api/workflows` defaults). |
| `until` | (inside `StepMode::Loop`) | `String` | `""` | Substring to match in output to terminate the loop (case-insensitive). |
| `tool_allowlist` | (when `mode` is `"adaptive"`) | `string[]` | `null` | If set, restricts which tools the agent may use during this step. |
| `allow_subagents` | (when `mode` is `"adaptive"`) | `bool` | `true` | Whether `agent_spawn` / delegation tools are allowed for this step. |
| `max_tokens` | (when `mode` is `"adaptive"`) | `u64` | `null` | Optional token budget hint for the adaptive step. |
| `collect_aggregation` | `collect_aggregation` | object | `null` | When `mode` is `"collect"`, how to merge fan-out outputs. Tagged JSON: `type` is `concatenate` \| `json_array` \| `consensus` \| `best_of` \| `summarize` \| `custom` (see **Collect** below). |

### Agent Resolution

Every step must specify exactly one of `agent_name` or `agent_id`. The `StepAgent` enum is:

```rust
pub enum StepAgent {
    ById { id: String },    // UUID of an existing agent
    ByName { name: String }, // Name match (first agent with this name)
}
```

If the agent cannot be resolved at execution time, the workflow fails with `"Agent not found for step '<name>'"`.

---

## Step Modes

The `mode` field controls how a step executes relative to other steps in the workflow.

### Sequential (default)

```json
{ "mode": "sequential" }
```

The step runs after the previous step completes. The previous step's output becomes `{{input}}` for this step. This is the default mode when `mode` is omitted.

### Fan-Out

```json
{ "mode": "fan_out" }
```

Fan-out steps run **in parallel**. The engine collects all consecutive `fan_out` steps and launches them simultaneously using `futures::future::join_all`. All fan-out steps receive the same `{{input}}` -- the output from the last step that ran before the fan-out group.

If any fan-out step fails or times out, the entire workflow fails immediately.

### Collect

```json
{ "mode": "collect" }
```

The `collect` step merges accumulated fan-out outputs and sets the result as `{{input}}` for subsequent steps.

**Default:** concatenate with separator `"\n\n---\n\n"` (same as `{"type":"concatenate","separator":"\n\n---\n\n"}`).

**Optional `collect_aggregation`** (per step, `POST` / `PUT /api/workflows` and on-disk JSON): tagged object, `type` in `snake_case` matching serde:

| `type` | Fields | Behavior |
|--------|--------|----------|
| `concatenate` | `separator` (string) | Join with separator |
| `json_array` | — | JSON array string of all outputs |
| `consensus` | `threshold` (f32, 0–1) | Plurality / threshold vote on trimmed outputs |
| `best_of` | `evaluator_agent` (name), `criteria` (string) | Evaluator must reply with a **1-based** candidate index; merged value is that fan-out output |
| `summarize` | `summarizer_agent` (name), `max_length` (optional usize) | Summarizer’s reply becomes the merged value |
| `custom` | `aggregator_agent` (name), `aggregation_prompt` (string) | Prompt expanded with `{{outputs}}`, `{{outputs_json}}`, `{{input}}` (workflow initial), `{{var}}` |

Agent-driven strategies (`best_of`, `summarize`, `custom`) invoke the named agent via the same runtime path as other steps; an extra `StepResult` row is recorded (`*:best_of`, `*:summarize`, or `*:custom`). With **one** fan-out output, the agent is not called (that output is passed through).

Example (`best_of`):

```json
{
  "name": "pick-best",
  "agent_name": "dummy",
  "prompt": "{{input}}",
  "mode": "collect",
  "collect_aggregation": {
    "type": "best_of",
    "evaluator_agent": "judge",
    "criteria": "accuracy and clarity"
  }
}
```

(`agent_name` is still required by the API schema; for pure collect it can point at any existing agent — only `evaluator_agent` / `summarizer_agent` / `aggregator_agent` are used for merging when those strategies are set.)

A typical fan-out/collect pattern:

```
step 1: fan_out  -->  runs in parallel
step 2: fan_out  -->  runs in parallel
step 3: collect  -->  joins outputs from steps 1 and 2
step 4: sequential --> receives joined output as {{input}}
```

### Conditional

```json
{ "mode": "conditional", "condition": "ERROR" }
```

The step only executes if the previous step's output **contains** the `condition` substring (case-insensitive comparison via `to_lowercase().contains()`). If the condition is not met, the step is skipped entirely and `{{input}}` is not modified.

When the condition is met, the step executes like a sequential step.

### Loop

```json
{ "mode": "loop", "max_iterations": 5, "until": "APPROVED" }
```

The step repeats up to `max_iterations` times. After each iteration, the engine checks whether the output **contains** the `until` substring (case-insensitive). If found, the loop terminates early.

Each iteration feeds its output back as `{{input}}` for the next iteration. Step results are recorded with names like `"refine (iter 1)"`, `"refine (iter 2)"`, etc.

If the `until` condition is never met, the loop runs exactly `max_iterations` times and continues to the next step with the last iteration's output.

### Adaptive

```json
{
  "mode": "adaptive",
  "max_iterations": 10,
  "tool_allowlist": ["web_search", "agent_spawn", "agent_delegate"],
  "allow_subagents": true,
  "max_tokens": 50000
}
```

An **adaptive** step runs a **full agent loop** for one user message (the expanded `prompt`), not a single model reply. Like other LLM steps, it receives **`OrchestrationPattern::Workflow`** and the shared per-run **`trace_id`** (see **Orchestration and traces** above); the kernel additionally passes **adaptive overrides** (`max_iterations`, `tool_allowlist`, `allow_subagents`, `max_tokens`) into the same path as interactive chat (`send_message_with_handle_and_blocks`). Design detail: [agent-orchestration-design.md](agent-orchestration-design.md) §1 / §4.

**REST shape (`POST /api/workflows`):** keep `mode` as the string `"adaptive"` and put the fields above as **siblings** of `name`, `agent_name`, `prompt`, etc. Do **not** nest them under an `"adaptive"` object (that shape is for raw `Workflow` JSON deserialized directly by serde).

**Defaults** (from `crates/openfang-api/src/routes.rs`): `max_iterations` defaults to **20** if omitted; `allow_subagents` defaults to **true**; `tool_allowlist` and `max_tokens` default to unset.

**Example step** (after a planning step stored `outline` in `output_var`):

```json
{
  "name": "deep-research",
  "agent_name": "researcher",
  "prompt": "Research thoroughly. Topic: {{input}}\nOutline: {{outline}}",
  "mode": "adaptive",
  "max_iterations": 10,
  "tool_allowlist": ["web_search", "agent_spawn", "agent_coordinate"],
  "allow_subagents": true,
  "timeout_secs": 900,
  "output_var": "research"
}
```

---

## Variable Substitution

Prompt templates support two kinds of variable references:

### `{{input}}` -- Previous step output

Always available. Contains the output from the immediately preceding step (or the workflow's initial input for the first step).

### `{{variable_name}}` -- Named variables

When a step has `"output_var": "my_var"`, its output is stored in a variable map under the key `my_var`. Any subsequent step can reference it with `{{my_var}}` in its prompt template.

The expansion logic (from `WorkflowEngine::expand_variables`):

```rust
fn expand_variables(template: &str, input: &str, vars: &HashMap<String, String>) -> String {
    let mut result = template.replace("{{input}}", input);
    for (key, value) in vars {
        result = result.replace(&format!("{{{{{key}}}}}"), value);
    }
    result
}
```

Variables persist for the entire workflow run. A later step can overwrite a variable by using the same `output_var` name.

**Example**: A three-step workflow where step 3 references outputs from both step 1 and step 2:

```json
{
  "steps": [
    { "name": "research", "output_var": "research_output", "prompt": "Research: {{input}}" },
    { "name": "outline",  "output_var": "outline_output",  "prompt": "Outline based on: {{input}}" },
    { "name": "combine",  "prompt": "Write article.\nResearch: {{research_output}}\nOutline: {{outline_output}}" }
  ]
}
```

---

## Error Handling

Each step has an `error_mode` that controls behavior when the step fails or times out.

### Fail (default)

```json
{ "error_mode": "fail" }
```

The workflow aborts immediately. The run state is set to `Failed`, the error message is recorded, and `completed_at` is set. The error message format is `"Step '<name>' failed: <error>"` or `"Step '<name>' timed out after <N>s"`.

### Skip

```json
{ "error_mode": "skip" }
```

The step is silently skipped on error or timeout. A warning is logged, but the workflow continues. The `{{input}}` for the next step remains unchanged (it keeps the value from before the skipped step). No `StepResult` is recorded for the skipped step.

### Retry

```json
{ "error_mode": "retry", "max_retries": 3 }
```

The step is retried up to `max_retries` times after the initial attempt (so `max_retries: 3` means up to 4 total attempts: 1 initial + 3 retries). Each attempt gets the full `timeout_secs` budget independently. If all attempts fail, the workflow aborts with `"Step '<name>' failed after <N> retries: <last_error>"`.

### Timeout Behavior

Every step execution is wrapped in `tokio::time::timeout(Duration::from_secs(step.timeout_secs), ...)`. The default timeout is 120 seconds. Timeouts are treated as errors and handled according to the step's `error_mode`.

For fan-out steps, each parallel step gets its own timeout individually.

---

## Examples

### Example 1: Code Review Pipeline

A sequential pipeline where code is analyzed, reviewed, and a summary is produced.

```json
{
  "name": "code-review-pipeline",
  "description": "Analyze code, review for issues, and produce a summary report",
  "steps": [
    {
      "name": "analyze",
      "agent_name": "code-reviewer",
      "prompt": "Analyze the following code for bugs, style issues, and security vulnerabilities:\n\n{{input}}",
      "mode": "sequential",
      "timeout_secs": 180,
      "error_mode": "fail",
      "output_var": "analysis"
    },
    {
      "name": "security-check",
      "agent_name": "security-auditor",
      "prompt": "Review this code analysis for security issues. Flag anything critical:\n\n{{analysis}}",
      "mode": "sequential",
      "timeout_secs": 120,
      "error_mode": "retry",
      "max_retries": 2,
      "output_var": "security_review"
    },
    {
      "name": "summary",
      "agent_name": "writer",
      "prompt": "Write a concise code review summary.\n\nCode Analysis:\n{{analysis}}\n\nSecurity Review:\n{{security_review}}",
      "mode": "sequential",
      "timeout_secs": 60,
      "error_mode": "fail"
    }
  ]
}
```

### Example 2: Research and Write Article

Research a topic, outline it, then write -- with a conditional fact-check step.

```json
{
  "name": "research-and-write",
  "description": "Research a topic, outline, write, and optionally fact-check",
  "steps": [
    {
      "name": "research",
      "agent_name": "researcher",
      "prompt": "Research the following topic thoroughly. Cite sources where possible:\n\n{{input}}",
      "mode": "sequential",
      "timeout_secs": 300,
      "error_mode": "retry",
      "max_retries": 1,
      "output_var": "research"
    },
    {
      "name": "outline",
      "agent_name": "planner",
      "prompt": "Create a detailed article outline based on this research:\n\n{{research}}",
      "mode": "sequential",
      "timeout_secs": 60,
      "output_var": "outline"
    },
    {
      "name": "write",
      "agent_name": "writer",
      "prompt": "Write a complete article.\n\nOutline:\n{{outline}}\n\nResearch:\n{{research}}",
      "mode": "sequential",
      "timeout_secs": 300,
      "output_var": "article"
    },
    {
      "name": "fact-check",
      "agent_name": "analyst",
      "prompt": "Fact-check this article and note any claims that need verification:\n\n{{article}}",
      "mode": "conditional",
      "condition": "claim",
      "timeout_secs": 120,
      "error_mode": "skip"
    }
  ]
}
```

The fact-check step only runs if the article contains the word "claim" (case-insensitive). If the fact-check agent fails, the workflow continues with the article as-is.

### Example 3: Multi-Agent Brainstorm (Fan-Out + Collect)

Three agents brainstorm in parallel, then a fourth agent synthesizes their ideas.

```json
{
  "name": "brainstorm",
  "description": "Parallel brainstorm with 3 agents, then synthesize",
  "steps": [
    {
      "name": "creative-ideas",
      "agent_name": "writer",
      "prompt": "Brainstorm 5 creative ideas for: {{input}}",
      "mode": "fan_out",
      "timeout_secs": 60,
      "output_var": "creative"
    },
    {
      "name": "technical-ideas",
      "agent_name": "architect",
      "prompt": "Brainstorm 5 technically feasible ideas for: {{input}}",
      "mode": "fan_out",
      "timeout_secs": 60,
      "output_var": "technical"
    },
    {
      "name": "business-ideas",
      "agent_name": "analyst",
      "prompt": "Brainstorm 5 ideas with strong business potential for: {{input}}",
      "mode": "fan_out",
      "timeout_secs": 60,
      "output_var": "business"
    },
    {
      "name": "gather",
      "agent_name": "planner",
      "prompt": "unused",
      "mode": "collect"
    },
    {
      "name": "synthesize",
      "agent_name": "orchestrator",
      "prompt": "You received brainstorm results from three perspectives. Synthesize them into the top 5 actionable ideas, ranked by impact:\n\n{{input}}",
      "mode": "sequential",
      "timeout_secs": 120
    }
  ]
}
```

The three fan-out steps run in parallel. The `collect` step joins their outputs with `---` separators. The `synthesize` step receives the combined output.

### Example 4: Iterative Refinement (Loop)

An agent refines a draft until it meets a quality bar.

```json
{
  "name": "iterative-refinement",
  "description": "Refine a document until approved or max iterations reached",
  "steps": [
    {
      "name": "first-draft",
      "agent_name": "writer",
      "prompt": "Write a first draft about: {{input}}",
      "mode": "sequential",
      "timeout_secs": 120,
      "output_var": "draft"
    },
    {
      "name": "review-and-refine",
      "agent_name": "code-reviewer",
      "prompt": "Review this draft. If it meets quality standards, respond with APPROVED at the start. Otherwise, provide specific feedback and a revised version:\n\n{{input}}",
      "mode": "loop",
      "max_iterations": 4,
      "until": "APPROVED",
      "timeout_secs": 180,
      "error_mode": "retry",
      "max_retries": 1
    }
  ]
}
```

The loop runs the reviewer up to 4 times. Each iteration receives the previous iteration's output as `{{input}}`. Once the reviewer includes "APPROVED" in its response, the loop terminates early.

---

## Trigger Engine

The trigger engine (`openfang-kernel/src/triggers.rs`) provides event-driven automation. Triggers watch the kernel's event bus and automatically send messages to agents when matching events arrive.

### Core Types

| Rust type | Description |
|---|---|
| `TriggerId(Uuid)` | Unique identifier for a trigger. |
| `Trigger` | A registered trigger: agent, pattern, prompt template, fire count, limits. |
| `TriggerPattern` | Enum defining which events to match. |
| `TriggerEngine` | The engine: `DashMap`-backed concurrent storage with agent-to-trigger index. |

### Trigger Definition

```rust
pub struct Trigger {
    pub id: TriggerId,
    pub agent_id: AgentId,         // Which agent receives the message
    pub pattern: TriggerPattern,   // What events to match
    pub prompt_template: String,   // Template with {{event}} placeholder
    pub enabled: bool,             // Can be toggled on/off
    pub created_at: DateTime<Utc>,
    pub fire_count: u64,           // How many times it has fired
    pub max_fires: u64,            // 0 = unlimited
}
```

### Event Patterns

The `TriggerPattern` enum supports 9 matching modes:

| Pattern | JSON | Description |
|---|---|---|
| `All` | `"all"` | Matches every event (wildcard). |
| `Lifecycle` | `"lifecycle"` | Matches any lifecycle event (spawned, started, terminated, etc.). |
| `AgentSpawned` | `{"agent_spawned": {"name_pattern": "coder"}}` | Matches when an agent with a name containing `name_pattern` is spawned. Use `"*"` for any agent. |
| `AgentTerminated` | `"agent_terminated"` | Matches when any agent terminates or crashes. |
| `System` | `"system"` | Matches any system event (health checks, quota warnings, etc.). |
| `SystemKeyword` | `{"system_keyword": {"keyword": "quota"}}` | Matches system events whose debug representation contains the keyword (case-insensitive). |
| `MemoryUpdate` | `"memory_update"` | Matches any memory change event. |
| `MemoryKeyPattern` | `{"memory_key_pattern": {"key_pattern": "config"}}` | Matches memory updates where the key contains `key_pattern`. Use `"*"` for any key. |
| `ContentMatch` | `{"content_match": {"substring": "error"}}` | Matches any event whose human-readable description contains the substring (case-insensitive). |

### Pattern Matching Details

The `matches_pattern` function determines how each pattern evaluates:

- **`All`**: Always returns `true`.
- **`Lifecycle`**: Checks `EventPayload::Lifecycle(_)`.
- **`AgentSpawned`**: Checks for `LifecycleEvent::Spawned` where `name.contains(name_pattern)` or `name_pattern == "*"`.
- **`AgentTerminated`**: Checks for `LifecycleEvent::Terminated` or `LifecycleEvent::Crashed`.
- **`System`**: Checks `EventPayload::System(_)`.
- **`SystemKeyword`**: Formats the system event via `Debug` trait, lowercases it, and checks `contains(keyword)`.
- **`MemoryUpdate`**: Checks `EventPayload::MemoryUpdate(_)`.
- **`MemoryKeyPattern`**: Checks `delta.key.contains(key_pattern)` or `key_pattern == "*"`.
- **`ContentMatch`**: Uses the `describe_event()` function to produce a human-readable string, then checks `contains(substring)` (case-insensitive).

### Prompt Template and `{{event}}`

When a trigger fires, the engine replaces `{{event}}` in the `prompt_template` with a human-readable event description. The `describe_event()` function produces strings like:

- `"Agent 'coder' (id: <uuid>) was spawned"`
- `"Agent <uuid> terminated: shutdown requested"`
- `"Agent <uuid> crashed: out of memory"`
- `"Kernel started"`
- `"Quota warning: agent <uuid>, tokens at 85.0%"`
- `"Health check failed: agent <uuid>, unresponsive for 30s"`
- `"Memory Created on key 'config' for agent <uuid>"`
- `"Tool 'web_search' succeeded (450ms): ..."`

### Max Fires and Auto-Disable

When `max_fires` is set to a value greater than 0, the trigger automatically disables itself (sets `enabled = false`) once `fire_count >= max_fires`. Setting `max_fires` to 0 means the trigger fires indefinitely.

### Trigger Use Cases

**Monitor agent health:**
```json
{
  "agent_id": "<ops-agent-uuid>",
  "pattern": {"content_match": {"substring": "health check failed"}},
  "prompt_template": "ALERT: {{event}}. Investigate and report the status of all agents.",
  "max_fires": 0
}
```

**React to new agent spawns:**
```json
{
  "agent_id": "<orchestrator-uuid>",
  "pattern": {"agent_spawned": {"name_pattern": "*"}},
  "prompt_template": "A new agent was just created: {{event}}. Update the fleet roster.",
  "max_fires": 0
}
```

**One-shot quota alert:**
```json
{
  "agent_id": "<admin-agent-uuid>",
  "pattern": {"system_keyword": {"keyword": "quota"}},
  "prompt_template": "Quota event detected: {{event}}. Recommend corrective action.",
  "max_fires": 1
}
```

---

## API Endpoints

### Workflow Endpoints

#### `POST /api/workflows` -- Create a workflow

Register a new workflow definition.

**Request body:**
```json
{
  "name": "my-pipeline",
  "description": "Description of the workflow",
  "steps": [
    {
      "name": "step-1",
      "agent_name": "researcher",
      "prompt": "Research: {{input}}",
      "mode": "sequential",
      "timeout_secs": 120,
      "error_mode": "fail",
      "output_var": "research"
    },
    {
      "name": "step-2-adaptive",
      "agent_name": "researcher",
      "prompt": "Go deeper on: {{input}}\nPrior notes: {{research}}",
      "mode": "adaptive",
      "max_iterations": 8,
      "allow_subagents": true,
      "timeout_secs": 600
    }
  ]
}
```

See **Step Modes → Adaptive** above for all optional fields (`tool_allowlist`, `max_tokens`, defaults).

**Response (201 Created):**
```json
{ "workflow_id": "<uuid>" }
```

#### `GET /api/workflows` -- List all workflows

Returns an array of registered workflow summaries.

**Response (200 OK):**
```json
[
  {
    "id": "<uuid>",
    "name": "my-pipeline",
    "description": "Description of the workflow",
    "steps": 3,
    "created_at": "2026-01-15T10:30:00Z"
  }
]
```

#### `POST /api/workflows/:id/run` -- Execute a workflow

Start a synchronous workflow execution. The call blocks until the workflow completes or fails.

**Request body:**
```json
{ "input": "The initial input text for the first step" }
```

**Response (200 OK):**
```json
{
  "run_id": "<uuid>",
  "output": "Final output from the last step",
  "status": "completed"
}
```

**Response (500 Internal Server Error):**
```json
{ "error": "Workflow execution failed" }
```

#### `GET /api/workflows/:id/runs` -- List workflow runs

Returns all workflow runs (not filtered by workflow ID in the current implementation).

**Response (200 OK):**
```json
[
  {
    "id": "<uuid>",
    "workflow_name": "my-pipeline",
    "state": "completed",
    "steps_completed": 3,
    "started_at": "2026-01-15T10:30:00Z",
    "completed_at": "2026-01-15T10:32:15Z"
  }
]
```

### Trigger Endpoints

#### `POST /api/triggers` -- Create a trigger

Register a new event trigger for an agent.

**Request body:**
```json
{
  "agent_id": "<agent-uuid>",
  "pattern": "lifecycle",
  "prompt_template": "A lifecycle event occurred: {{event}}",
  "max_fires": 0
}
```

**Response (201 Created):**
```json
{
  "trigger_id": "<uuid>",
  "agent_id": "<agent-uuid>"
}
```

#### `GET /api/triggers` -- List all triggers

Optionally filter by agent: `GET /api/triggers?agent_id=<uuid>`

**Response (200 OK):**
```json
[
  {
    "id": "<uuid>",
    "agent_id": "<agent-uuid>",
    "pattern": "lifecycle",
    "prompt_template": "Event: {{event}}",
    "enabled": true,
    "fire_count": 5,
    "max_fires": 0,
    "created_at": "2026-01-15T10:00:00Z"
  }
]
```

#### `PUT /api/triggers/:id` -- Enable/disable a trigger

Toggle a trigger's enabled state.

**Request body:**
```json
{ "enabled": false }
```

**Response (200 OK):**
```json
{ "status": "updated", "trigger_id": "<uuid>", "enabled": false }
```

#### `DELETE /api/triggers/:id` -- Delete a trigger

**Response (200 OK):**
```json
{ "status": "removed", "trigger_id": "<uuid>" }
```

**Response (404 Not Found):**
```json
{ "error": "Trigger not found" }
```

---

## CLI Commands

All workflow and trigger CLI commands require a running ArmaraOS daemon.

### Workflow Commands

```
openfang workflow list
```
Lists all registered workflows with their ID, name, step count, and creation date.

```
openfang workflow create <file>
```
Creates a workflow from a JSON file. The file should contain the same JSON structure as the `POST /api/workflows` request body.

```
openfang workflow run <workflow_id> <input>
```
Executes a workflow by its UUID with the given input text. Blocks until completion and prints the output.

### Trigger Commands

```
openfang trigger list [--agent-id <uuid>]
```
Lists all registered triggers. Optionally filter by agent ID.

```
openfang trigger create <agent_id> <pattern_json> [--prompt <template>] [--max-fires <n>]
```
Creates a trigger for the specified agent. The `pattern_json` argument is a JSON string describing the pattern.

Defaults:
- `--prompt`: `"Event: {{event}}"`
- `--max-fires`: `0` (unlimited)

Examples:
```bash
# Watch all lifecycle events
openfang trigger create <agent-id> '"lifecycle"' --prompt "Lifecycle: {{event}}"

# Watch for a specific agent spawn
openfang trigger create <agent-id> '{"agent_spawned":{"name_pattern":"coder"}}' --max-fires 1

# Watch for content containing "error"
openfang trigger create <agent-id> '{"content_match":{"substring":"error"}}'
```

```
openfang trigger delete <trigger_id>
```
Deletes a trigger by its UUID.

---

## Execution Limits

### Run Eviction Cap

The workflow engine retains a maximum of **200** workflow runs (`WorkflowEngine::MAX_RETAINED_RUNS`). When this limit is exceeded after creating a new run, the oldest **completed** or **failed** runs are evicted (sorted by `started_at`). Runs in `Pending` or `Running` state are never evicted.

### Step Timeouts

Each step has a configurable `timeout_secs` (default: 120 seconds). The timeout is enforced via `tokio::time::timeout` and applies per-attempt -- retry mode gives each attempt a fresh timeout budget. Fan-out steps each get their own independent timeout.

### Loop Iteration Cap

Loop steps are bounded by `max_iterations` (default: 5 in the API). The engine will never execute more than this many iterations, even if the `until` condition is never met.

### Hourly Token Quota

The `AgentScheduler` (in `openfang-kernel/src/scheduler.rs`) tracks per-agent token usage with a rolling 1-hour window via `UsageTracker`. If an agent exceeds its `ResourceQuota.max_llm_tokens_per_hour`, the scheduler returns `ArmaraOSError::QuotaExceeded`. The window resets automatically after 3600 seconds. This quota applies to all agent interactions, including those invoked by workflows.

---

## Workflow Data Flow Diagram

```
                    input
                      |
                      v
              +---------------+
              |   Step 1      |  mode: sequential
              |   agent: A    |
              +-------+-------+
                      | output -> {{input}} for step 2
                      |          -> variables["var1"] if output_var set
                      v
              +---------------+
              |   Step 2      |  mode: fan_out
              |   agent: B    |---+
              +---------------+   |
              +---------------+   |  parallel execution
              |   Step 3      |   |  (all receive same {{input}})
              |   agent: C    |---+
              +---------------+   |
                      |           |
                      v           v
              +---------------+
              |   Step 4      |  mode: collect
              |   (no agent)  |  joins all outputs with "---"
              +-------+-------+
                      | combined output -> {{input}}
                      v
              +---------------+
              |   Step 5      |  mode: conditional { condition: "issue" }
              |   agent: D    |  (skipped if {{input}} does not contain "issue")
              +-------+-------+
                      |
                      v
              +---------------+
              |   Step 6      |  mode: loop { max_iterations: 3, until: "DONE" }
              |   agent: E    |  repeats, feeding output back as {{input}}
              +-------+-------+
                      |
                      v
                 final output
```

---

## Internal Architecture Notes

- The `WorkflowEngine` is decoupled from `OpenFangKernel`. The `execute_run` method takes two closures: `agent_resolver` (resolves `StepAgent` to `AgentId` + name) and `send_message` (sends a prompt to an agent and returns output + token counts). This design makes the engine testable without a live kernel.
- All state is held in `Arc<RwLock<HashMap>>`, allowing concurrent read access and serialized writes.
- The `TriggerEngine` uses `DashMap` for lock-free concurrent access, with an `agent_triggers` index for efficient per-agent trigger lookups.
- Fan-out parallelism uses `futures::future::join_all` -- all fan-out steps in a consecutive group are launched simultaneously.
- The trigger `evaluate` method uses `iter_mut()` on the `DashMap` to atomically increment fire counts while checking patterns, preventing race conditions.
