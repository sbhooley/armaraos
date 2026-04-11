# Lead-gen pipeline (AINL showcase)

Production-flavored **scan → enrich → score** flow using only **read-only HTTPS** (same host family as `new_version_checker`) plus **`core`** heuristics. Output is JSON (`ainl run --json`); when executed as a **curated cron** job, the kernel appends that JSON to the **target agent session** (inbox-style), same as other scheduled AINL graphs.

## What it does

1. **Fetch** a stable public profile (`GET https://api.github.com/users/octocat`) as a stand-in lead row.
2. **Enrich** a human-readable `lead_card` (`login — display_name`) and capture `public_repos`.
3. **Score** with a deterministic heuristic (`heuristic_follower_digits` = length of the followers count string — a toy signal, not real lead scoring).
4. **Optional LLM**: if `extra.use_llm` is exactly `"yes"`, calls `llm.COMPLETION` once for a short ICP note (`llm_note`). Omit or set any other value to stay fully offline.

## Enable (cron)

1. Dashboard **Scheduler** → enable **`armaraos-lead-gen-pipeline`** (shipped **disabled** by default).
2. Or call `POST /api/ainl/library/register-curated` after upgrade so the job row exists, then enable it.

Schedule (bundled catalog): **Monday 07:00** — adjust in the Scheduler after enabling if you prefer a different cadence.

## Required config / secrets

| Mode | Needs |
|------|--------|
| Default (`use_llm` not `yes`) | Outbound HTTPS to `api.github.com` only (respect GitHub rate limits). |
| LLM branch | Provider keys resolved for scheduled `ainl` (same mechanism as other graphs: vault / `~/.armaraos/.env` / env — see [scheduled-ainl.md](../../docs/scheduled-ainl.md)). Graph comment references **`llm/openrouter`**; align with your `AINL` / MCP LLM setup. |

## Manual run

```bash
ainl run programs/lead_gen_pipeline/lead_gen_pipeline.ainl --strict --json \
  --frame-json @programs/lead_gen_pipeline/frame.example.json
```

To try the LLM path, set `"use_llm": "yes"` in `extra` inside the frame JSON.

## Expected behavior

- **JSON** containing `pipeline`, `seed_company`, `lead_card`, `phone`, `heuristic_email_len`, `source`, `generated_at`.
- With `use_llm: "yes"`, adds `llm_note` (or the run fails if no LLM adapter / keys).

## Extension notes

- Swap the URL for your CRM or enrichment HTTP API; add hosts to adapter allowlists as needed; keep **query parameters in the URL** per AINL `http.GET` rules.
- Do not put inline `{ ... }` dicts on `R http.*` lines — build bodies via `frame` / `core.MERGE` patterns (see [AI_Native_Lang AGENTS.md](https://github.com/sbhooley/ainativelang/blob/main/AGENTS.md)).
- Tune **`timeout_secs`** on the cron row if you add slower network steps.
