# Changelog

All notable changes to OpenFang will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.6.6] - 2026-04-07

### Added

- **Dashboard → Settings:** At-a-glance line under the tab bar (**Daemon**, **Config schema** as `effective (binary N)`, **API**, **Log**, **Home**); **System** tab **Config schema** stat tile.
- **Support diagnostics zip:** `README.txt`, `diagnostics_snapshot.json` (structured triage: config schema, paths, runtime, memory SQLite `user_version` vs expected, env override presence flags), and expanded `meta.json` (plus existing config, secrets redaction, audit, DB, logs).
- **`openfang-memory`:** `memory_substrate_schema_expected()`, `read_sqlite_user_version()` for read-only bundle snapshots.
- **Dashboard:** **Command palette** (Cmd/Ctrl+K) — full-window overlay searching pages, agents, actions, and recent sessions (`static/js/pages/command-palette.js`, `index_body.html`).
- **HTTP API:** **`GET /api/system/network-hints`** — host-side VPN/tunnel/proxy hints (`crates/openfang-api/src/network_hints.rs`); wired into Setup Wizard and chat; loopback GET allowed without Bearer (see `middleware.rs`).
- **Embedded AINL (`programs/`):** Six new compact graphs materialized with the kernel’s **`armaraos-programs`** mirror (see [docs/ootb-ainl.md](docs/ootb-ainl.md)):
  - **`agent_health_monitor`** — polls `GET /api/health` and `GET /api/agents` on the local daemon (comments: ~15 min cadence when scheduled).
  - **`budget_threshold_alert`** — compares spend to budget; emits when usage crosses **80%** of the configured limit (comments: hourly when scheduled).
  - **`channel_session_digest`** — lightweight snapshot (`active_agents`, timestamp) for session feeds (comments: ~6 h when scheduled).
  - **`daily_budget_digest`** — morning budget summary: period, totals, limit (comments: **08:00** when scheduled).
  - **`new_version_checker`** — compares **GitHub** latest ArmaraOS tag and **PyPI** `ainativelang` to `GET /api/version` (comments: weekly **Saturday 10:00** when scheduled).
  - **`weekly_usage_report`** — aggregates budget/agents/skills and calls **`llm.COMPLETION`** (`llm/openrouter`) for a short summary (comments: **Sunday 18:00** when scheduled; requires **`AINL_MCP_LLM_ENABLED=1`** or **`AINL_CONFIG`** with an LLM section).

### Documentation

- **Config schema & diagnostics:** [troubleshooting.md](docs/troubleshooting.md) (TOC, dashboard at-a-glance, bundle contents); [api-reference.md](docs/api-reference.md) (`GET /api/status`, `GET /api/config`, `POST /api/support/diagnostics`); [dashboard-settings-runtime-ui.md](docs/dashboard-settings-runtime-ui.md); [dashboard-testing.md](docs/dashboard-testing.md); [data-directory.md](docs/data-directory.md); [configuration.md](docs/configuration.md) (`config_schema_version` row); [getting-started.md](docs/getting-started.md); [desktop.md](docs/desktop.md); [docs/README.md](docs/README.md); root [CLAUDE.md](CLAUDE.md).
- **`docs/agent-automation-hardening.md`:** Agent workflows — valid `file_write` / `shell_exec` JSON, persist vs re-acquire, loop guard interaction, acquire/extract/persist/verify phases, workspace habits, optional future preflight notes and caveats; **`armaraos-skill-mint-stub-monthly`** reference.
- **`docs/troubleshooting.md`:** New subsection for missing `path`/`command`; loop guard note on empty repeated tool calls; TOC link to hardening guide.
- **`docs/ootb-ainl.md`:** Expanded **`armaraos-skill-mint-stub-monthly`** row (schedule, frame, host Markdown).
- **`docs/README.md`**, **`docs/agent-files-and-documents.md`:** Cross-links and **`file_write`** section.

## [0.6.5] - 2026-04-05

### Added

