# Budget threshold alert (`budget_threshold_alert`)

Runs **hourly** (`armaraos-budget-threshold-alert`). Reads `GET /api/budget` and:

- If **`budget_limit_usd` is 0** — emits `{ alert: false, reason: "no_limit_set", ... }` (no threshold math).
- If **limit is greater than 0** — computes `at_risk = core.GTE (spent * 100) (limit * 80)` and emits a **single** JSON object with `alert` set from that boolean (same 80% rule as before, without nested `if` blocks that break strict compilation).

## Where output goes

Structured JSON is appended to the **target agent session** for the job (same as other curated `ainl run` graphs with `json_output: true`).

## Enable / secrets

- **Loopback only** — requires the daemon on `http://127.0.0.1:4200` (default API bind).
- No extra secrets for the graph itself; budget data comes from the local API.

## Extension notes

- Add a second threshold (e.g. 95%) by duplicating the compare block with different multipliers, or emit both booleans in one `out` using extra `core` assignments (keep graphs small to respect step limits).
- Wire **PagerDuty / Slack** by adding a follow-on job or channel automation that reads the session JSON.
