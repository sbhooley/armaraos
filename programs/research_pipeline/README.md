# Research pipeline (AINL showcase)

**Keyword → evidence → structured report** pattern using the **GitHub Search API** (read-only, unauthenticated, rate-limited). Optional **one-shot** `llm.COMPLETION` when `extra.use_llm` is `"yes"`. JSON output is suitable for cron + session capture (same mechanics as other `armaraos-programs` graphs).

## What it does

1. Reads **`extra.research_query`** (must be URL-safe: letters, numbers, `+`, `-`, no raw spaces).
2. **`http.GET`** `https://api.github.com/search/repositories?q=<query>&per_page=1`.
3. Emits `total_count`, `top_repo`, `top_url`, timestamps.
4. If **`extra.use_llm`** is `"yes"`, adds **`llm_brief`** via the configured LLM adapter.

## Enable (cron)

Enable **`armaraos-research-pipeline`** in the Scheduler (bundled **disabled** by default). Register curated jobs after upgrade if needed (`POST /api/ainl/library/register-curated`).

Default catalog schedule: **Tuesday 06:30** — change after enable if you want a different window.

## Required config / secrets

| Mode | Needs |
|------|--------|
| Default | Outbound HTTPS to `api.github.com`. Respect [GitHub rate limits](https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api); fine for occasional cron, not a high-frequency scraper. |
| LLM branch | Provider keys for scheduled `ainl` (see [scheduled-ainl.md](../../docs/scheduled-ainl.md)). |

## Manual run

```bash
ainl run programs/research_pipeline/research_pipeline.ainl --strict --json \
  --frame-json @programs/research_pipeline/frame.example.json
```

Use a narrow query (e.g. `"ainl+language:Python"`) by editing `extra.research_query`.

## Expected behavior

JSON with `pipeline: "research"`, `query`, `total_count`, `top_repo`, `top_url`, `source`, `generated_at`, and optionally `llm_brief`.

If the search returns **zero** items, the graph may fail at `core.GET items "0"` — pick a query that returns at least one repository for demos.

## Extension notes

- Add a second `http.GET` for docs or releases, then **`core.MERGE`** dicts (variables only — no inline object literals on `R` lines).
- For production, add auth (token via env / frame-safe indirection) and lower cron frequency.
- Prefer **`per_page` small** to keep adapter calls and payload size predictable under runtime limits.
