# System health monitor (`system_health_monitor`)

**Option B showcase:** one graph that blends **local** checks with **upstream** release signals. It does **not** replace `agent_health_monitor` or `new_version_checker`; those jobs and schedules stay as-is for backward compatibility.

## What it does

1. `GET /api/health` + `GET /api/agents` (agent count via `core.LEN` on the agents array).
2. `GET /api/version` for the running daemon build.
3. `GET https://api.github.com/repos/sbhooley/armaraos/releases/latest` for latest tag.
4. `GET https://pypi.org/pypi/ainativelang/json` for published AINL package version.

Output includes `overall` (`ok` | `degraded`), `update_available` when the GitHub tag differs from `armaraos_running`, and version fields for dashboards.

## Curated job

- **Name:** `armaraos-system-health-monitor`
- **Default:** **disabled** (opt-in in Scheduler).
- **Suggested schedule:** every 2 hours at `:15` (see `curated_ainl_cron.json`).

## Where output goes

JSON stdout → **target agent session** (cron capture), same as other embedded programs.

## Extension notes

- Add `GET /api/metrics` for scrape-style probes once you confirm parsing needs in `core` (keep responses small).
- For remediation (restart, notify), use a **separate** workflow or tool-backed agent; keep this graph read-only and fast.
