# Learning frame v1

JSON object passed as the AINL **`frame`** argument (`ainl run … --frame` / `POST /run` body field `frame`) for **learning and skill** graphs. Keys are **flat** so IR resolves them as `intent`, `run_id`, etc.

## Canonical artifacts

| Artifact | Purpose |
|----------|---------|
| [schemas/learning-frame-v1.schema.json](../schemas/learning-frame-v1.schema.json) | JSON Schema (draft 2020-12) |
| `openfang_types::learning_frame` | Rust serde types + `LearningFrameV1::validate` |

## Host integration (ArmaraOS kernel)

Scheduled `ainl run` jobs use `CronAction::AinlRun` with an optional **`frame`** field: a JSON **object** passed to the CLI as `ainl run --frame-json @<tempfile>` (written under the system temp directory, deleted after the run). Same shape as this document. Size is capped when validating cron jobs (256 KiB serialized). Build frames with `LearningFrameV1`, call `validate_defaults()`, then `to_cron_json_value()` for the `frame` field.

### Staging on disk

- **`~/.armaraos/skills/staging/`** — Markdown drafts written by `openfang_kernel::skills_staging::write_skill_draft_markdown` (full Meta block, episode, refs, tags).
- **HTTP:** `POST /api/learning/skill-draft` with a JSON body matching `LearningFrameV1` (same auth as other dashboard API routes when `api_key` is set).

### Opt-in chat capture (`[learn]`)

Messages to **`POST /api/agents/:id/message`** whose body starts with the ASCII case-insensitive prefix **`[learn]`** trigger a staging draft **after** a successful reply. The substring after the prefix becomes `intent` (default: `User-requested skill capture` if empty). The episode includes the full user message and the assistant text returned to the client (including the empty or fallback string when the agent is silent or returns no text). The JSON response may include **`skill_draft_path`** when a file was written. Helpers: `openfang_kernel::skills_staging::learn_prefixed_intent`, `frame_from_agent_learn_turn`.

### AINL reference graph

[`programs/skill-mint-stub/skill_mint_stub.ainl`](../programs/skill-mint-stub/skill_mint_stub.ainl) builds a minimal title + episode body for `ainl run --frame-json`; use it for CLI experiments. The canonical rich template is the Rust renderer above.

## Required fields

| Field | Type | Notes |
|-------|------|--------|
| `frame_version` | `"1"` | Must match for strict validation |
| `op` | string | `capture` \| `consolidate` \| `skill_mint` \| `promote` \| `evolve` |
| `run_id` | string | Non-empty; correlates UI, logs, drafts |
| `intent` | string | One line: what the user was trying to do |
| `outcome` | string | `ok` \| `fail` \| `partial` |
| `episode` | string | Host-redacted narrative; cap size (see below) |

**`tier`** (`unattended` \| `assisted` \| `user`) — optional in JSON; defaults to `assisted` (matches Rust `LearningFrameV1` and JSON Schema default).

## Optional fields

| Field | Type |
|-------|------|
| `scope` | `{ hand_id?, agent_id?, channel? }` |
| `user_note` | string or null — user correction |
| `refs` | `{ trace_uri?, bundle_path?, prior_skill_id? }` — large blobs stay out of frame |
| `artifacts` | `[{ kind, uri, label? }]` |
| `tags` | string[] |
| `locale` | BCP-47 tag |
| `created_at` | RFC3339 UTC |
| `extra` | arbitrary JSON — extensions |

## Size guidance

- **`episode`:** truncate to ~32 KiB before send; full traces live behind `refs`.
- **Total frame:** keep under ~256 KiB serialized JSON to stay within typical `max_frame_bytes` limits in the AINL runtime.

## Example

```json
{
  "frame_version": "1",
  "op": "skill_mint",
  "run_id": "learn-2026-03-31T12-00-00Z-abc",
  "intent": "Export my weekly metrics to Slack without pinging the whole channel",
  "outcome": "ok",
  "episode": "Used http adapter for API, posted to webhook #metrics; user confirmed message shape.",
  "tier": "assisted",
  "user_note": "Always use the internal webhook URL from settings.",
  "refs": { "trace_uri": "file:///…/traces/learn-abc.jsonl" },
  "tags": ["slack", "metrics"]
}
```

## Sample graph

See [`programs/learning-frame-echo/`](../programs/learning-frame-echo/) — minimal graph whose `in:` matches the core keys.