- **HTTP API:** **`GET /api/version/github-latest`** — server-side fetch of the latest GitHub release for the ArmaraOS repo (dashboard **Check daemon vs GitHub** / **Check vs GitHub** no longer calls `api.github.com` from the browser).
- **Dashboard → Daemon / API:** **Reload config** (`POST /api/config/reload`), **Reload channels** (`POST /api/channels/reload`), **Reload integrations** (`POST /api/integrations/reload`), and **Shut down** (`POST /api/shutdown`) with confirmation modals; shared Alpine mixin in **`static/js/daemon_lifecycle.js`** (bundled from **`webchat.rs`** after **`api.js`**). **`OpenFangToast.confirm`** accepts optional **`{ confirmLabel, danger }`**.
- **Dashboard → Get started:** **Quick actions** — **App Store** (`#ainl-library`), **Daemon & runtime** (`#runtime`), plus agents/skills/channels/workflows/settings; **seven-tile** loading skeleton. **Setup Wizard** / **Run setup again** in the page header with visibility tied to **`localStorage`** **`openfang-onboarded`**; sidebar **Get started** re-click (`navigateOverview`) reveals the wizard for onboarded users; checklist **Setup Wizard** button follows the same flag.
- **Dashboard → Settings / Runtime:** Page-scoped backgrounds, headers with subtitles, Settings tab bar in a rounded accent toolbar, Runtime responsive stat grid and styled **System** / **Providers** panels (`settings-page-*`, `runtime-page-*` classes in `components.css`).
- **Dashboard → App Store:** On-disk catalog section title **AI Native Lang Programs Available** (replacing “AINL programs on disk”).
- **HTTP API:** `GET /api/agents` and `GET /api/agents/{id}` expose **`system_prompt`**, full **`identity`** (`archetype`, `vibe`, `greeting_style`, …), and detail adds **`tool_allowlist`** / **`tool_blocklist`** for dashboard and clients.
- **Dashboard (Agents):** **Config** tab reloads agent detail into the form; **Add messaging tools** for `channel_send` / `event_publish`; save re-fetches to stay in sync.
- **Tool presets (`openfang-types`):** Non-**Full** profiles (Minimal, Coding, Research, Messaging, Automation, …) include **`channel_send`** and **`event_publish`** where appropriate.
- **Bundled hands:** Metadata and skills reference channel/event tools for alerts (e.g. Predictor); regression coverage in **`openfang-hands`**.
- **Desktop (Tauri):** OS notifications use ArmaraOS branding; **`HealthCheckFailed`** is not shown as a desktop toast (logs / Web UI only).
- **Scheduled `ainl run` (kernel):** each `ainl` subprocess receives **`AINL_ALLOW_IR_DECLARED_ADAPTERS=1`** by default so IR-declared adapters (e.g. **`web`**, **`http`**) work without users exporting host-adapter env; per-agent opt-out via manifest **`ainl_allow_ir_declared_adapters`** (`"0"`, `"false"`, `"off"`, `"no"`, or JSON **`false`**).
- **HTTP API:** agent detail **`scheduled_ainl_host_adapter`** includes **`ainl_allow_ir_declared_adapters`** (`"1"` / `"0"`) alongside allowlist summary fields.
- **Desktop (Tauri) — product analytics:** Optional one-time PostHog event **`armaraos_desktop_first_open`** (anonymous: app version, OS, arch). Release builds can embed **`ARMARAOS_POSTHOG_KEY`** at compile time (GitHub Actions secret); runtime env overrides for debugging. Send is deferred (**~120s** or after Setup Wizard **Welcome** → **Get Started** when usage stays enabled); prefs in **`desktop_telemetry_prefs.json`**. Opt-out: uncheck **Anonymous usage** on wizard step 1, or **`ARMARAOS_PRODUCT_ANALYTICS=0`**. IPC: **`get_desktop_product_analytics_prefs`**, **`set_desktop_product_analytics_prefs`** (dashboard permission allowlist).
- **Dashboard → Setup Wizard:** **Anonymous usage** checkbox on Welcome (desktop shell only) syncs telemetry consent before any analytics request.
- **HTTP API:** `GET /api/armaraos-home/download?path=` streams a file from the ArmaraOS home tree as `application/octet-stream` with `Content-Disposition: attachment` (cap **256 MiB**; separate from the 512 KiB **preview** limit on `GET /api/armaraos-home/read`).
- **Dashboard → Home folder:** Per-row **Download** (green) and modal **Download** / **Download full file** / error-state **Download file** so large binaries (e.g. diagnostics `.zip`) save even when **View** fails with “file too large” for preview.
- **Desktop (Tauri):** `copy_home_file_to_downloads` — copies a home-relative path (e.g. `support/armaraos-diagnostics-*.zip`) to the user **Downloads** folder (used from the Home folder page on desktop).
- **CLI daemon:** `openfang start` / **`openfang gateway start`** mirror `tracing` to **stderr** and **`{home}/logs/daemon.log`** (creates `logs/` as needed); falls back to stderr-only if the file cannot be opened.
- **HTTP API:** `GET /api/logs/daemon/recent` and **`GET /api/logs/daemon/stream`** (SSE) read the daemon tracing file (`daemon.log`, else `tui.log`); **`GET /api/logs/stream`** supports `level` and `filter` query parameters for server-side audit filtering.
- **Dashboard → Logs:** **Daemon** tab (tail + SSE, optional `log_level` save reminding restart); **Live** tab reconnects the audit SSE when filters change.
- **Tests:** `crates/openfang-api/tests/sse_stream_auth.rs` asserts loopback vs non-loopback auth for **`/api/logs/daemon/stream`**.

