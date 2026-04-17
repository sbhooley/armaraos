# Dashboard testing notes

## Automated API smoke (daemon running)

With the daemon running, pass the **Dashboard** base URL from `openfang start` (default is often `http://127.0.0.1:4200`, but `config.toml` may use another port such as `50051`):

```bash
./scripts/verify-dashboard-smoke.sh http://127.0.0.1:4200
# or, if the CLI printed a different dashboard URL:
./scripts/verify-dashboard-smoke.sh http://127.0.0.1:50051
```

This hits `/api/health`, `/api/status`, `/api/schedules`, **`GET /api/version/github-latest`**, **`GET /api/logs/daemon/recent?lines=5`** (may return empty `lines` until `logs/daemon.log` exists), `POST /api/support/diagnostics` (writes a zip under `~/.armaraos/support/` on success), **`GET /api/support/diagnostics/download`** for that zip’s `bundle_filename`, **`GET /api/armaraos-home/download`** for the same file under `support/…`, a sample `POST /api/agents` to verify auth/error JSON when a key is configured, and (when agents exist) `GET /api/agents/:id/session/digest`.

### CI: temp daemon + same smoke script

GitHub Actions (Linux) runs an optional job **`dashboard-smoke`** that builds **`openfang`** release, uses a throwaway **`ARMARAOS_HOME`**, runs **`openfang init --quick`**, starts the daemon, then invokes **`./scripts/verify-dashboard-smoke.sh`**. Wrapper: **`scripts/ci-dashboard-smoke.sh`** (set **`OPENFANG_BIN`** if the binary lives somewhere other than **`target/release/openfang`**). The job is **`continue-on-error: true`** so a flaky boot does not block merges.

### Rust: HTTP integration tests use the production router

**`cargo test -p openfang-api`** integration tests (**`tests/api_integration_test.rs`**, **`tests/load_test.rs`**, and lifecycle tests in **`tests/daemon_lifecycle_test.rs`**) bind a random port and serve **`openfang_api::server::build_router`** — the same Axum route table, auth middleware, rate limiting, and layers as **`run_daemon`**, not a hand-maintained parallel router. Add new endpoint coverage there when you change **`crates/openfang-api/src/server.rs`**.

## PUT full manifest + on-disk `agent.toml` (optional API QA)

Verifies **`PUT /api/agents/{id}/update`** applies **`AgentManifest`** TOML to the running kernel and syncs **`agents/<name>/agent.toml`** under the configured home (see [api-reference.md](api-reference.md)). **Auth:** when `api_key` is set, send **`Authorization: Bearer <key>`** (same as other write routes).

1. Set **`BASE`** to your dashboard origin (e.g. `http://127.0.0.1:4200`). Resolve an agent id: **`GET $BASE/api/agents`** and pick **`id`** plus the agent’s **`name`** (the manifest’s **`name =`** must match that string).
2. **`PUT $BASE/api/agents/<id>/update`** with JSON **`{"manifest_toml": "..."}`** — use a full valid TOML body; change a harmless field (e.g. **`description`**) so you can spot it on disk.
3. Expect **`200`** and JSON **`status`: `"ok"`**, **`name`**, and a non-empty **`note`** (session memory clear; autonomous loops reload in-process without daemon restart).
4. Optional: **`GET $BASE/api/audit/recent?n=30`** — confirm an **`AgentManifestUpdate`** entry whose **`detail`** mentions **`PUT agent manifest update`** and the agent name.
5. On disk, open **`$ARMARAOS_HOME/agents/<name>/agent.toml`** (default **`~/.armaraos`**) and confirm the field matches the PUT body.

**Automated regression:** `cargo test -p openfang-api --test api_integration_test test_put_agent_update`.

## Get started page (hash `#overview`) — setup checklist

**UI labels:** The sidebar shows **Get started** (section above **Chat**); the page title is **Get started**. The router still uses the internal page id `overview` and hash `#overview`.

**Checklist card**

- **Core (3):** configure an LLM provider, create your first agent, create a scheduled job. Section title **Getting Started**; progress bar reflects these three until all are done.
- **Optional block:** messaging channel (completable), plus two **perpetual shortcuts** — send your first message (→ Chat/agents) and browse or install a skill (→ Skills). The shortcuts always show **○** and **Go**; they are **never** marked complete so users can jump back anytime. After core completes, the card title switches to **Optional setup**; the bar tracks only **channel** (0–100% for one row). Subtitle text reflects **4** trackable rows total (core three + channel).
- **Dismiss** stores `of-checklist-dismissed` in `localStorage` and hides the whole checklist. After core + channel are satisfied, the card hides automatically unless dismissed earlier.

**Removed onboarding keys (do not use in tests anymore):** `of-first-msg` and `of-skill-browsed` are no longer written or read. There is no `armaraos-onboarding-local` refresh for the checklist.

**Quick actions, layout, Setup Wizard visibility, and CSS:** See **[dashboard-overview-ui.md](dashboard-overview-ui.md)** (section order, `!loadError && !loading` visibility, **seven-tile** skeleton + **App Store** + **Daemon & runtime** actions, `openfang-onboarded` / `navigateOverview`, hash targets).

**Settings / Runtime layout:** See **[dashboard-settings-runtime-ui.md](dashboard-settings-runtime-ui.md)** (headers, tab bar, stat grid, panels, **daemon lifecycle** buttons).

## Manual browser checklist (copy/paste & bundles)

