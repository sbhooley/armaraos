# OpenFang — Agent Instructions

## Project Overview
OpenFang is an open-source Agent Operating System written in Rust (14 crates).
- Config: `~/.armaraos/config.toml` (override: `ARMARAOS_HOME` / `OPENFANG_HOME`; legacy `~/.openfang` migrated automatically when possible)
- Default API: `http://127.0.0.1:4200`
- CLI binary: `target/release/openfang.exe` (or `target/debug/openfang.exe`)

**Dashboard chat (v0.6.4+):** Unread badges use WebSocket events, kernel SSE, and **`GET /api/agents/{id}/session/digest`** (lightweight counts). Leaving `#agents` may keep the agent WS alive with UI callbacks cleared (`wsClearUiCallbacks`). See **`docs/dashboard-testing.md`** (section *Chat unread badges + session digest*). Live check: `curl -s "$BASE/api/agents/$ID/session/digest"` after resolving `$ID` from `GET /api/agents`.

**Dashboard Get started (hash `#overview`, formerly “Overview”):** Sidebar section **Get started** sits **above Chat**; **Comms** is under **Monitor**. Page title matches; nav uses **`navigateOverview()`** so a second click on **Get started** while already on the page dispatches **`openfang-overview-nav-same-page`** and reveals **Setup Wizard** for users with **`localStorage`** **`openfang-onboarded`** **`true`**. **Quick actions** (agents, skills, **App Store** → `#ainl-library`, channels, workflows, settings, **Daemon & runtime** → `#runtime`) follow the optional **Live** SSE strip; **seven-tile** skeleton while loading — **`docs/dashboard-overview-ui.md`**. **Setup Wizard** header/checklist visibility: **`openfang-onboarded`**, **`overviewWizardCtaVisible`**, **Run setup again** — same doc. **`#wizard`** flow (steps, provider **Next** rules, flat `manifest_toml`, `wizard.js` embedded in **`webchat.rs`**): **`docs/dashboard-setup-wizard.md`**. **Setup checklist** (`overview.js`): **core** = provider + agent + schedule; **optional** = channel only for progress after core; **Chat** / **Skills** rows are **perpetual shortcuts** (always ○ + **Go**, never complete). **Dismiss** → **`of-checklist-dismissed`**. Removed: **`of-first-msg`**, **`of-skill-browsed`**, **`armaraos-onboarding-local`** checklist refresh. Manual QA → **`docs/dashboard-testing.md`** (*Get started page*, *Daemon lifecycle*). Internal probe agent names **`allowlist-probe`**, **`offline-cron`**, **`allow-ir-off`**: hidden from main sidebar list (`isInternalAutomationProbeChatAgentName` in **`js/app.js`**).

**Dashboard Settings / Runtime:** Polished headers (subtitles), Settings tab toolbar (`settings-page-tabs`), Runtime stat grid and panels — **`docs/dashboard-settings-runtime-ui.md`**. **Daemon / API:** **Reload config** (`POST /api/config/reload`), **Reload channels**, **Reload integrations**, **Shut down** (`POST /api/shutdown`, loopback may omit Bearer); **Check vs GitHub** uses **`GET /api/version/github-latest`** (server-side). Shared logic: **`static/js/daemon_lifecycle.js`** (included from **`webchat.rs`** after **`api.js`**); confirm modal options: **`OpenFangToast.confirm(..., { confirmLabel, danger })`**. **App Store** on-disk section title: **AI Native Lang Programs Available**.

**Support diagnostics + Home folder downloads:** Redacted zip: `POST /api/support/diagnostics`, then `GET /api/support/diagnostics/download?name=<bundle_filename>` (loopback may skip Bearer when `api_key` is set). Zip includes **`README.txt`**, **`diagnostics_snapshot.json`** (config schema effective+binary, paths, runtime, memory SQLite `user_version`), expanded **`meta.json`**, plus config, redacted secrets, audit, DB, logs. Full file under home: `GET /api/armaraos-home/download?path=support/…` (256 MiB cap; **read** preview stays 512 KiB). Desktop Tauri: **`copy_diagnostics_to_downloads`** `{ bundlePath }`, **`copy_home_file_to_downloads`** `{ relativePath }` — see **`docs/desktop.md`** (*IPC Commands*). **Settings** at-a-glance strip: config schema + API + log + home — **`docs/troubleshooting.md`** (*Config schema in the dashboard*), **`docs/dashboard-settings-runtime-ui.md`**. Home folder UI (View vs Download): **`docs/dashboard-home-folder.md`**. Manual QA: **`docs/dashboard-testing.md`** (*Support diagnostics bundle*, *Home folder browser — preview vs download*); smoke: **`./scripts/verify-dashboard-smoke.sh`**.