### Changed

- **`PATCH /api/agents/{id}/config`:** Empty **`system_prompt`** / **`description`** are ignored; identity fields merge so **`""`** clears optional strings but does not wipe color accidentally; **`PATCH …/identity`** merges with current row instead of replacing unspecified fields with null.
- **Dashboard → Sidebar:** **Comms** moved under **Monitor** (with Timeline, Logs, Runtime, …) instead of **Agents**.
- **Dashboard → Skills, Channels, Hands, Home folder, Analytics:** Shared **`dashboard-page-body-polish`** / **`dashboard-page-header-polish`** shell; **`dashboard-toolbar-tabs`** on tab rows; **Channels** filters in **`dashboard-inline-filters`**; **Analytics** stats on **`dashboard-stats-grid`** / **`dashboard-stat-card`**; **Home folder** polished header + **`dashboard-home-intro-panel`**.
- **Dashboard:** Sidebar labels the landing dashboard **Get started** and places it **above Chat**; page title matches. Internal route id remains `overview` / `#overview`.
- **Dashboard — setup checklist:** First chat message and browse/install skill rows are **perpetual shortcuts** (always ○ + **Go**, never marked complete). Optional progress after core steps tracks **channel** only. Removed `localStorage` keys `of-first-msg` and `of-skill-browsed` and checklist refresh via `armaraos-onboarding-local`.
- **Dashboard → Get started:** **Quick actions** moved to the top of the page (after the Live SSE strip) with a grid card and loading skeleton; removed the duplicate quick-actions block from the bottom.

### Fixed

- **Agents → Config:** Opening or saving agent settings no longer wiped **system prompt**, **archetype**, **vibe**, or tool allow/block lists—the API returns those values on **`GET /api/agents`** / **`GET /api/agents/{id}`**, the dashboard reloads detail into the form, and partial PATCH bodies no longer overwrite stored fields with empty strings.
- **Dashboard → Agents list:** Internal automation probe agents (**`allowlist-probe`**, **`offline-cron`**, **`allow-ir-off`**) stay available for automation but are **hidden from the main agent sidebar**; grouped with existing internal-chat behavior (`isInternalAutomationProbeChatAgentName` in **`js/app.js`**).
- **Desktop (`openfang-desktop`):** after **`~/.armaraos/.env`** / **`secrets.env`**, sets **`AINL_ALLOW_IR_DECLARED_ADAPTERS=1`** when still unset; **`ainl_try_library_file`** (Settings → AINL library validate/run) passes **`AINL_ALLOW_IR_DECLARED_ADAPTERS=1`** on the subprocess.
- **Support diagnostics zip:** `GET /api/support/diagnostics/download` is allowed from **loopback** without Bearer (same policy as `POST …/diagnostics`) so the dashboard fetch + blob save works when `api_key` is set; client also sends `token` query + `credentials: 'same-origin'` for robustness.
- **Desktop:** `copy_diagnostics_to_downloads` again takes a single argument **`bundlePath`** (Tauri IPC camelCase) to match code generation; resolves `support/<filename>` when needed before copying to Downloads.
- **Home folder:** Symlink entries can use row **Download**; preview modal always exposes **Download** when a path is known.

