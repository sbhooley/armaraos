# Multi-channel digest (`channel_session_digest`)

Scheduled **every 6 hours** by default (`armaraos-channel-session-digest`). Emits one JSON object combining:

- **`daemon_status`** from `GET /api/health`
- **`active_agents`** — count of rows from `GET /api/agents` (the API returns a **JSON array**; the graph uses `core.LEN`)
- **`channel_adapters_catalog`** / **`channels_configured`** from `GET /api/channels` (`total` and `configured_count`)
- **`workflows_defined`** — `core.LEN` of the JSON array from `GET /api/workflows`

## Where output goes

With **`json_output: true`**, the kernel runs `ainl run --json` and appends the formatted stdout to the **curated job’s target agent session** (system / inbox-style message). It is **not** written to agent KV unless you add that yourself (e.g. `http.PUT` to `/api/memory/...` in a separate graph).

## Enable / tweak

- **Scheduler:** job `armaraos-channel-session-digest` (on by default in the bundled catalog).
- **Timeout:** catalog uses **45s** to allow four loopback `http.GET` calls on localhost.

## Extension notes

- Add per-agent `GET /api/agents/:id/session/digest` only if you pass agent IDs via `frame` / host tooling (avoid unbounded loops in AINL).
- For Slack/email delivery, use **`delivery`** on a custom cron row or a channel integration reading the session feed.
