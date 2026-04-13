# AINL showcases (ArmaraOS `programs/`)

ArmaraOS ships **embedded** AINL graphs from the repo [`programs/`](../programs/) tree. They materialize to:

`~/.armaraos/ainl-library/armaraos-programs/`

Curated **Scheduler** jobs are defined in [`crates/openfang-kernel/src/curated_ainl_cron.json`](../crates/openfang-kernel/src/curated_ainl_cron.json) and register idempotently (upsert by **name**; your **enabled** toggle is preserved on upgrade). See [ootb-ainl.md](ootb-ainl.md) for env overrides and library layout.

## Five operator-facing showcases

| # | Slug | What it demonstrates | Curated job name | Default |
|---|------|----------------------|------------------|---------|
| 1 | `lead_gen_pipeline` | Read-only GitHub profile stand-in + heuristic score; optional `llm.COMPLETION` when `extra.use_llm` is `yes` | `armaraos-lead-gen-pipeline` | Off |
| 2 | `research_pipeline` | GitHub repo search + structured JSON; optional LLM brief | `armaraos-research-pipeline` | Off |
| 3 | `channel_session_digest` | Loopback digest: health, **agent count** (`core.LEN` on `/api/agents`), channel catalog, workflow list | `armaraos-channel-session-digest` | On |
| 4 | `budget_threshold_alert` | Hourly spend vs **80%** of limit with clear branch structure | `armaraos-budget-threshold-alert` | On |
| 5 | `system_health_monitor` | **Combined** local health + agents + `/api/version` + GitHub + PyPI (Option B; does not remove legacy split jobs) | `armaraos-system-health-monitor` | Off |

Each program directory includes a **`README.md`** (and `lead_gen` / `research` include **`frame.example.json`** for LearningFrame `extra` fields).

## Where output goes

Scheduled `ainl run` with **`json_output: true`** formats stdout and the kernel **appends it to the target agent’s session** as a system message. This is **not** the same as the AINL `memory` adapter or automatic `PUT /api/memory/...` unless your graph adds it.

## Validate locally

```bash
ainl validate programs/lead_gen_pipeline/lead_gen_pipeline.ainl --strict
ainl validate programs/research_pipeline/research_pipeline.ainl --strict
ainl validate programs/channel_session_digest/channel_session_digest.ainl --strict
ainl validate programs/budget_threshold_alert/budget_threshold_alert.ainl --strict
ainl validate programs/system_health_monitor/system_health_monitor.ainl --strict
```

## Sample JSON (illustrative)

**Lead-gen (deterministic branch):**

```json
{
  "pipeline": "lead_gen",
  "seed_company": "Acme Robotics",
  "lead_card": "octocat — The Octocat",
  "public_repos": 8,
  "heuristic_follower_digits": 5,
  "source": "api.github.com",
  "generated_at": 1712563200
}
```

**Research (deterministic branch):**

```json
{
  "pipeline": "research",
  "query": "armaraos",
  "total_count": 42,
  "top_repo": "sbhooley/armaraos",
  "top_url": "https://github.com/sbhooley/armaraos",
  "source": "api.github.com",
  "generated_at": 1712563200
}
```

**Multi-channel digest (`digest_version: 2`):**

```json
{
  "digest_version": 2,
  "daemon_status": "ok",
  "active_agents": 3,
  "channel_adapters_catalog": 40,
  "channels_configured": 1,
  "workflows_defined": 0,
  "generated_at": 1712563200
}
```

**Budget threshold:**

```json
{
  "alert": false,
  "threshold_rule": "spent_ge_80pct_of_limit",
  "spent_usd": 1.25,
  "budget_limit_usd": 50,
  "period": "monthly",
  "checked_at": 1712563200
}
```

**System health:**

```json
{
  "check": "system_health",
  "overall": "ok",
  "daemon_status": "ok",
  "agents": 3,
  "update_available": false,
  "armaraos_running": "0.7.3",
  "armaraos_upstream_tag": "v0.7.3",
  "ainl_pypi_version": "1.5.0",
  "checked_at": 1712563200
}
```

Exact values depend on your daemon, catalog size, and live API responses.

## Load testing

After a harness run, smoke the embedded graphs against a live daemon (loopback + optional GitHub) — see [load-testing.md](load-testing.md#ainl-showcases-after-load-test).