### Docs

- **`docs/api-reference.md`:** **`GET /api/version/github-latest`**; reload/shutdown routes; agents list/detail + **`PATCH`/`GET`/`PUT`** config/tools; **`GET /api/logs/daemon/recent`**, audit and daemon SSE; ArmaraOS home **`/download`**; **`scheduled_ainl_host_adapter.ainl_allow_ir_declared_adapters`**; summary table.
- **`docs/dashboard-overview-ui.md`**, **`docs/dashboard-settings-runtime-ui.md`**, **`docs/dashboard-testing.md`**, **`docs/dashboard-home-folder.md`**, **`docs/dashboard-bookmarks.md`:** Get started (quick actions, Setup Wizard, **App Store**, seven-tile skeleton); Settings/Runtime/daemon lifecycle (**`daemon_lifecycle.js`**); Skills/Channels/Hands/Analytics/Home polish classes; support bundle + Home folder QA; **`github-latest`** and **`verify-dashboard-smoke.sh`** smoke steps; Logs tabs.
- **`docs/README.md`**, **`docs/getting-started.md`**, **`docs/troubleshooting.md`**, **`docs/architecture.md`**, **`docs/configuration.md`**, **`docs/scheduled-ainl.md`**, **`CONTRIBUTING.md`**, **`CLAUDE.md`**, **`docs/release-desktop.md`**, **`docs/ootb-ainl.md`**, **`docs/channel-adapters.md`**, **`docs/agent-templates.md`**, **`docs/desktop.md`**, **`docs/data-directory.md`**, **`docs/cli-reference.md`:** Cross-links, diagnostics/home download, daemon tracing, gateway CLI, PostHog/release-desktop, AINL env and scheduled runs, **`docs/snippets/agent-metadata-intelligence-cron.toml`**.
- **`README.md`:** Diagnostics, home-folder download, PostHog analytics (collection, opt-out, CI secrets).
- **`.env.example`:** PostHog vars — baked key vs runtime override.
- **`scripts/verify-dashboard-smoke.sh`:** Diagnostics download, **`armaraos-home/download`**, **`GET /api/logs/daemon/recent`**, **`GET /api/version/github-latest`**.

### Build / CI

- **`.github/workflows/release.yml`:** Desktop job passes **`ARMARAOS_POSTHOG_KEY`** and **`ARMARAOS_POSTHOG_HOST`** from secrets into the Tauri build (optional; empty when unset).

## [0.6.4] - 2026-04-05

### Added

- **Setup wizard (dashboard):** After saving an API key, the wizard automatically runs the provider **connection test** and only enables **Next** when it succeeds; entering the provider step with an already-configured key triggers the same check. Inline copy explains verify-before-continue behavior.
- **Dashboard:** Event timeline experience (`timeline.js` + routing), channels and scheduler UI improvements, agents page polish (spawn defaults, stats), overview and usage tweaks.
- **Desktop:** Updater and AINL integration refinements (`updater.rs`, `ainl.rs`, `lib.rs`, `ainl_version.rs`).
- **Dashboard → Chat unread:** Notification-style badges on **All Agents** (nav + Chat heading), **Quick open** sidebar rows, and **agent picker** cards when there is new assistant-side activity; cleared when the user opens that agent’s chat or returns to a visible tab on that conversation.
- **HTTP API:** `GET /api/agents/{id}/session/digest` returns `message_count`, `assistant_message_count`, and ids only (no full transcript) for lightweight polling.
- **Dashboard resilience:** Agent WebSocket can stay connected when navigating away from `#agents` (UI callbacks detached via `wsClearUiCallbacks`); global `armaraos-agent-ws` + periodic digest polling (~24s) keep unread accurate when the stream is down or another client updated the session.

### Changed

