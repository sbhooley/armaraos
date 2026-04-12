# Workflow copy-paste examples

Short, **self-contained** JSON bodies you can register with `POST /api/workflows` and run with `POST /api/workflows/{id}/run`. Replace **`YOUR_AGENT`** (and similar) with real agent names from your daemon (`GET /api/agents`). Full field reference: **`docs/workflows.md`**. Each run gets a shared orchestration **`trace_id`** (`wf:…:run:…`); see **`docs/workflows.md`** *Orchestration and traces* to inspect it in the UI or CLI.

## Register and run (curl)

```bash
BASE=http://127.0.0.1:4200
HDR=(-H "Content-Type: application/json")
# Optional if api_key is set in config:
# HDR+=(-H "Authorization: Bearer YOUR_API_KEY")

ID=$(curl -s "${HDR[@]}" -d @workflow.json "$BASE/api/workflows" | jq -r '.workflow_id')
curl -s "${HDR[@]}" -d '{"input":"Hello from curl"}' "$BASE/api/workflows/$ID/run" | jq .
```

Save any example below as `workflow.json` (top-level object with `name`, `description`, `steps`).

---

## 1. Minimal two-step pipeline

Sequential research → short summary. Good sanity check that agents resolve and `{{input}}` / `output_var` work.

```json
{
  "name": "minimal-two-step",
  "description": "Research then summarize",
  "steps": [
    {
      "name": "research",
      "agent_name": "YOUR_AGENT",
      "prompt": "Research in 3 bullet points: {{input}}",
      "output_var": "notes"
    },
    {
      "name": "summarize",
      "agent_name": "YOUR_AGENT",
      "prompt": "One paragraph summary:\n{{notes}}"
    }
  ]
}
```

---

## 2. Parallel brainstorm + default collect

Two `fan_out` steps run together, then `collect` merges with the default separator (`\n\n---\n\n`).

```json
{
  "name": "fanout-collect-default",
  "description": "Two parallel perspectives, then merged text",
  "steps": [
    {
      "name": "angle-a",
      "agent_name": "YOUR_AGENT",
      "prompt": "List 3 pros for: {{input}}",
      "mode": "fan_out"
    },
    {
      "name": "angle-b",
      "agent_name": "YOUR_AGENT",
      "prompt": "List 3 cons for: {{input}}",
      "mode": "fan_out"
    },
    {
      "name": "merge",
      "agent_name": "YOUR_AGENT",
      "prompt": "{{input}}",
      "mode": "collect"
    }
  ]
}
```

---

## 3. Collect as JSON array

Same fan-out pattern, but merged output is a **JSON array string** of each branch’s text (useful for downstream parsing).

```json
{
  "name": "fanout-collect-json-array",
  "description": "Parallel drafts as JSON array",
  "steps": [
    {
      "name": "draft-a",
      "agent_name": "YOUR_AGENT",
      "prompt": "Draft version A for: {{input}}",
      "mode": "fan_out"
    },
    {
      "name": "draft-b",
      "agent_name": "YOUR_AGENT",
      "prompt": "Draft version B for: {{input}}",
      "mode": "fan_out"
    },
    {
      "name": "pack",
      "agent_name": "YOUR_AGENT",
      "prompt": "{{input}}",
      "mode": "collect",
      "collect_aggregation": { "type": "json_array" }
    }
  ]
}
```

---

## 4. Conditional branch

Runs the second step **only if** the previous output contains `REVIEW` (case-insensitive).

```json
{
  "name": "conditional-gate",
  "description": "Optional deep dive when flagged",
  "steps": [
    {
      "name": "triage",
      "agent_name": "YOUR_AGENT",
      "prompt": "Classify. If human review is needed, include the word REVIEW in the first line.\n\n{{input}}"
    },
    {
      "name": "deep-review",
      "agent_name": "YOUR_AGENT",
      "prompt": "Perform a careful review:\n\n{{input}}",
      "mode": "conditional",
      "condition": "REVIEW"
    }
  ]
}
```

---

## 5. Loop until marker

Repeats the step until the output contains `DONE` or `max_iterations` is reached.

```json
{
  "name": "loop-until-done",
  "description": "Iterative refinement",
  "steps": [
    {
      "name": "refine",
      "agent_name": "YOUR_AGENT",
      "prompt": "Improve this text. When satisfied, end with a line containing DONE.\n\n{{input}}",
      "mode": "loop",
      "max_iterations": 4,
      "until": "DONE"
    }
  ]
}
```

---

## 6. Adaptive step (orchestration + traces)

Runs a full agent loop for one step (tools, delegation). Produces **orchestration trace** events viewable under **`#orchestration-traces`** and via `openfang orchestration`.

```json
{
  "name": "adaptive-research-step",
  "description": "Single adaptive step with tool limits",
  "steps": [
    {
      "name": "deep-work",
      "agent_name": "YOUR_AGENT",
      "prompt": "Solve the task using tools if needed:\n\n{{input}}",
      "mode": "adaptive",
      "max_iterations": 12,
      "tool_allowlist": ["web_search", "file_read", "agent_delegate"],
      "allow_subagents": true,
      "timeout_secs": 600
    }
  ]
}
```

---

## 7. Fan-out + best-of merge

Requires a named **evaluator** agent. After parallel drafts, the evaluator picks the best candidate (1-based index in reply).

```json
{
  "name": "fanout-best-of",
  "description": "Two drafts, judge picks one",
  "steps": [
    {
      "name": "draft-a",
      "agent_name": "YOUR_AGENT",
      "prompt": "Draft answer A for: {{input}}",
      "mode": "fan_out"
    },
    {
      "name": "draft-b",
      "agent_name": "YOUR_AGENT",
      "prompt": "Draft answer B for: {{input}}",
      "mode": "fan_out"
    },
    {
      "name": "pick",
      "agent_name": "YOUR_AGENT",
      "prompt": "{{input}}",
      "mode": "collect",
      "collect_aggregation": {
        "type": "best_of",
        "evaluator_agent": "judge",
        "criteria": "Pick the clearest answer; reply with a single digit 1 or 2."
      }
    }
  ]
}
```

Replace **`judge`** with an agent name that exists in your daemon.

---

## 8. Fan-out + consensus (voting)

Uses **consensus** aggregation with a **threshold** (0.0–1.0 fraction of branches agreeing on exact trimmed text).

```json
{
  "name": "fanout-consensus",
  "description": "Parallel classifiers, majority label",
  "steps": [
    {
      "name": "c1",
      "agent_name": "YOUR_AGENT",
      "prompt": "Reply with exactly one word: POSITIVE or NEGATIVE for: {{input}}",
      "mode": "fan_out"
    },
    {
      "name": "c2",
      "agent_name": "YOUR_AGENT",
      "prompt": "Reply with exactly one word: POSITIVE or NEGATIVE for: {{input}}",
      "mode": "fan_out"
    },
    {
      "name": "merge",
      "agent_name": "YOUR_AGENT",
      "prompt": "{{input}}",
      "mode": "collect",
      "collect_aggregation": {
        "type": "consensus",
        "threshold": 0.5
      }
    }
  ]
}
```

---

## See also

- **`docs/workflows.md`** — modes, errors, aggregation strategies, larger examples
- **`docs/orchestration-guide.md`** — CLI and API for traces after adaptive / delegation runs
- **`docs/orchestration-walkthrough.md`** — dashboard + API walkthrough
- **`docs/agent-orchestration-design.md`** — design background
