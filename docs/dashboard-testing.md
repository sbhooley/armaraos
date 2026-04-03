# Dashboard testing notes

## Automated API smoke (daemon running)

With the daemon running, pass the **Dashboard** base URL from `openfang start` (default is often `http://127.0.0.1:4200`, but `config.toml` may use another port such as `50051`):

```bash
./scripts/verify-dashboard-smoke.sh http://127.0.0.1:4200
# or, if the CLI printed a different dashboard URL:
./scripts/verify-dashboard-smoke.sh http://127.0.0.1:50051
```

This hits `/api/health`, `/api/status`, `/api/schedules`, `POST /api/support/diagnostics` (writes a zip under `~/.armaraos/support/` on success), and a sample `POST /api/agents` to verify auth/error JSON when a key is configured.

## Overview “Getting Started” checklist

- **Core (4):** provider, agent, channel, scheduled job — progress bar reflects these first.
- **Optional (2):** first chat message + browse/install a skill — after core is complete, the card stays visible with title **Getting Started (optional next)** until both optional rows are done or the user dismisses the card.
- **Dismiss** stores `of-checklist-dismissed` in `localStorage` and hides the whole checklist.

## Manual browser checklist (copy/paste & bundles)

Run the daemon, open the **Dashboard** URL from `openfang start` (default is often `http://127.0.0.1:50051` — see `api_listen` in `config.toml`).

1. **Disconnected sidebar** — Stop the daemon (or point the browser at a wrong port). Confirm the sidebar shows **Copy debug info** and **Generate + copy bundle**. Click **Copy debug info**; paste into a scratch buffer and check for **URL**, **Where**, **Request ID**, **Error**, **Hint**, **Detail**, **Time**. Click **Generate + copy bundle**; paste and confirm **Bundle:** path plus the same context lines (when the API is reachable from loopback, a `.zip` is created under `~/.armaraos/support/` or your `ARMARAOS_HOME`).
2. **Overview** — From a fresh profile or after clearing `localStorage` keys `of-checklist-dismissed`, `of-first-msg`, `of-skill-browsed` (optional), walk the checklist: complete **core** steps, then confirm the title switches to **Getting Started (optional next)** and the progress bar tracks **optional** tasks. Finish optional rows or use **Dismiss**.
3. **Settings → System Info → Support** — Use **Generate diagnostics bundle** (same redacted `.zip` as the API). Confirm the UI mentions `.zip` and the generated path matches a file on disk.

## Kernel SSE (`GET /api/events/stream`)

- **Smoke:** Open the dashboard, confirm the sidebar **SSE** badge appears when the stream connects (kernel running, same origin / loopback as usual).
- **API:** `cargo test -p openfang-api --test api_integration_test test_kernel_events_stream_sse_smoke` and `cargo test -p openfang-api --test sse_stream_auth` cover HTTP behavior (including loopback vs remote auth).

## Overview refresh

- On **Overview**, lifecycle/system kernel events trigger a **debounced** refresh (~400ms) via `armaraos-kernel-event`. The page also shows a **Last kernel event** line when `kernelEvents.last` is set.
- **Page leave:** The overview component registers `@page-leave.window="stopAutoRefresh()"` so timers and kernel listeners are cleared when navigating away. If you add **Playwright** (or similar) later, assert that after switching to another hash/route, `setInterval`-driven refresh is not still firing (e.g. spy on `/api/usage` or equivalent after leaving Overview).