- **Default models:** Bundled `agents/*/agent.toml`, TUI templates/wizard, and related surfaces align on **OpenRouter** with **`stepfun/step-3.5-flash:free`** (or provider-appropriate fallbacks) for new-agent defaults.
- **Hands:** Bundled predictor and other packaged hands metadata updates (`HAND.toml`, `SKILL.md`, `bundled.rs`).
- **Kernel / runtime / types:** Registry, agent manifest handling, approval/heartbeat hooks, LLM driver and agent-loop adjustments to match the above.
- **Dashboard static client (`api.js`):** `wsConnect` reuses an existing open socket for the same agent id (callback refresh only); `wsDisconnect` still used when backing out of chat or switching sessions.

### Docs

- **`docs/dashboard-testing.md`:** Chat unread behavior, digest endpoint, and smoke-script note.
- **`CLAUDE.md`:** Pointers for dashboard chat/unread and `session/digest` live checks.

### Fixed

- **Chat (HTTP + WebSocket):** When the assistant produces **no text** and **token usage is 0**, the UI message now points users at **missing or invalid provider API keys** (e.g. OpenRouter / `OPENROUTER_API_KEY`) instead of a generic empty reply.
- **Setup wizard:** The **selected** provider must be configured and **verified** before continuing; the progress bar can no longer skip ahead without meeting that bar (avoids OpenRouter 401s after “completing” setup with another provider’s key only).

## [0.6.3] - 2026-04-04

### Added

- **Dashboard → Home folder:** Read-only browse of the daemon ArmaraOS home directory (`~/.armaraos` / `ARMARAOS_HOME`) with `GET /api/armaraos-home/list` and `GET /api/armaraos-home/read` (path sandboxing, size caps).
- **Optional safe edits:** `[dashboard] home_editable_globs` in `config.toml` (globset patterns) enables `POST /api/armaraos-home/write` for UTF-8 files; blocked paths include `data/`, `.env`, `config.toml`, `vault.enc`, and other secrets/core files. Optional `.bak` before overwrite.

### Changed

- **Desktop updater:** After the marketing-site Tauri feed reports “up to date”, the app now also compares the running version to **GitHub’s latest release** (same as the existing fallback when the feed errors), so users see an update notification and release link when ainativelang.com is stale.
- **Formatting:** Workspace rustfmt applied (`cargo fmt --all`) so CI `cargo fmt --check` stays green.

### Docs

- **`docs/release-desktop.md`:** Table explaining when the “Publish updater to ainativelang.com” CI job is skipped, fails, or exits without a push.

## [0.6.2] - 2026-04-02

### Added

- **Dashboard resilience:** Friendly recovery UI when the embedded dashboard fails to load (static assets or API unreachable), with reload and open-in-browser actions.
- **`scripts/verify-dashboard-smoke.sh`:** Optional local smoke script for dashboard/API checks documented in `docs/dashboard-testing.md`.

### Changed

- **HTTP API:** More consistent JSON error bodies (`detail`, `path`, `request_id` where applicable) on key routes; rate limiting and middleware aligned with expanded route surface.
- **Dashboard:** Chat layout and scroll behavior updates; wizard and settings copy aligned with configured provider/model; assorted Overview, Runtime, Skills, and Agents polish.
- **Docs:** Troubleshooting, production checklist, desktop and dashboard testing guides updated for diagnostics bundles and release flow.

### Fixed

- **AINL cron / daemon:** The desktop app now writes **`~/.armaraos/.armaraos-ainl-bin`** (absolute path to the internal venv `ainl`) whenever AINL status is healthy, so the background kernel can run scheduled AINL jobs without **`ainl` on `PATH`** or **`ARMARAOS_AINL_BIN`** (resolves “No such file or directory” spawn failures in Audit logs).

## [0.6.1] - 2026-04-02

### Added

- **Scheduler output visibility:** Scheduled cron jobs now emit user-visible entries in the Audit Log (Dashboard → Logs), and scheduled AINL outputs are appended into the associated agent’s chat session without invoking the LLM.
- **Desktop update UX:** Desktop-only “Check for Updates” buttons were added to Runtime and Settings, and update activity is logged to the Audit Log. If the website updater feed is unreachable, ArmaraOS falls back to a public GitHub Releases check (download-page flow).
- **AINL library usability:** Added a Strict validation toggle in AINL Library (runs `ainl validate` with or without `--strict`).

### Changed