**Dashboard → Logs:** **Live** = audit SSE (`GET /api/logs/stream?level=&filter=`) + poll fallback; **Daemon** = CLI tracing file **`logs/daemon.log`** (else **`tui.log`**) via `GET /api/logs/daemon/recent` and **`GET /api/logs/daemon/stream`**; **Audit Trail** = chain UI. With `api_key` set, loopback may omit Bearer on those SSE routes (and `/api/events/stream`). Saving **`log_level`** (UI or `POST /api/config/set`) requires **daemon restart** for `tracing` to pick it up. Details: **`docs/dashboard-testing.md`** (*Logs page*), **`docs/api-reference.md`** (SSE section).

**Agent tools / loop guard:** `file_write` requires **`path` + `content`**; `shell_exec` requires **`command`**. Empty `{}` calls fail fast; repeated identical failures interact with **`loop_guard`** (`crates/openfang-runtime/src/loop_guard.rs`). Persist failures should not automatically trigger full re-acquisition — see **`docs/agent-automation-hardening.md`** and **`docs/troubleshooting.md`** (*Agent Issues*).

**Skills / ClawHub capture:** See `docs/openclaw-workspace-bridge.md` — **OpenClaw is not required**; `[skills_workspace]` or `[openclaw_workspace]`, `ARMARAOS_SKILLS_WORKSPACE` / `OPENCLAW_WORKSPACE`, default `~/.armaraos/skills-workspace`. Tray + startup digest only touch files (kernel does not load `.learnings/` into DB memory).

**Agents → Config (dashboard + API):** `GET /api/agents` and `GET /api/agents/{id}` return **`system_prompt`**, full **`identity`**, and (detail) **`tool_allowlist`** / **`tool_blocklist`**. **`PATCH /api/agents/{id}/config`** merges identity and skips empty prompt/description (see **`docs/api-reference.md`**). Tool filters: **`GET`/`PUT /api/agents/{id}/tools`**. Desktop: **`HealthCheckFailed`** is not toasted; ArmaraOS notification branding — **`docs/desktop.md`** (*Native OS Notifications*). Manual QA: **`docs/dashboard-testing.md`** (*Agents page → Config tab*).

**Chat UX features:** **Command palette** (Cmd/Ctrl+K) — full-window overlay searching pages, agents, actions, and recent sessions; JS in `static/js/pages/command-palette.js`, overlay HTML at top of `index_body.html`. **Pinned agents** — sidebar Quick-open rows show a hover pin button; pinned state shown as accent left border (never overlaps right-side status dot); order stored in `localStorage` `armaraos-pinned-agents`. **Chat input history** — ↑/↓ in empty input; per-agent, `localStorage` `armaraos-chat-history-<id>`. **Session rename** — click title to edit inline, Enter saves, Esc cancels. **Jump back in** — Quick-open ordered by last-activity from `localStorage` `armaraos-recent-agents`. **Chat/tool-call persistence** — module-level `_agentMsgCache` in `chat.js` survives component destruction and agent-switch within a page load; server SQLite is source of truth across upgrades. **`/btw` injection** — `POST /api/agents/{id}/btw` with `{"text":"…"}` enqueues context into a running loop mid-iteration; `/btw <text>` slash command in chat UI; returns 409 when idle. **Slash templates** — `/t save <name> <text>` / `/t <name>` / `/t list` / `/t delete <name>`; stored server-side in `~/.armaraos/slash-templates.json` via `GET`/`PUT /api/slash-templates` (atomic write). Manual QA: **`docs/dashboard-testing.md`** (*Command Palette*, *Chat UX — Sidebar & session features*, */btw*, *Slash templates*).

**Scheduled AINL + desktop AINL env:** Kernel injects **`AINL_ALLOW_IR_DECLARED_ADAPTERS=1`** on each scheduled **`ainl run`** child by default (manifest **`ainl_allow_ir_declared_adapters`** opt-out). Desktop sets the same when unset after dotenv. Detail route includes **`scheduled_ainl_host_adapter.ainl_allow_ir_declared_adapters`**. See **`docs/scheduled-ainl.md`**.

## Build & Verify Workflow
After every feature implementation, run ALL THREE checks:
```bash
cargo build --workspace --lib          # Must compile (use --lib if exe is locked)
cargo test --workspace                 # All tests must pass (currently 1744+)
cargo clippy --workspace --all-targets -- -D warnings  # Zero warnings
```

## Adding or changing built-in tools

1. Add a `match` arm in `crates/openfang-runtime/src/tool_runner.rs` (`execute_tool`).
2. Add `ToolDefinition` in `builtin_tool_definitions()` (same file).
3. Pass `ainl_library_root` into `execute_tool` from the agent loop and API (`routes.rs` MCP bridge) — read tools (`file_read`, `file_list`, `document_extract`) use `resolve_file_path_read` so `ainl-library/...` paths work.
4. Register in `openfang-types/src/tool_compat.rs` (`is_known_openfang_tool`) if the name should normalize as a first-class tool.
5. Timeouts: `agent_loop.rs` `tool_timeout_for` for slow tools; approval: `openfang-kernel/src/approval.rs` for writes.
6. Run `cargo test -p openfang-runtime` (includes `test_builtin_tool_names_unique` and dispatch smoke).

