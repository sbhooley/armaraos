# Changelog

All notable changes to OpenFang will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