- **Brand theming:** Default dashboard accent color is now red-forward (`#ef5350`) instead of orange-forward.
- **AINL upstream sync:** Desktop upstream AINL library sync now defaults to a tag matching the installed `ainativelang` version to reduce validation failures from `main`/version skew (override via `ARMARAOS_AINL_LIBRARY_REF`).
- **TUI templates:** Built-in templates now inherit the system default model/provider (`provider="default"`, `model="default"`) instead of hard-coded provider/model pairs.

### Fixed

- **LLM resilience:** When rate limited or overloaded after retries, agents automatically attempt OpenRouter free-model fallbacks (`stepfun/step-3.5-flash:free`, then `nvidia/nemotron-3-super-120b-a12b:free`) to keep the UX flowing.

## [0.6.0] - 2026-04-01

### Added

- **AINL OOTB:** The kernel embeds the repo `programs/` tree at build time (`crates/openfang-kernel/build.rs`) and materializes it to `~/.armaraos/ainl-library/armaraos-programs/` on boot. Curated cron registers idempotently: a **default enabled** weekly job **`armaraos-ainl-health-weekly`** runs a core-only health graph with `ainl run --json`; additional catalog entries (upstream examples, learning-frame echo with frame JSON, skill-mint stub, automation HTTP template) can be toggled in the Scheduler. Opt-out env: **`ARMARAOS_DISABLE_CURATED_AINL_CRON=1`**, **`ARMARAOS_SKIP_EMBEDDED_AINL_PROGRAMS=1`**. Dashboard Scheduler and AINL Library pages explain `armaraos-programs/` vs mirrored `demo/` / `examples/` / `intelligence/`; **Register curated cron** shows both `registered` and `embedded_programs_written`. CI job **openfang-kernel + AINL programs** builds/tests the kernel and runs `ainl validate --strict` on all `programs/**/*.ainl`. Manual steps: [docs/ootb-ainl-smoke.md](docs/ootb-ainl-smoke.md).
- **Tests:** SSE smoke for `GET /api/events/stream` (`api_integration_test`); auth matrix for kernel events stream (`sse_stream_auth`: loopback vs remote, Bearer token).
- **Dashboard:** Overview shows an optional **Last kernel event** line from `kernelEvents.last`; Settings → AINL shows **Last checked** after PyPI/GitHub version checks (desktop).
- **Docs:** README subsection on kernel SSE; `docs/dashboard-testing.md`; `docs/release-desktop.md`.
- **Docs:** `docs/data-directory.md` (default `~/.armaraos/`, `ARMARAOS_HOME` / `OPENFANG_HOME`, migration from `~/.openfang`); README and user-facing guides updated so paths and env vars match runtime behavior; `MIGRATION.md` destinations use `~/.armaraos/`.
- **Docs:** `docs/docker.md` — Docker image, OpenSSL, cargo-chef, build args, multi-arch notes.

### Changed

- **BREAKING:** Dashboard password hashing switched from SHA256 to Argon2id. Existing `password_hash` values in `config.toml` must be regenerated with `openfang auth hash-password`. Only affects users with `[auth] enabled = true`.
- **`GET /api/events/stream`** and **`GET /api/logs/stream`**: when `api_key` is set, requests from **non-loopback** addresses now require authentication (Bearer or `token` query), same as other protected routes. Loopback clients may still open the stream without credentials so the embedded dashboard works locally.

### Fixed

- Dashboard passwords were hashed with plain SHA256 (no salt), making them vulnerable to rainbow table and GPU-accelerated brute force attacks. Now uses Argon2id with random salts.
- **Docker / CI:** GHCR image builds failed while building vendored OpenSSL from source (Perl/toolchain in `rust:slim`). The Dockerfile now sets **`OPENSSL_NO_VENDOR=1`**, installs **`libssl3`** at runtime, uses **`cargo-chef`** with **`--package openfang-cli`**, copies **`programs/`** for `openfang-kernel` embeds, and defaults to **thin LTO** + **parallel codegen units** for faster builds without changing behavior.
- **Docker:** Default **`api_listen`** is **`127.0.0.1:50051`**, which does not accept connections from the host through Docker port publishing. The runtime image now sets **`OPENFANG_LISTEN=0.0.0.0:50051`**, **`EXPOSE 50051`**, and docs use **`-p 50051:50051`** so the dashboard is reachable at **`http://localhost:50051/`**.
- **CLI:** `openfang start` in Linux containers skips the “daemon already running” HTTP probe when **`/.dockerenv`** exists (avoids false positives with host networking); optional **`OPENFANG_SKIP_DAEMON_CHECK=1`** for Podman and similar.