CI already runs `cargo check`, `cargo test --workspace`, `cargo clippy -D warnings`, and `cargo fmt --check` on push/PR.

## MANDATORY: Live Integration Testing
**After implementing any new endpoint, feature, or wiring change, you MUST run live integration tests.** Unit tests alone are not enough — they can pass while the feature is actually dead code. Live tests catch:
- Missing route registrations in server.rs
- Config fields not being deserialized from TOML
- Type mismatches between kernel and API layers
- Endpoints that compile but return wrong/empty data

### How to Run Live Integration Tests

#### Step 1: Stop any running daemon
```bash
tasklist | grep -i openfang
taskkill //PID <pid> //F
# Wait 2-3 seconds for port to release
sleep 3
```

#### Step 2: Build fresh release binary
```bash
cargo build --release -p openfang-cli
```

#### Step 3: Start daemon with required API keys
```bash
GROQ_API_KEY=<key> target/release/openfang.exe start &
sleep 6  # Wait for full boot
curl -s http://127.0.0.1:4200/api/health  # Verify it's up
```
The daemon command is `start` (not `daemon`).

#### Step 4: Test every new endpoint
```bash
# GET endpoints — verify they return real data, not empty/null
curl -s http://127.0.0.1:4200/api/<new-endpoint>

# POST/PUT endpoints — send real payloads
curl -s -X POST http://127.0.0.1:4200/api/<endpoint> \
  -H "Content-Type: application/json" \
  -d '{"field": "value"}'

# Verify write endpoints persist — read back after writing
curl -s -X PUT http://127.0.0.1:4200/api/<endpoint> -d '...'
curl -s http://127.0.0.1:4200/api/<endpoint>  # Should reflect the update
```

#### Step 5: Test real LLM integration
```bash
# Get an agent ID
curl -s http://127.0.0.1:4200/api/agents | python3 -c "import sys,json; print(json.load(sys.stdin)[0]['id'])"

# Send a real message (triggers actual LLM call to Groq/OpenAI)
curl -s -X POST "http://127.0.0.1:4200/api/agents/<id>/message" \
  -H "Content-Type: application/json" \
  -d '{"message": "Say hello in 5 words."}'
```

#### Step 6: Verify side effects
After an LLM call, verify that any metering/cost/usage tracking updated:
```bash
curl -s http://127.0.0.1:4200/api/budget       # Cost should have increased
curl -s http://127.0.0.1:4200/api/budget/agents  # Per-agent spend should show
```

#### Step 7: Verify dashboard HTML
```bash
# Check that new UI components exist in the served HTML
curl -s http://127.0.0.1:4200/ | grep -c "newComponentName"
# Should return > 0
```

#### Step 8: Cleanup
```bash
tasklist | grep -i openfang
taskkill //PID <pid> //F
```

### Key API Endpoints for Testing
| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/api/health` | GET | Basic health check |
| `/api/agents` | GET | List all agents |
| `/api/agents/{id}/message` | POST | Send message (triggers LLM) |
| `/api/budget` | GET/PUT | Global budget status/update |
| `/api/budget/agents` | GET | Per-agent cost ranking |
| `/api/budget/agents/{id}` | GET | Single agent budget detail |
| `/api/network/status` | GET | OFP network status |
| `/api/peers` | GET | Connected OFP peers |
| `/api/a2a/agents` | GET | External A2A agents |
| `/api/a2a/discover` | POST | Discover A2A agent at URL |
| `/api/a2a/send` | POST | Send task to external A2A agent |
| `/api/a2a/tasks/{id}/status` | GET | Check external A2A task status |

## Architecture Notes
- **Don't touch `openfang-cli`** — user is actively building the interactive CLI
- `KernelHandle` trait avoids circular deps between runtime and kernel
- `AppState` in `server.rs` bridges kernel to API routes
- New routes must be registered in `server.rs` router AND implemented in `routes.rs`
- Dashboard is Alpine.js SPA in `static/index_body.html` — new tabs need both HTML and JS data/methods
- **Overview (Get started):** Quick actions live at the top of the overview body (after the live SSE strip); see **`docs/dashboard-overview-ui.md`** for section order, visibility (`!loadError && !loading`), Setup Wizard gating, and CSS classes
- Config fields need: struct field + `#[serde(default)]` + Default impl entry + Serialize/Deserialize derives

## Common Gotchas
- `openfang.exe` may be locked if daemon is running — use `--lib` flag or kill daemon first
- `PeerRegistry` is `Option<PeerRegistry>` on kernel but `Option<Arc<PeerRegistry>>` on `AppState` — wrap with `.as_ref().map(|r| Arc::new(r.clone()))`
- Config fields added to `KernelConfig` struct MUST also be added to the `Default` impl or build fails
- `AgentLoopResult` field is `.response` not `.response_text`
- CLI command to start daemon is `start` not `daemon`
- On Windows: use `taskkill //PID <pid> //F` (double slashes in MSYS2/Git Bash)