Run the daemon, open the **Dashboard** URL from `openfang start` (default is often `http://127.0.0.1:50051` — see `api_listen` in `config.toml`).

1. **Disconnected sidebar** — Stop the daemon (or point the browser at a wrong port). Confirm the sidebar shows **Copy debug info** and **Generate + copy bundle**. Click **Copy debug info**; paste into a scratch buffer and check for **URL**, **Where**, **Request ID**, **Error**, **Hint**, **Detail**, **Time**. Click **Generate + copy bundle**; paste and confirm **Bundle:** path plus the same context lines (when the API is reachable from loopback, a `.zip` is created under `~/.armaraos/support/` or your `ARMARAOS_HOME`).
2. **Get started** — From a fresh profile or after clearing `localStorage` key `of-checklist-dismissed`, open **Get started** (`#overview`). Walk **core** rows (3), then confirm the card title becomes **Optional setup**, the bar tracks **channel** only, and **chat** / **skill** rows stay open with **Go** after you use them. Connect a channel or use **Dismiss** to clear the card.
3. **Get started → Quick actions** — After load completes, confirm the **Quick actions** card appears near the top (under the **Live** strip when kernel events exist). Click each action and verify: **New Agent** → `#agents`, **Browse Skills** → `#skills`, **App Store** → `#ainl-library`, **Add Channel** → `#channels`, **Create Workflow** → `#workflows`, **Settings** → `#settings`, **Daemon & runtime** → `#runtime`. During initial load, a **seven-cell** skeleton should appear in that slot, then swap to buttons without a large layout jump.

4. **Get started → Setup Wizard visibility** — With `localStorage` **`openfang-onboarded`** set to **`true`** (e.g. after finishing the wizard, or set manually in devtools), open **Get started**. Expect **Setup Wizard** hidden in the page header and in the setup checklist card; **Run setup again** visible in the header. Click **Run setup again** → **Setup Wizard** appears (header + checklist). Click **Setup Wizard** → `#wizard`. Alternatively, while still on Get started, click the sidebar **Get started** item again → same reveal. Clear `openfang-onboarded` or use a fresh profile → **Setup Wizard** should show by default without **Run setup again**. With onboarded + collapsed CTA, completing the wizard again (or a silent refresh after the flag flips) should hide the primary wizard button again.

5. **Setup Wizard (`#wizard`) — end-to-end** — Open `#wizard` (or use the steps in item 4). After **Get Started** on step 1, on **step 2 (Provider)** with an already-configured provider: wait for the connection test to finish; **Next** should become enabled (label **Next**), even if the test shows a warning for a free model or transient API error. On **step 3**, confirm template cards reflect the configured provider/default model where applicable; enter an agent name, **Create Agent**, and confirm the toast and sidebar use that **name** (not `unnamed`). Complete **Try It**, optional **Channel**, and **Done**; confirm the summary lists provider and agent names sensibly. **Dev note:** `wizard.js` is embedded in the binary — rebuild the daemon after editing static assets so the browser picks up changes. Full contract: [dashboard-setup-wizard.md](dashboard-setup-wizard.md).

6. **App Store → on-disk section title** — Open **App Store** (`#ainl-library`). The collapsible catalog section should read **AI Native Lang Programs Available** (not the old “on disk” wording).