## [0.1.0] - 2026-02-24

### Added

#### Core Platform
- 15-crate Rust workspace: types, memory, runtime, kernel, api, channels, wire, cli, migrate, skills, hands, extensions, desktop, xtask
- Agent lifecycle management: spawn, list, kill, clone, mode switching (Full/Assist/Observe)
- SQLite-backed memory substrate with structured KV, semantic recall, vector embeddings
- 41 built-in tools (filesystem, web, shell, browser, scheduling, collaboration, image analysis, inter-agent, TTS, media)
- WASM sandbox with dual metering (fuel + epoch interruption with watchdog thread)
- Workflow engine with pipelines, fan-out parallelism, conditional steps, loops, and variable expansion
- Visual workflow builder with drag-and-drop node graph, 7 node types, and TOML export
- Trigger system with event pattern matching, content filters, and fire limits
- Event bus with publish/subscribe and correlation IDs
- 7 Hands packages for autonomous agent actions

#### LLM Support
- 3 native LLM drivers: Anthropic, Google Gemini, OpenAI-compatible
- 27 providers: Anthropic, Gemini, OpenAI, Groq, OpenRouter, DeepSeek, Together, Mistral, Fireworks, Cohere, Perplexity, xAI, AI21, Cerebras, SambaNova, Hugging Face, Replicate, Ollama, vLLM, LM Studio, and more
- Model catalog with 130+ built-in models, 23 aliases, tier classification
- Intelligent model routing with task complexity scoring
- Fallback driver for automatic failover between providers
- Cost estimation and metering engine with per-model pricing
- Streaming support (SSE) across all drivers

#### Token Management & Context
- Token-aware session compaction (chars/4 heuristic, triggers at 70% context capacity)
- In-loop emergency trimming at 70%/90% thresholds with summary injection
- Tool profile filtering (cuts default 41 tools to 4-10 for chat agents, saving 15-20K tokens)
- Context budget allocation for system prompt, tools, history, and response
- MAX_TOOL_RESULT_CHARS reduced from 50K to 15K to prevent tool result bloat
- Default token quota raised from 100K to 1M per hour

#### Security
- Capability-based access control with privilege escalation prevention
- Path traversal protection in all file tools
- SSRF protection blocking private IPs and cloud metadata endpoints
- Ed25519 signed agent manifests
- Merkle hash chain audit trail with tamper detection
- Information flow taint tracking
- HMAC-SHA256 mutual authentication for peer wire protocol
- API key authentication with Bearer token
- GCRA rate limiter with cost-aware token buckets
- Security headers middleware (CSP, X-Frame-Options, HSTS)
- Secret zeroization on all API key fields
- Subprocess environment isolation
- Health endpoint redaction (public minimal, auth full)
- Loop guard with SHA256-based detection and circuit breaker thresholds
- Session repair (validates and fixes orphaned tool results, empty messages)

#### Channels
- 40 channel adapters: Telegram, Discord, Slack, WhatsApp, Signal, Matrix, Email, Teams, Mattermost, Google Chat, Webex, Feishu/Lark, LINE, Viber, Facebook Messenger, Mastodon, Bluesky, Reddit, LinkedIn, Twitch, IRC, XMPP, and 18 more
- Unified bridge with agent routing, command handling, message splitting
- Per-channel user filtering and RBAC enforcement
- Graceful shutdown, exponential backoff, secret zeroization on all adapters

#### API
- 100+ REST/WS/SSE API endpoints (axum 0.8)
- WebSocket real-time streaming with per-agent connections
- OpenAI-compatible `/v1/chat/completions` API (streaming SSE + non-streaming)
- OpenAI-compatible `/v1/models` endpoint
- WebChat embedded UI with Alpine.js
- Google A2A protocol support (agent card, task send/get/cancel)
- Prometheus text-format `/api/metrics` endpoint for monitoring
- Multi-session management: list, create, switch, label sessions per agent
- Usage analytics: summary, by-model, daily breakdown
- Config hot-reload via polling (30-second interval, no restart required)

