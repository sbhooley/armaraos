# Dashboard testing notes

## Automated API smoke (daemon running)

With the daemon running, pass the **Dashboard** base URL from `openfang start` (default is often `http://127.0.0.1:4200`, but `config.toml` may use another port such as `50051`):

```bash
./scripts/verify-dashboard-smoke.sh http://127.0.0.1:4200
# or, if the CLI printed a different dashboard URL:
./scripts/verify-dashboard-smoke.sh http://127.0.0.1:50051
```

This hits `/api/health`, `/api/status`, `/api/schedules`, **`GET /api/version/github-latest`**, **`GET /api/logs/daemon/recent?lines=5`** (may return empty `lines` until `logs/daemon.log` exists), `POST /api/support/diagnostics` (writes a zip under `~/.armaraos/support/` on success), **`GET /api/support/diagnostics/download`** for that zip’s `bundle_filename`, **`GET /api/armaraos-home/download`** for the same file under `support/…`, a sample `POST /api/agents` to verify auth/error JSON when a key is configured, and (when agents exist) `GET /api/agents/:id/session/digest`.

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

5. **App Store → on-disk section title** — Open **App Store** (`#ainl-library`). The collapsible catalog section should read **AI Native Lang Programs Available** (not the old “on disk” wording).

6. **Settings and Runtime pages — layout polish** — Open **Settings**: elevated header with subtitle; tab bar is a rounded card with accent top stripe and pill-style active tab. Open **Runtime**: header subtitle; stat tiles wrap in a responsive grid on a narrow window; **System** and **Providers** panels use uppercase section titles and readable tables. **Runtime** footer: **Refresh**, **Reload config**, **Reload channels**, **Reload integrations**, **Shut down** (see [Daemon lifecycle & GitHub version check](#daemon-lifecycle--github-version-check) below).

7. **Settings → System Info → Support** — Use **Generate diagnostics bundle** (same redacted `.zip` as the API). Confirm the UI mentions `.zip` and the generated path matches a file on disk. On **desktop**, confirm a copy lands in **Downloads** (or use the fallback download if copy fails). With `api_key` set, confirm the browser/WebView still completes save (loopback GET download + `token` query).

8. **Home folder → `support/`** — Open **Home folder** in the nav, go to **`support`**, find a diagnostics `.zip`. Use row **Download** (green) or **View** then **Download** in the modal header. Large zips may show a preview error; **Download** must still save the full file. On desktop, row **Download** uses Tauri **`copy_home_file_to_downloads`** (`relativePath`).

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

**Download (API):** `GET /api/support/diagnostics/download?name=<bundle_filename>` streams the zip (`Content-Disposition: attachment`). From **loopback**, this GET is also allowed without Bearer (same rationale as POST). Remote clients must authenticate. The dashboard client may append `&token=<api_key>` and sends `Authorization` when a key is stored.

**Desktop app:** After create, the shell invokes **`copy_diagnostics_to_downloads`** with **`{ bundlePath: "<absolute path from JSON>" }`** (Tauri camelCase, single required key). If copy fails, the UI may fall back to the HTTP download above. Rebuild the desktop app after changes to this command or permissions.

**Copy any home file (desktop):** From **Home folder**, row **Download** uses **`copy_home_file_to_downloads`** with **`{ relativePath: "support/armaraos-diagnostics-….zip" }`** (path relative to ArmaraOS home). Allowed in `crates/openfang-desktop/permissions/dashboard.toml` and generated ACL manifests.

## Home folder browser — preview vs download

**List / preview:** `GET /api/armaraos-home/list` and `GET /api/armaraos-home/read` — **read** is capped at **512 KiB** and returns UTF-8 text or base64 for small binary previews.

**Full file:** `GET /api/armaraos-home/download?path=<relative path>` streams up to **256 MiB** with attachment headers. Loopback may call without Bearer (embedded dashboard). Use this when diagnostics zips (or other large files) fail preview with “file too large”.

**UI:** The **Home folder** table has **View** (preview) and **Download** (full file). The file modal includes **Download** in the header whenever a path is set, plus **Download file** in the error panel when preview fails — large `.zip` under `support/` should still be saveable.

## Kernel SSE (`GET /api/events/stream`)

- **Smoke:** Open the dashboard, confirm the sidebar **SSE** badge appears when the stream connects (kernel running, same origin / loopback as usual).
- **API:** `cargo test -p openfang-api --test api_integration_test test_kernel_events_stream_sse_smoke` and `cargo test -p openfang-api --test sse_stream_auth` cover HTTP behavior (including loopback vs remote auth).

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
- **Page leave:** The overview component registers `@page-leave.window="stopAutoRefresh()"` so timers and kernel listeners are cleared when navigating away. If you add **Playwright** (or similar) later, assert that after switching to another hash/route, `setInterval`-driven refresh is not still firing (e.g. spy on `/api/usage` or equivalent after leaving the Get started page).

## Chat unread badges + session digest

**Behavior (dashboard static app):**

- **Badges** appear on **All Agents** (sidebar nav count), the **Chat** page title (total), **Quick open** agent rows, and **agent picker** cards when the UI detects new **assistant** activity for an agent while that conversation is not the focused, visible inline chat.
- **Sources:** (1) WebSocket frames `response` / `canvas` (broadcast as `armaraos-agent-ws`), (2) kernel SSE `Message` events with `role: agent` and an `Agent` target (e.g. inter-agent), (3) **digest polling** every ~24s using `GET /api/agents/{id}/session/digest` so unread still updates if the WS is disconnected or another process appended to the session.
- **Baseline:** The client keeps a per-agent `assistant_message_count` baseline from the digest to avoid double-counting when combined with WS/SSE; opening a chat **primes** the baseline and clears that agent’s unread.
- **Navigate away from `#agents`:** The socket may stay open; `OpenFangAPI.wsClearUiCallbacks()` drops Alpine chat handlers so destroyed components are not called. Returning to the same agent reattaches handlers and may **reuse** the connection.
- **Session switch / new session:** Chat code calls `wsDisconnect()` before reconnect so the server session binding stays correct.

## Agents page → Config tab (identity, prompt, tool filters)

**API contract:** `GET /api/agents` and `GET /api/agents/{id}` return **`system_prompt`**, full **`identity`** (`archetype`, `vibe`, …), and (on the detail route) **`tool_allowlist`** / **`tool_blocklist`**. The dashboard loads detail after open and reapplies the form so edits are not blank. **`PATCH /api/agents/{id}/config`** ignores empty `system_prompt` / `description` and merges identity so stray `""` values do not wipe stored data.

**Manual checks (daemon + browser):**

1. Spawn or pick an agent; set **Archetype**, **Vibe**, **System prompt**, and add tools to **Allowlist** / **Blocklist**; save.
2. Close the detail modal and reopen **Config** — fields and lists should match what you saved.
3. Optional: click **Add messaging tools** — `channel_send` and `event_publish` are added to the allowlist when it is non-empty, or removed from the blocklist when using profile-default tools (empty allowlist).

**curl (replace `AGENT_ID` and port):**

```bash
curl -sS "http://127.0.0.1:4200/api/agents/AGENT_ID" | jq '.system_prompt, .identity, .tool_allowlist'
curl -sS "http://127.0.0.1:4200/api/agents/AGENT_ID/tools"
```

**API smoke (daemon running, replace `AGENT_ID`):**

```bash
curl -sS "http://127.0.0.1:4200/api/agents/AGENT_ID/session/digest"
# Expect JSON: session_id, agent_id, message_count, assistant_message_count
```

`./scripts/verify-dashboard-smoke.sh` calls this automatically when `GET /api/agents` returns at least one agent.