7. **Settings and Runtime pages — layout polish** — Open **Settings**: elevated header with subtitle; tab bar is a rounded card with accent top stripe and pill-style active tab. **Below the tab bar**, confirm the **at-a-glance** line shows **Daemon**, **Config schema** (`N (binary M)`), **API**, **Log**, and **Home**. Open **Settings → System** and confirm the **Config schema** stat tile matches that line. Open **Runtime**: header subtitle; stat tiles wrap in a responsive grid on a narrow window; **Config schema** appears in the stat grid; **System** and **Providers** panels use uppercase section titles and readable tables. **Runtime** footer: **Refresh**, **Reload config**, **Reload channels**, **Reload integrations**, **Shut down** (see [Daemon lifecycle & GitHub version check](#daemon-lifecycle--github-version-check) below).

7b. **Ultra Cost-Efficient Mode (Budget + Chat)** — Open **Settings → Budget** and confirm the **Ultra Cost-Efficient Mode** card and dropdown (Off / Balanced / Aggressive) with helper copy. Change the mode, reload **`GET /api/config`** (or re-open Settings) and confirm **`efficient_mode`** persisted. Open **Chat** with an agent: confirm the **⚡ eco** header pill cycles modes and that after a long user message (≥80 estimated tokens with compression enabled) the response meta can show **`⚡ eco ↓X%`** and the **diff** control opens the Eco Diff modal with **Original** vs **Compressed**. See [prompt-compression-efficient-mode.md](prompt-compression-efficient-mode.md).

   **Per-agent persistence:** With two agents, set **⚡ eco** to different modes on each, navigate to another dashboard page and back, and reload the app — each chat should restore its own mode. Confirm **`GET /api/ui-prefs`** returns **`agent_eco_modes`** with both ids (and that **`PUT /api/ui-prefs`** is sent when cycling the pill). The global **`efficient_mode`** in **`config.toml`** remains the server default for new sessions; chat still calls **`POST /api/config/set`** so the active compression mode matches the open agent.

8. **Settings → System Info → Support** — Use **Generate diagnostics bundle** (same redacted `.zip` as the API). Confirm the UI mentions `.zip`, **README.txt** / **diagnostics_snapshot.json** in the Support copy, and the generated path matches a file on disk. Unzip once and confirm **`README.txt`** and **`diagnostics_snapshot.json`** exist and JSON includes `config_schema_version` / `config_schema_version_binary` under `daemon`. On **desktop**, confirm a copy lands in **Downloads** (or use the fallback download if copy fails). With `api_key` set, confirm the browser/WebView still completes save (loopback GET download + `token` query).

9. **Home folder → `support/`** — Open **Home folder** in the nav, go to **`support`**, find a diagnostics `.zip`. Use row **Download** (green) or **View** then **Download** in the modal header. Large zips may show a preview error; **Download** must still save the full file. On desktop, row **Download** uses Tauri **`copy_home_file_to_downloads`** (`relativePath`).

## Daemon lifecycle & GitHub version check

**GitHub compare (no browser → GitHub):** On **Settings → System Info → Daemon / API runtime** and **Monitor → Runtime**, **Check daemon vs GitHub** / **Check vs GitHub** must succeed when the daemon is up (uses **`GET /api/version/github-latest`** server-side). Smoke from shell:

```bash
curl -sS "http://127.0.0.1:4200/api/version/github-latest" | head -c 400
```

**Hot reload (dev / after editing `config.toml` on disk):** From **Settings** (same **Daemon / API** card) or **Runtime** footer:

1. **Reload config** — confirm modal → success or “no changes” / “restart required” toast; **Settings** summary fields refresh when possible.
2. **Reload channels** — confirm → success toast with started channel names (or empty list); brief bridge restart is OK.
3. **Reload integrations** — confirm → success toast.

**Shut down (last):** Use **Shut down daemon** / **Shut down** only when you intend to stop the process. Confirm the modal; expect disconnect or “connection closed” style feedback. Restart with the desktop app, **`openfang start`**, or your supervisor.

**Internal automation agents (sidebar):** Agents whose names match internal automation probes (**`allowlist-probe`**, **`offline-cron`**, **`allow-ir-off`**) are grouped for automation and **hidden from the main agent sidebar list**; chat behavior for those agents is unchanged aside from capability hints. See `js/app.js` (`isInternalAutomationProbeChatAgentName`) and `js/pages/agents.js`.

## Support diagnostics bundle (create, download, desktop)

**Create (API):** `POST /api/support/diagnostics` with body `{}` returns JSON including `bundle_path`, `bundle_filename`, and `relative_path`. From **loopback**, this POST does not require Bearer auth (so local UI works with `api_key` configured).

**Bundle contents (triage):** The zip includes **`README.txt`**, **`diagnostics_snapshot.json`** (start here for support: schema versions, paths, runtime, SQLite memory `user_version` vs expected), expanded **`meta.json`**, `config.toml`, redacted `secrets.env`, `audit.json`, `data/openfang.db*` when present, and `home/logs/…` (see [api-reference.md](api-reference.md#post-apisupportdiagnostics) and [troubleshooting.md](troubleshooting.md#dashboard-support-bundle-redacted-zip)).

**Download (API):** `GET /api/support/diagnostics/download?name=<bundle_filename>` streams the zip (`Content-Disposition: attachment`). From **loopback**, this GET is also allowed without Bearer (same rationale as POST). Remote clients must authenticate. The dashboard client may append `&token=<api_key>` and sends `Authorization` when a key is stored.

**Desktop app:** After create, the shell invokes **`copy_diagnostics_to_downloads`** with **`{ bundlePath: "<absolute path from JSON>" }`** (Tauri camelCase, single required key). If copy fails, the UI may fall back to the HTTP download above. Rebuild the desktop app after changes to this command or permissions.

**Copy any home file (desktop):** From **Home folder**, row **Download** uses **`copy_home_file_to_downloads`** with **`{ relativePath: "support/armaraos-diagnostics-….zip" }`** (path relative to ArmaraOS home). Allowed in `crates/openfang-desktop/permissions/dashboard.toml` and generated ACL manifests.

## Home folder browser — preview vs download

**List / preview:** `GET /api/armaraos-home/list` and `GET /api/armaraos-home/read` — **read** is capped at **512 KiB** and returns UTF-8 text or base64 for small binary previews.

**Full file:** `GET /api/armaraos-home/download?path=<relative path>` streams up to **256 MiB** with attachment headers. Loopback may call without Bearer (embedded dashboard). Use this when diagnostics zips (or other large files) fail preview with “file too large”.

**UI:** The **Home folder** table has **View** (preview) and **Download** (full file). **View** opens a **near full-viewport** modal (desktop-sized window) so long text files are easier to read; the modal still includes **Download** in the header whenever a path is set, plus **Download file** in the error panel when preview fails — large `.zip` under `support/` should still be saveable.

## Kernel SSE (`GET /api/events/stream`)

- **Smoke:** Open the dashboard, confirm the sidebar **SSE** badge appears when the stream connects (kernel running, same origin / loopback as usual). The **[Notification center (bell)](#notification-center-bell)** also consumes the same stream for persistent rows (crashes, quota, health, cron failures) alongside approvals + budget polling.
- **API:** `cargo test -p openfang-api --test api_integration_test test_kernel_events_stream_sse_smoke` and `cargo test -p openfang-api --test sse_stream_auth` cover HTTP behavior (including loopback vs remote auth).

## Orchestration traces (`#orchestration-traces`)

**Navigation:** Sidebar **Agents → Orchestration** (below **Graph Memory**), or open **`http://127.0.0.1:4200/#orchestration-traces`** (adjust host/port if needed).

**Smoke (after a workflow or multi-agent run produced a trace):**

1. **List** — Page loads without console errors; recent traces appear (substring filter narrows rows).
2. **Detail** — Open a trace row: **delegation graph** (indented tree; agent id copy affordance if present), **Gantt-style** timeline, **token in/out heatmap**, **quota** summary when available.
3. **Filters** — **Event type** filter (comma-separated) narrows Gantt + raw events JSON; date / id filters behave as described in **`docs/orchestration-guide.md`**.
4. **JSON** — Expand/collapse blocks for **events**, **tree**, and **cost** payloads; JSON should match **`GET /api/orchestration/traces/{id}`** (+ `/tree`, `/cost`) for the same id.
5. **CLI parity** — With daemon running: `openfang orchestration list` shows the same traces as the list view; `openfang orchestration trace <TRACE_ID> --json` matches API shape.

**Automated API:** Run **`cargo test -p openfang-api --test api_integration_test`** (orchestration routes are exercised alongside other HTTP surface tests in that harness).

**Dev note:** Page logic lives in **`crates/openfang-api/static/js/pages/orchestration-traces.js`** with **`.orch-*`** styles in **`static/css/layout.css`**. Rebuild / restart the daemon after editing embedded assets.

**Further reading:** **`docs/orchestration-guide.md`**, **`docs/orchestration-walkthrough.md`**, **`docs/workflows.md`** (Orchestration and traces).

## Notification center (bell)

Persistent queue (not toasts): fixed **bell** top-right; **badge** = number of rows in the panel. Implemented in `static/js/app.js` (`Alpine.store('notifyCenter')`), `static/index_body.html`, `static/css/layout.css`, and **`static/js/pages/command-palette.js`** (palette action **Notifications**). Rebuild the daemon after editing embedded static assets.

**Data sources:**

| Row type | Source |
|----------|--------|
| Pending approvals | Poll **`GET /api/approvals`**; immediate refresh when kernel SSE emits **`ApprovalPending`** |
| Budget threshold | Poll **`GET /api/budget`** (compare spend vs limit × **`alert_threshold`**) |
| Kernel events (crash, quota, health, cron, workflow finished, assistant reply, …) | Same SSE client as sidebar: **`GET /api/events/stream`** → `notifyCenter.ingestKernelEvent` |
| Daemon vs GitHub | Every **6 hours**: **`GET /api/version`** + **`GET /api/version/github-latest`** → `syncAppReleaseUpdate` (semver compare); dismiss id **`release-<tag>`** |
| AINL vs PyPI | Same **6h** poll: **`GET /api/ainl/runtime-version`** → `syncAinlPypiUpdate` when **`pypi_latest_version`** is newer than host **`pip_version`**; dismiss id **`ainl-pypi-<version>`**; **`localStorage`** **`armaraos-notify-dismissed-ainl-pypi`** |

**Layout (no overlap with header actions):** `:root` defines **`--notify-bell-reserve`** (default **56px**). **`.main-content`** uses **`padding-right: calc(var(--notify-bell-reserve) + env(safe-area-inset-right))`** so in-flow controls (page headers, chat toolbar) stay left of the fixed bell; the bell uses **`right: calc(14px + env(safe-area-inset-right))`**. **Focus mode** clears that extra padding (bell hidden with other chrome). On narrow viewports (**`max-width: 480px`**), the right gutter is preserved so the bell does not cover content.

**A11y & UX:** With the panel **closed**, new rows update a visually hidden **`aria-live="polite"`** region (“New notification: …”). With the panel **open**, **Tab** / **Shift+Tab** cycle focus within the panel (focus trap); closing restores focus to the element that had focus before open (usually the bell). Command palette (**Cmd/Ctrl+K**) includes **Notifications** → opens the panel.

**Persistence:** Dismissing a **kernel** row (id `k-<event id>`) stores that id in **`localStorage`** key **`armaraos-notify-dismissed-kernel`** (capped list) so a full page reload does not show the same SSE event again. Synthetic rows **`approval-pending`** and **`budget-alert`** are not stored there. **GitHub release** rows use **`armaraos-notify-dismissed-release-tag`** (last dismissed tag). **AINL PyPI** rows use **`armaraos-notify-dismissed-ainl-pypi`** (last dismissed PyPI version string).

**Agent reply noise:** Settings → **System** → **Notification center** — **`notify_chat_replies`** in **`ui-prefs.json`**: **`all`** (always bell rows for **`AgentAssistantReply`**), **`hidden`** (skip while that agent’s inline chat is the focused, visible surface), **`off`**. Local mirror: **`armaraos-notify-chat-replies`**.

**Automated checks:** `./scripts/verify-dashboard-smoke.sh` curls **`GET /`**, **`GET /api/budget`**, **`GET /api/approvals`**, and **`GET /api/ainl/runtime-version`** (JSON key smoke). Integration tests (substring filter matches both): `cargo test -p openfang-api --test api_integration_test json_shape`. Kernel SSE smoke: `cargo test -p openfang-api --test api_integration_test test_kernel_events_stream_sse_smoke`.

**Manual smoke (daemon + dashboard in browser):**

1. **Bell + panel** — Confirm the bell appears when connected. Click it: panel opens; **badge** matches row count. **Esc** and clicking the **dimmed backdrop** close the panel. **Tab** cycles controls inside the panel while open. **Open** on a row navigates via hash and closes the panel.
2. **Approvals** — With at least one **pending** execution approval (`GET /api/approvals`), expect a **Pending approval(s)** row within a few seconds (poll) or immediately after the kernel emits **`ApprovalPending`** on `GET /api/events/stream` (same SSE path as sidebar **SSE**). **Dismiss** hides that row until the set of pending request ids changes (new approval or all resolved). **Clear all** removes every row and resets snooze/debounce state used by the center (it does **not** clear kernel-dismissed ids in `localStorage`).
3. **Budget** — When a configured limit exists and spend fraction meets or exceeds **`alert_threshold`** from `GET /api/budget` (default **0.85** in the UI when unset), expect a **Budget:** row pointing at **Settings**. **Dismiss** on that row suppresses re-adding the synthetic budget row for **one hour** (or use **Clear all**).
4. **Kernel-driven rows** — From live SSE, confirm new rows can appear for **agent crashed** (Lifecycle **Crashed**), **Quota enforced** / **Quota warning**, **Health check failed** (debounced per agent like toasts, ~90s), **Cron job** finished/failed (AINL vs agent-turn titles when **`action_kind`** is set), **workflow finished**, and **agent replied** (unless **`notify_chat_replies`** is **off** / **hidden** while viewing that chat). Each row should be individually **Dismiss**-able; after dismiss + reload, the same event id should not reappear (kernel rows). **Release** / **AINL PyPI** dismissals persist via the keys above.
5. **Focus mode** — Toggle focus mode (sidebar / chat UX). The bell + panel should **hide** with other chrome (same behavior as the mobile menu button).
6. **Command palette** — Open the palette, search **Notifications**, confirm it opens the notification panel.
7. **Layout** — On a page with a dense top-right toolbar (e.g. **Agents** chat), confirm header actions sit **left** of the bell strip (no overlap); rotate a notched device or use browser safe-area emulation if available.

## Logs page (Live, Daemon, Audit Trail)

The **Logs** page has three tabs:

| Tab | Source | Notes |
|-----|--------|--------|
| **Live** | Merkle **audit** trail | SSE: `GET /api/logs/stream` with optional `level` (`info` / `warn` / `error`) and `filter` (substring). The UI reconnects when filters change. Polling fallback uses `GET /api/audit/recent`. |
| **Daemon** | **`logs/daemon.log`** (CLI `openfang start` / `gateway start`), else **`tui.log`** | `GET /api/logs/daemon/recent?lines=&level=&filter=` and SSE `GET /api/logs/daemon/stream` (same query shape; `level` also allows `trace` / `debug`). **Config log level** is loaded from `GET /api/status` and saved with `POST /api/config/set` (`path`: `log_level`); toast reminds you to **restart the daemon** for tracing to pick it up. |
| **Audit Trail** | Same audit store as Live | Hash chain UI + `GET /api/audit/recent` / `GET /api/audit/verify`. |

**Auth:** With a non-empty `api_key`, SSE endpoints accept **`?token=`** (or `Authorization: Bearer`) for non-loopback clients. From **loopback** only, `/api/logs/stream`, `/api/logs/daemon/stream`, and `/api/events/stream` are allowed without credentials (embedded dashboard / WebView).

**Manual smoke (daemon running):**

```bash
curl -sS "http://127.0.0.1:4200/api/logs/daemon/recent?lines=50"
curl -sS -N "http://127.0.0.1:4200/api/logs/stream?level=info" | head -n 5
```

**Tests:** `cargo test -p openfang-api --test sse_stream_auth` includes **`/api/logs/daemon/stream`** loopback vs non-loopback behavior.

## Get started page refresh

- On the **Get started** page (`overview` route), lifecycle/system kernel events trigger a **debounced** refresh (~400ms) via `armaraos-kernel-event`. When `kernelEvents.last` is set, the page shows a compact **Live** strip (last event summary + **Timeline**).
- **MCP readiness (panel grid):** After load, when **`GET /api/mcp/servers`** returns `readiness.checks`, the **MCP readiness** card lists one badge per check (e.g. **Calendar**) with green = ready / amber = not ready. Data comes from `overview.js` (`mcpReadiness`, getter `mcpReadinessChecks`). API details: [mcp-a2a.md](mcp-a2a.md) and [api-reference.md](api-reference.md#get-apimcpservers). CLI: `openfang doctor` prints the same checks; JSON checks use `daemon_mcp_readiness_<id>` (legacy `daemon_mcp_calendar` retained while `calendar_readiness` is aliased).
- **Skills → MCP:** Open **#skills** → **MCP Servers**. Confirm the primary **Add custom MCP server** form calls **`POST /api/integrations/custom/validate`** and **`POST /api/integrations/custom/add`**, then **`POST /api/integrations/:id/reconnect`** as needed. Expand **Preset examples** and confirm presets load (`GET /api/integrations/mcp-presets`), fields come from **`GET /api/integrations/available`**, and **Validate** / **Install** / **Reconnect** call **`POST /api/integrations/validate`**, **`POST /api/integrations/add`** (with `env`/`config`), and **`POST /api/integrations/:id/reconnect`**. After install, **`GET /api/mcp/servers`** should list the server under **`configured`** and (when healthy) under **`connected`**.
- **Usage / cost hero:** Token and cost figures load from **`GET /api/usage/summary`** (SQLite-backed totals), not ephemeral scheduler-only counters — values should survive **daemon restarts** and desktop upgrades as long as the data directory is preserved.
- **Analytics parity:** The **Analytics** page summary tab uses the same **`/api/usage/summary`** store; **By agent** uses **`GET /api/usage`**, which now prefers the same persistent metering totals (see **`source`** per row in [api-reference.md](api-reference.md#get-apiusage)).
- **Page leave:** The overview component registers `@page-leave.window="stopAutoRefresh()"` so timers and kernel listeners are cleared when navigating away. If you add **Playwright** (or similar) later, assert that after switching to another hash/route, `setInterval`-driven refresh is not still firing (e.g. spy on `/api/usage/summary` or equivalent after leaving the Get started page).

## Chat unread badges + session digest

**Behavior (dashboard static app):**

- **Badges** appear on **All Agents** (sidebar nav count), the **Chat** page title (total), **Quick open** agent rows, and **agent picker** cards when the UI detects new **assistant** activity for an agent while that conversation is not the focused, visible inline chat.
- **Sources:** (1) WebSocket frames `response` / `canvas` (broadcast as `armaraos-agent-ws`), (2) kernel SSE `Message` events with `role: agent` and an `Agent` target (e.g. inter-agent), (3) **digest polling** every ~24s using `GET /api/agents/{id}/session/digest` so unread still updates if the WS is disconnected or another process appended to the session.
- **Baseline:** The client keeps a per-agent `assistant_message_count` baseline from the digest to avoid double-counting when combined with WS/SSE; opening a chat **primes** the baseline and clears that agent’s unread.
- **Navigate away from `#agents`:** The socket may stay open; `OpenFangAPI.wsClearUiCallbacks()` drops Alpine chat handlers so destroyed components are not called. Returning to the same agent reattaches handlers and may **reuse** the connection.
- **Session switch / new session:** Chat code calls `wsDisconnect()` before reconnect so the server session binding stays correct.

## Agents page → Agent detail modal (gear icon)

The **Info / Files / Config** modal is owned by the **`agentsPage`** Alpine scope (not the inline **`chatPage`** scope), so one template covers **both** the agent picker and an open inline chat.

**Manual checks:**

1. From **Chat** with **no** agent selected (picker / grid): open an agent’s detail (row actions or card), confirm the modal opens; close with **×** or overlay click.
2. Open an agent’s **inline chat** (conversation view). Click the **gear** (agent settings) in the header — the **same** modal must open (tabs **Info**, **Files**, **Config**).
3. While **in** inline chat, confirm the modal’s primary **Chat** action is **hidden** (you are already chatting). From the picker-only flow, **Chat** should still be visible and navigate into chat.
4. Optional: `curl -s http://127.0.0.1:4200/api/ui-prefs` after pinning — expect `pinned_agents` in JSON (see [api-reference.md](api-reference.md#ui-preferences-endpoints)). After toggling **⚡ eco** in chat, the same file may include **`agent_eco_modes`** (map of agent id → mode string).

## Agents page → Config tab (identity, prompt, tool filters)

**API contract:** `GET /api/agents` and `GET /api/agents/{id}` return **`system_prompt`**, full **`identity`** (`archetype`, `vibe`, …), **`manifest_toml`** (detail route — canonical TOML for the full manifest editor), and (on the detail route) **`tool_allowlist`** / **`tool_blocklist`**. The dashboard loads detail after open and reapplies the form so edits are not blank. **`PATCH /api/agents/{id}/config`** ignores empty `system_prompt` / `description` and merges identity so stray `""` values do not wipe stored data.

**Manual checks (daemon + browser):**

1. Spawn or pick an agent; set **Archetype**, **Vibe**, **System prompt**, and add tools to **Allowlist** / **Blocklist**; save — toast should say **partial — session preserved**.
2. Close the detail modal and reopen **Config** — fields and lists should match what you saved.
3. **Default allowlist merge:** When the allowlist is **non-empty**, the kernel merges core file/network/channel tools plus **AINL MCP** helpers (`mcp_ainl_*`) automatically (see [api-reference.md](api-reference.md#get-apiagentsidtools)); you should see those names present after save/reload even if you did not type them manually. **Empty allowlist** still means “profile defaults” (no merge).
4. Optional: click **Add messaging tools** — `channel_send` and `event_publish` are added to the allowlist when it is non-empty, or removed from the blocklist when using profile-default tools (empty allowlist).
5. **Advanced full manifest:** expand **Show advanced — full manifest**, click **Reload from server** (textarea fills), change a harmless line (e.g. `description`), **Apply full manifest** — confirm dialog lists session clear + audit; after success, toast includes server **`note`** and audit hint.

**curl (replace `AGENT_ID` and port):**

```bash
curl -sS "http://127.0.0.1:4200/api/agents/AGENT_ID" | jq '{ system_prompt, identity, tool_allowlist, manifest_preview: (.manifest_toml[0:120] // "") }'
curl -sS "http://127.0.0.1:4200/api/agents/AGENT_ID/tools"
```

**API smoke (daemon running, replace `AGENT_ID`):**

```bash
curl -sS "http://127.0.0.1:4200/api/agents/AGENT_ID/session/digest"
# Expect JSON: session_id, agent_id, message_count, assistant_message_count
```

`./scripts/verify-dashboard-smoke.sh` calls this automatically when `GET /api/agents` returns at least one agent.

---

## Command Palette (Cmd/Ctrl+K)

The command palette overlays the full window and searches pages, agents, actions, and recent sessions simultaneously.

**Open / close:**

- Press **Cmd+K** (macOS) or **Ctrl+K** (Windows/Linux) from anywhere in the dashboard — the overlay should appear centered, focused, and ready to type.
- Press **Esc**, click the **esc** hint button, or click outside the dialog to close.
- The palette must **not** be visible on initial app load; it only opens on the keyboard shortcut.

**Search behavior:**

- With an empty query, the palette shows up to 5 recent agents, up to 5 agents, all pages, and all actions as grouped sections.
- Typing filters all sections simultaneously by name and description.
- Internal automation agents (`allowlist-probe`, `offline-cron`, `allow-ir-off`) must not appear.

**Keyboard navigation:**

- **↑ / ↓** move the highlight through all visible items across sections.
- **Enter** activates the highlighted item and closes the palette.
- Mouse hover also moves the highlight.

**Item actions:**

- **Recent / Agent** items open that agent's chat.
- **Page** items navigate to the corresponding hash route.
- **Actions** (New Agent, Reload Config, Toggle Focus Mode, etc.) execute immediately.

**curl smoke (daemon running):**

```bash
# Verify the command palette JS is served in the page bundle
curl -s http://127.0.0.1:4200/ | grep -c "commandPalette"
# Expect: 1 or more
```

---

## Chat UX — Sidebar & session features

### Pinned agents

- Hover over any agent row in the **Quick open** sidebar list — a small **pin** button appears to the left of the status dot.
- Click it to pin; the row gains an **accent left border** to indicate pinned state (the pin button itself disappears when not hovered).
- Pinned agents appear at the top of the **Quick open** list. The client seeds from `localStorage` (`armaraos-pinned-agents`) for instant display, then **`GET /api/ui-prefs`** loads the authoritative list from **`~/.armaraos/ui-prefs.json`** (so pins survive **desktop reinstalls** that wipe WebView storage). Each toggle runs **`PUT /api/ui-prefs`** with `{ "pinned_agents": [...] }`.
- Click the pin button again on hover to unpin and remove the accent border.
- The pin indicator (left border) must not overlap or obscure the green running-state dot on the right.

### Chat input history

- Send several messages in an agent chat.
- Press **↑** in the empty input field — the previous message should appear.
- Press **↑** again to go further back; **↓** to move forward through history.
- History is per-agent, persisted in `localStorage` under `armaraos-chat-history-<agent_id>` (up to 50 entries, deduped).
- Navigating away and back should preserve the history.

### Session rename

- Open an agent chat. Click the session name / title area at the top of the chat to enter edit mode (an `<input>` should appear).
- Type a new name and press **Enter** or click away — the name should update and persist.
- Press **Esc** to cancel without saving.
- The renamed session title must appear in the **Sessions** page and survive a daemon restart.

### Open workspace (Home folder)

- With an agent open in **Chat**, when **`GET /api/agents`** includes **`workspace_rel_home`** for that agent, the header shows a **workspace** pill (folder icon; tint follows **`identity.color`**).
- Click it — the app navigates to **`#home-files?path=<encoded workspace_rel_home>`** (or **`#home-files`** when the relative path is empty) so you can browse that agent’s on-disk workspace under ArmaraOS home.
- If the pill is missing, the workspace path is not under **`home_dir`** (not browseable via the Home-folder sandbox) — expect a toast instead of navigation.

### Jump back in (recent agents strip)

- The **Quick open** section in the sidebar shows the most recently used agents at the top, ordered by last activity time (stored in `localStorage` under `armaraos-recent-agents`).
- After chatting with an agent, navigate away and back — that agent should be first in the strip.
- Agents that are deleted from the system should be filtered out of the strip on next load.

### HTTP chat fallback — tool cards and assets

When the agent WebSocket is not connected, chat uses **`POST /api/agents/{id}/message`** (blocking). The response may include a top-level **`tools`** array (`name`, `input` as a JSON string, `result`, `is_error`) — one entry per tool execution for **the whole turn** (all LLM iterations in that request). The UI maps them into the same in-bubble **tool cluster** as the WebSocket path (`tool_start` / `tool_end` / `tool_result`). There is **no** token streaming on this path; the assistant reply appears once when the request completes.

**Manual checks:**

1. Disconnect or block WebSocket (e.g. devtools offline on WS only) so the toast **“Using HTTP mode (no streaming)”** appears.
2. Ask the agent to run a safe built-in tool (e.g. **`file_read`** on a small file in the workspace).
3. Expect **tool cluster** chrome (intro strip + collapsible cards + final assistant bubble styling) on the completed message, not only plain text.
4. After changing **`index_body.html`**, **`components.css`**, or **`chat.js`**, rebuild the daemon — the dashboard HTML/CSS/JS are **embedded** from `crates/openfang-api` at compile time (`webchat.rs` / `include_str!`). Restart **`openfang start`** (or the desktop shell) so the browser does not keep an old bundle.

**Optional API check (daemon running, loopback):**

```bash
AGENT_ID=$(curl -s http://127.0.0.1:4200/api/agents | python3 -c "import sys,json; print(json.load(sys.stdin)[0]['id'])")
curl -sS -H "Content-Type: application/json" \
  -d "{\"message\":\"Use file_read on README.md (or any small file). Reply ok.\"}" \
  "http://127.0.0.1:4200/api/agents/$AGENT_ID/message" | python3 -c \
  "import sys,json; d=json.load(sys.stdin); print('tools', len(d.get('tools') or []), [t.get('name') for t in d.get('tools') or []])"
```

Expect a non-zero tool count when the model actually invoked **`file_read`** (model-dependent).

### LLM error banner (`humanizeChatError`)

Provider failures show a dismissible amber/red banner; **hover** shows the raw daemon message (`lastStreamErrorTechnical`).

**Manual checks (wording only; use a dev key you can revoke):**

1. **401 / invalid key** — With an intentionally wrong `OPENROUTER_API_KEY`, send chat once: banner should steer toward **Settings / API key**, not billing.
2. **403** — If you can reproduce a provider **403** without invalid key text (e.g. model access), banner should **not** claim only a bad key; copy should mention access / hover for details (see **[openrouter.md](openrouter.md)**).
3. **Billing-like body** — If the raw error includes insufficient-credit wording, banner should mention **credits / provider dashboard**, not exclusively the API key field.

Contract: `static/js/pages/chat.js` → `humanizeChatError`.

### Chat history and tool call persistence

Tool call results, streaming activity, and full message history must survive agent-switch and page navigation without a round-trip delay.

**Switch agents:**

1. Open agent A, trigger a multi-tool run (e.g. ask it to read a file and search the web).
2. While results are streaming in, switch to agent B in the sidebar.
3. Switch back to agent A — all messages, tool cards, and partial streaming content must appear immediately (from the in-memory cache); the full history is then confirmed by the server round-trip in the background.

**Navigate away and return:**

1. Open agent A chat, send a message, let it complete.
2. Navigate to **Settings** or another page via the sidebar.
3. Navigate back to agent A — the full conversation (including any tool use cards) must be present without re-sending the message.

**Application upgrade:**

- Full history (including tool use blocks) is stored server-side in SQLite and reloaded on `loadSession` — it survives app upgrades.
- In-session cache (`_agentMsgCache`) survives component destruction within the same page load; it resets on hard refresh (expected behavior).

---

## `/btw` — mid-loop context injection

The `/btw` command lets you inject extra context into an agent loop that is already running.

**Using the command:**

1. Start a long-running task in an agent chat (e.g. ask it to write and test a multi-file feature).
2. While it is working, type `/btw <your context>` in the chat input and press Enter — e.g.:
   ```
   /btw Also make sure to add a changelog entry for this change
   ```
3. A local confirmation should appear in the timeline immediately.
4. On its next iteration, the agent should pick up the injected text as a `[btw] …` user message and incorporate it into its plan.

**When the agent is idle:**

- Sending `/btw` when no loop is running should show an error toast (agent not currently running — wait until it is busy, then inject).

**curl smoke:**

```bash
AGENT_ID=$(curl -s http://127.0.0.1:4200/api/agents | python3 -c "import sys,json; print(json.load(sys.stdin)[0]['id'])")

# Send a long task first, then immediately inject:
curl -s -X POST "http://127.0.0.1:4200/api/agents/$AGENT_ID/btw" \
  -H "Content-Type: application/json" \
  -d '{"text": "Also check the Cargo.lock is up to date."}'
# Returns 200 {"status":"injected"} if loop is active,
# or 409 {"error":"agent not running"} if idle.
```

---

## Slash templates

Reusable message shortcuts stored server-side in `~/.armaraos/slash-templates.json`.

**Create a template:**

```
/t save standup Give me a morning standup summary: what did you do yesterday, what's today's plan, any blockers?
```

**Use a template:**

```
/t standup
```

The saved text is expanded and sent as your message.

**List templates:**

```
/t list
```

Shows all saved template names.

**Delete a template:**

```
/t delete standup
```

**Persistence check:**

```bash
# After saving at least one template:
curl -s http://127.0.0.1:4200/api/slash-templates | python3 -m json.tool
# Expect: {"templates": [...]}

cat ~/.armaraos/slash-templates.json
# Same JSON on disk — survives application upgrades and browser data clears.
```

**Overwrite (PUT) smoke:**

```bash
curl -s -X PUT http://127.0.0.1:4200/api/slash-templates \
  -H "Content-Type: application/json" \
  -d '{"templates":[{"name":"test","text":"This is a test template."}]}'
# Expect: {"status":"saved","count":1}

curl -s http://127.0.0.1:4200/api/slash-templates
# Expect: {"templates":[{"name":"test","text":"This is a test template."}]}
```

---

## Graph Memory (`#graph-memory`)

**Navigation:** Sidebar **Graph Memory**, or `http://127.0.0.1:4200/#graph-memory`.

**Manual checks**

1. Pick an agent with existing **`ainl_memory.db`** data (or run a short chat turn to create episodes).
2. **Canvas:** Episode / semantic / procedural / persona / **runtime** filters toggle node sets; edges show a **tooltip** with relation kind (hover line).
3. **Details panel (node click):** Shows **What**, **Why**, **Evidence** (JSON), **Edges (typed)** with direction + `rel`, and **Neighbors**. Full node id + **Copy**.
4. **Live timeline:** After graph writes, entries should show **kernel-provided summaries** when `SystemEvent::GraphMemoryWrite.provenance` is present (not only the generic “New … stored”). Click a row to **focus** the node when `nodeId` is known.
5. **API spot-check:** `GET /api/graph-memory?agent_id=<uuid>&limit=50` — each node should include **`explain`** with `what_happened` / `why_happened` / `evidence` / `relations`.

See **[GRAPH_MEMORY_EXPLAINABILITY.md](GRAPH_MEMORY_EXPLAINABILITY.md)** for the event/API contract and release ordering.