#### Web UI
- Chat message search with Ctrl+F, real-time filtering, text highlighting
- Voice input with hold-to-record mic button (WebM/Opus codec)
- TTS audio playback inline in tool cards
- Browser screenshot rendering in chat (inline images)
- Canvas rendering with iframe sandbox and CSP support
- Session switcher dropdown in chat header
- 6-step first-run setup wizard with provider API key help (12 providers)
- Skill marketplace with 4 tabs (Installed, ClawHub, MCP Servers, Quick Start)
- Copy-to-clipboard on messages, message timestamps
- Visual workflow builder with drag-and-drop canvas

#### Client SDKs
- JavaScript SDK (`@openfang/sdk`): full REST API client with streaming, TypeScript declarations
- Python client SDK (`openfang_client`): zero-dependency stdlib client with SSE streaming
- Python agent SDK (`openfang_sdk`): decorator-based framework for writing Python agents
- Usage examples for both languages (basic + streaming)

#### CLI
- 14+ subcommands: init, start, agent, workflow, trigger, migrate, skill, channel, config, chat, status, doctor, dashboard, mcp
- Daemon auto-detection via PID file
- Shell completion generation (bash, zsh, fish, PowerShell)
- MCP server mode for IDE integration

#### Skills Ecosystem
- 60 bundled skills across 14 categories
- Skill registry with TOML manifests
- 4 runtimes: Python, Node.js, WASM, PromptOnly
- FangHub marketplace with search/install
- ClawHub client for OpenClaw skill compatibility
- SKILL.md parser with auto-conversion
- SHA256 checksum verification
- Prompt injection scanning on skill content

#### Desktop App
- Tauri 2.0 native desktop app
- System tray with status and quick actions
- Single-instance enforcement
- Hide-to-tray on close
- Updated CSP for media, frame, and blob sources

#### Session Management
- LLM-based session compaction with token-aware triggers
- Multi-session per agent with named labels
- Session switching via API and UI
- Cross-channel canonical sessions
- Extended chat commands: `/new`, `/compact`, `/model`, `/stop`, `/usage`, `/think`

#### Image Support
- `ContentBlock::Image` with base64 inline data
- Media type validation (png, jpeg, gif, webp only)
- 5MB size limit enforcement
- Mapped to all 3 native LLM drivers

#### Usage Tracking
- Per-response cost estimation with model-aware pricing
- Usage footer in WebSocket responses and WebChat UI
- Usage events persisted to SQLite
- Quota enforcement with hourly windows

#### Interoperability
- OpenClaw migration engine (YAML/JSON5 to TOML)
- MCP client (JSON-RPC 2.0 over stdio/SSE, tool namespacing)
- MCP server (exposes OpenFang tools via MCP protocol)
- A2A protocol client and server
- Tool name compatibility mappings (21 OpenClaw tool names)

#### Infrastructure
- Multi-stage Dockerfile (debian:bookworm-slim runtime)
- docker-compose.yml with volume persistence
- GitHub Actions CI (check, test, clippy, format)
- GitHub Actions release (multi-platform, GHCR push, SHA256 checksums)
- Cross-platform install script (curl/irm one-liner)
- systemd service file for Linux deployment

#### Multi-User
- RBAC with Owner/Admin/User/Viewer roles
- Channel identity resolution
- Per-user authorization checks
- Device pairing and approval system

#### Production Readiness
- 1731+ tests across 15 crates, 0 failures
- Cross-platform support (Linux, macOS, Windows)
- Graceful shutdown with signal handling (SIGINT/SIGTERM on Unix, Ctrl+C on Windows)
- Daemon PID file with stale process detection
- Release profile with LTO, single codegen unit, symbol stripping
- Prometheus metrics for monitoring
- Config hot-reload without restart

[0.1.0]: https://github.com/RightNow-AI/openfang/releases/tag/v0.1.0
[0.6.6]: https://github.com/sbhooley/armaraos/releases/tag/v0.6.6
[0.6.5]: https://github.com/sbhooley/armaraos/releases/tag/v0.6.5
[0.6.4]: https://github.com/sbhooley/armaraos/releases/tag/v0.6.4
