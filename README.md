<p align="center">
  <img src="public/assets/armaraos-logo.png" width="160" alt="ArmaraOS Logo" />
</p>

<h1 align="center">ArmaraOS</h1>
<h3 align="center">The Agent Operating System</h3>

<p align="center">
  Open-source Agent OS built in Rust. 137K LOC. 15 library crates + <code>xtask</code> (16 workspace members). 1,767+ tests. Zero clippy warnings.<br/>
  <strong>One binary. Battle-tested. Agents that actually work for you.</strong>
</p>

<p align="center">
  <a href="https://github.com/sbhooley/armaraos">Repository</a> &bull;
  <a href="docs/getting-started.md">Quick Start</a> &bull;
  <a href="docs/docker.md">Docker</a> &bull;
  <a href="ARCHITECTURE.md">Architecture</a> &bull;
  <a href="docs/graph-memory.md">Graph memory (runtime)</a> &bull;
  <a href="PRIOR_ART.md">Prior art (graph memory)</a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/language-Rust-orange?style=flat-square" alt="Rust" />
  <img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="MIT" />
  <img src="https://img.shields.io/badge/version-0.7.2-green?style=flat-square" alt="v0.7.2" />
  <img src="https://img.shields.io/badge/tests-1,767%2B%20passing-brightgreen?style=flat-square" 
</p>

---

> **ArmaraOS (March 2026)**
>
> ArmaraOS is a fork/rebrand of the upstream OpenFang project. Upstream license/attribution remains in place where applicable. You may encounter rough edges or breaking changes between minor versions. Pin to a specific commit for production use until v1.0.

---

## What is ArmaraOS?

ArmaraOS is an **open-source Agent Operating System** — not a chatbot framework, not a Python wrapper around an LLM, not a "multi-agent orchestrator." It is a full operating system for autonomous agents, built from scratch in Rust.

Traditional agent frameworks wait for you to type something. ArmaraOS runs **autonomous agents that work for you** — on schedules, 24/7, building knowledge graphs, monitoring targets, generating leads, managing your social media, and reporting results to your dashboard.

The entire system compiles to a **single ~32MB binary**. One install, one command, your agents are live.

**Implementation language (programs & apps):** [**AI Native Language (AINL)**](docs/ainl-first-language.md) is the **default, first-class** language for new automation, workflows, extensions, and apps in this ecosystem. Use another modern language (Rust, Python, TypeScript, …) only when **explicitly** requested or when constraints require it; the kernel and core services remain Rust. See the linked doc for the full policy.

```bash
# From GitHub directly (recommended until you host a vanity domain):
curl -fsSL https://raw.githubusercontent.com/sbhooley/armaraos/main/scripts/install.sh | bash
armaraos init
armaraos start
# Dashboard live at http://localhost:4200
```

<details>
<summary><strong>Windows</strong></summary>

```powershell
# From GitHub directly (recommended until you host a vanity domain):
irm https://raw.githubusercontent.com/sbhooley/armaraos/main/scripts/install.ps1 | iex
armaraos init
armaraos start
```

</details>

**Config and local data** default to **`~/.armaraos/`** (overrides: **`ARMARAOS_HOME`**, legacy **`OPENFANG_HOME`**; older **`~/.openfang/`** is migrated automatically when possible). Details: [`docs/data-directory.md`](docs/data-directory.md).

**Desktop installers (GUI app):** Builds are attached to [GitHub Releases](https://github.com/sbhooley/armaraos/releases). After each stable tag, CI can mirror **`latest.json`** and binaries to **[ainativelang.com](https://ainativelang.com)** under **`/downloads/armaraos/`**; the marketing homepage and **`/download`** page list installers from that manifest (see [`docs/release-desktop.md`](docs/release-desktop.md)). **Routine version bumps and tags:** [`docs/RELEASING.md`](docs/RELEASING.md). For **code signing**, Gatekeeper, and SmartScreen (vs Tauri updater keys), see [`docs/desktop-code-signing.md`](docs/desktop-code-signing.md).

### Desktop app — anonymous product analytics (PostHog)

Official **desktop** builds may include a **project API key** for [PostHog](https://posthog.com) **baked in at compile time** (same kind of key as `NEXT_PUBLIC_POSTHOG_KEY` on the marketing site). The app sends **at most one** anonymous event per machine (`armaraos_desktop_first_open`: app version, OS, architecture) to estimate installs. **No** chat content, API keys, or agent data are included.

- **Opt out:** In the embedded dashboard **Setup Wizard** (step 1 — Welcome), uncheck **Anonymous usage** before continuing. If you opt out, the app **does not** open network connections for this ping. A small preference file is stored under the app data directory (alongside other desktop state).
- **Timing:** If you leave usage enabled, the ping runs after you continue past Welcome **or** after **about two minutes** if you never open the wizard (so silent launches are still counted only when allowed).
- **Overrides:** Power users can still set **`ARMARAOS_PRODUCT_ANALYTICS=0`** to disable completely, or **`ARMARAOS_POSTHOG_KEY`** / **`ARMARAOS_POSTHOG_HOST`** at **runtime** to override the baked values (e.g. local debugging).
- **Releases:** Maintainers set **`ARMARAOS_POSTHOG_KEY`** (or org secret **`AINL_POSTHOG_KEY`**, same value as **`NEXT_PUBLIC_POSTHOG_KEY`** on ainativelangweb) and optionally **`ARMARAOS_POSTHOG_HOST`** or **`AINL_POSTHOG_HOST`** so release builds embed the key — **end users do not set environment variables** for normal installs.

---

## AINL program library & cron

The desktop app syncs upstream **`demo/`**, **`examples/`**, and **`intelligence/`** from [`sbhooley/ainativelang`](https://github.com/sbhooley/ainativelang) into **`~/.armaraos/ainl-library/`** (alongside config under `~/.armaraos/`). The kernel **embeds** the repo `programs/` tree and materializes it to **`ainl-library/armaraos-programs/`** on boot (safe, no collision with upstream). Scheduled jobs can run **AINL** programs via the real cron store (`/api/cron/jobs`), not the legacy KV schedules API.

| Topic | Details |
|-------|---------|
| **Binary** | `ainl` on `PATH`, or **`ARMARAOS_AINL_BIN`**, or (desktop internal venv) **`~/.armaraos/.armaraos-ainl-bin`** written by the app when AINL is healthy. If the OS reports “not found”, install AINL or set **`ARMARAOS_AINL_BIN`**. |
| **Structured output** | Cron actions of kind `ainl_run` support **`json_output: true`**, which runs `ainl run --json …` and pretty-prints JSON for delivery/webhooks. |
| **HTTP API** | `GET /api/ainl/library` — scan `.ainl` / `.lang` files; `GET /api/ainl/library/curated` — embedded curated catalog; `POST /api/ainl/library/register-curated` — idempotent registration (rate-limited). |
| **Dashboard** | **Scheduler** page lists job **type** (Agent / AINL / Workflow / Event) and can create agent-turn, AINL, or workflow cron jobs. |
| **Curated templates** | On boot, missing catalog entries are registered idempotently. A **safe** weekly health job (`core` only) is **enabled** by default; opt-in jobs and upstream examples are **disabled** until you enable them. |
| **Secrets & scheduled graphs** | Scheduled `ainl run` uses the **same** credential resolver as the daemon (**vault → `~/.armaraos/.env` → env**); optional per-agent **`metadata.ainl_host_adapter_allowlist`**. Details: [`docs/scheduled-ainl.md`](docs/scheduled-ainl.md). |

See also: [`docs/ainl-first-language.md`](docs/ainl-first-language.md), [`docs/ootb-ainl.md`](docs/ootb-ainl.md), manual smoke [`docs/ootb-ainl-smoke.md`](docs/ootb-ainl-smoke.md), [`docs/scheduled-ainl.md`](docs/scheduled-ainl.md).

### Dashboard: kernel SSE (live events)

The web dashboard opens **`GET /api/events/stream`** (Server-Sent Events) for the kernel event bus. Each message updates `Alpine.store('kernelEvents').last` and dispatches a window event **`armaraos-kernel-event`**. The **Get started** page (sidebar label; internal route/hash **`#overview`**) listens for lifecycle/system events and **debounces** a silent data refresh (~400ms) so stats stay current after spawns, crashes, quota events, etc., without a full reload.

**Auth:** If `api_key` is set in config, clients on **loopback** (127.0.0.1 / ::1) may connect to the stream without a token (embedded UI). **Non-loopback** clients must use the same authentication as the rest of the API (`Authorization: Bearer` with the configured key, or a `token` query parameter). See [`docs/dashboard-testing.md`](docs/dashboard-testing.md) for manual checks; [`docs/release-desktop.md`](docs/release-desktop.md) for desktop smoke (Tauri + SSE badge).

### Dashboard: notification center (bell)

A fixed **bell** (top-right) opens a panel of **persistent** rows: pending **`/api/approvals`**, budget threshold from **`/api/budget`**, and selected kernel events from the same **`/api/events/stream`** used for the sidebar SSE badge. Command palette (**Cmd/Ctrl+K**) → **Notifications** opens the panel; layout reserves space via **`--notify-bell-reserve`** so page headers do not sit under the bell. Details, a11y, and smoke commands: [`docs/dashboard-testing.md`](docs/dashboard-testing.md#notification-center-bell).

### Dashboard: diagnostics bundle (support)

- **`POST /api/support/diagnostics`** generates a **redacted `.zip`** under `~/.armaraos/support/` (includes `config.toml`, redacted `secrets.env`, `audit.json`, SQLite DB + WAL/SHM when present, recent logs, and `meta.json`).
- **`GET /api/support/diagnostics/download?name=…`** streams that zip for save/export; on **loopback**, both POST and GET work without Bearer when `api_key` is set (same idea as SSE — embedded local UI).
- **Home folder:** **`GET /api/armaraos-home/download?path=…`** streams any home-relative file up to **256 MiB** (large zips); **`GET /api/armaraos-home/read`** remains a **512 KiB** preview only.
- The dashboard can **generate + copy** a single pasteable block (bundle path + connection context). **Settings → System Info → Support** (or the desktop Help menu) creates the archive; **desktop** copies to **Downloads** via Tauri; **Home folder → support** offers **Download** when you need the full file after a failed preview.

### JSON error shape (REST integrators)

Many endpoints return a consistent JSON body on **4xx/5xx** (best-effort across the API; high-traffic routes are covered first):

| Field | Meaning |
|-------|---------|
| `error` | Short error code / title |
| `detail` | What failed (human-readable) |
| `path` | Logical route (e.g. `/api/agents`, `/api/workflows/:id/run`) |
| `request_id` | Same value as the **`x-request-id`** response header (correlation for logs) |
| `hint` | Optional recovery hint |

Example — **`POST /api/agents`** with an empty body / missing manifest:

```json
{
  "error": "Missing manifest",
  "detail": "Either 'manifest_toml' or 'template' is required in the JSON body.",
  "path": "/api/agents",
  "request_id": "<uuid>",
  "hint": "Paste a manifest TOML or set template to a folder name under ~/.armaraos/agents/."
}
```

---

## Hands: Agents That Actually Do Things

<p align="center"><em>"Traditional agents wait for you to type. Hands work <strong>for</strong> you."</em></p>

**Hands** are ArmaraOS's core innovation — pre-built autonomous capability packages that run independently, on schedules, without you having to prompt them. This is not a chatbot. This is an agent that wakes up at 6 AM, researches your competitors, builds a knowledge graph, scores the findings, and delivers a report to your Telegram before you've had coffee.

Each Hand bundles:
- **HAND.toml** — Manifest declaring tools, settings, requirements, and dashboard metrics
- **System Prompt** — Multi-phase operational playbook (not a one-liner — these are 500+ word expert procedures)
- **SKILL.md** — Domain expertise reference injected into context at runtime
- **Guardrails** — Approval gates for sensitive actions (e.g. Browser Hand requires approval before any purchase)

All compiled into the binary. No downloading, no pip install, no Docker pull.

### The 7 Bundled Hands

| Hand | What It Actually Does |
|------|----------------------|
| **Clip** | Takes a YouTube URL, downloads it, identifies the best moments, cuts them into vertical shorts with captions and thumbnails, optionally adds AI voice-over, and publishes to Telegram and WhatsApp. 8-phase pipeline. FFmpeg + yt-dlp + 5 STT backends. |
| **Lead** | Runs daily. Discovers prospects matching your ICP, enriches them with web research, scores 0-100, deduplicates against your existing database, and delivers qualified leads in CSV/JSON/Markdown. Builds ICP profiles over time. |
| **Collector** | OSINT-grade intelligence. You give it a target (company, person, topic). It monitors continuously — change detection, sentiment tracking, knowledge graph construction, and critical alerts when something important shifts. |
| **Predictor** | Superforecasting engine. Collects signals from multiple sources, builds calibrated reasoning chains, makes predictions with confidence intervals, and tracks its own accuracy using Brier scores. Has a contrarian mode that deliberately argues against consensus. |
| **Researcher** | Deep autonomous researcher. Cross-references multiple sources, evaluates credibility using CRAAP criteria (Currency, Relevance, Authority, Accuracy, Purpose), generates cited reports with APA formatting, supports multiple languages. |
| **Twitter** | Autonomous Twitter/X account manager. Creates content in 7 rotating formats, schedules posts for optimal engagement, responds to mentions, tracks performance metrics. Has an approval queue — nothing posts without your OK. |
| **Browser** | Web automation agent. Navigates sites, fills forms, clicks buttons, handles multi-step workflows. Uses Playwright bridge with session persistence. **Mandatory purchase approval gate** — it will never spend your money without explicit confirmation. |

```bash
# Activate the Researcher Hand — it starts working immediately
armaraos hand activate researcher

# Check its progress anytime
armaraos hand status researcher

# Activate lead generation on a daily schedule
armaraos hand activate lead

# Pause without losing state
armaraos hand pause lead

# See all available Hands
armaraos hand list
```

**Build your own.** Define a `HAND.toml` with tools, settings, and a system prompt. Publish to FangHub.

---

## ArmaraOS vs The Landscape

<p align="center">
  <img src="public/assets/openfang-vs-claws.png" width="600" alt="ArmaraOS vs OpenClaw vs ZeroClaw" />
</p>

### Benchmarks: Measured, Not Marketed

All data from official documentation and public repositories — February 2026.

#### Cold Start Time (lower is better)

```
ZeroClaw   ██░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░   10 ms
ArmaraOS   ██████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  180 ms    ★
LangGraph  █████████████████░░░░░░░░░░░░░░░░░░░░░░░░░  2.5 sec
CrewAI     ████████████████████░░░░░░░░░░░░░░░░░░░░░░  3.0 sec
AutoGen    ██████████████████████████░░░░░░░░░░░░░░░░░  4.0 sec
OpenClaw   █████████████████████████████████████████░░  5.98 sec
```

#### Idle Memory Usage (lower is better)

```
ZeroClaw   █░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░    5 MB
ArmaraOS   ████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░   40 MB    ★
LangGraph  ██████████████████░░░░░░░░░░░░░░░░░░░░░░░░░  180 MB
CrewAI     ████████████████████░░░░░░░░░░░░░░░░░░░░░░░  200 MB
AutoGen    █████████████████████████░░░░░░░░░░░░░░░░░░  250 MB
OpenClaw   ████████████████████████████████████████░░░░  394 MB
```

#### Install Size (lower is better)

```
ZeroClaw   █░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  8.8 MB
ArmaraOS   ███░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░   32 MB    ★
CrewAI     ████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  100 MB
LangGraph  ████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  150 MB
AutoGen    ████████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░  200 MB
OpenClaw   ████████████████████████████████████████░░░░  500 MB
```

#### Security Systems (higher is better)

```
ArmaraOS   ████████████████████████████████████████████   16      ★
ZeroClaw   ███████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░    6
OpenClaw   ████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░    3
AutoGen    █████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░    2
LangGraph  █████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░    2
CrewAI     ███░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░    1
```

#### Channel Adapters (higher is better)

```
ArmaraOS   ████████████████████████████████████████████   40      ★
ZeroClaw   ███████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░   15
OpenClaw   █████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░   13
CrewAI     ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░    0
AutoGen    ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░    0
LangGraph  ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░    0
```

#### LLM Providers (higher is better)

```
ZeroClaw   ████████████████████████████████████████████   28
ArmaraOS   ██████████████████████████████████████████░░   27      ★
LangGraph  ██████████████████████░░░░░░░░░░░░░░░░░░░░░   15
CrewAI     ██████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░   10
OpenClaw   ██████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░   10
AutoGen    ███████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░    8
```

### Feature-by-Feature Comparison

| Feature | ArmaraOS | OpenClaw | ZeroClaw | CrewAI | AutoGen | LangGraph |
|---------|----------|----------|----------|--------|---------|-----------|
| **Language** | **Rust** | TypeScript | **Rust** | Python | Python | Python |
| **Autonomous Hands** | **7 built-in** | None | None | None | None | None |
| **Security Layers** | **16 discrete** | 3 basic | 6 layers | 1 basic | Docker | AES enc. |
| **Agent Sandbox** | **WASM dual-metered** | None | Allowlists | None | Docker | None |
| **Channel Adapters** | **40** | 13 | 15 | 0 | 0 | 0 |
| **Built-in Tools** | **53 + MCP + A2A** | 50+ | 12 | Plugins | MCP | LC tools |
| **Memory** | **SQLite + vector** | File-based | SQLite FTS5 | 4-layer | External | Checkpoints |
| **Desktop App** | **Tauri 2.0** | None | None | None | Studio | None |
| **Audit Trail** | **Merkle hash-chain** | Logs | Logs | Tracing | Logs | Checkpoints |
| **Cold Start** | **<200ms** | ~6s | ~10ms | ~3s | ~4s | ~2.5s |
| **Install Size** | **~32 MB** | ~500 MB | ~8.8 MB | ~100 MB | ~200 MB | ~150 MB |
| **License** | MIT | MIT | MIT | MIT | Apache 2.0 | MIT |

---

## 16 Security Systems — Defense in Depth

ArmaraOS doesn't bolt security on after the fact. Every layer is independently testable and operates without a single point of failure.

| # | System | What It Does |
|---|--------|-------------|
| 1 | **WASM Dual-Metered Sandbox** | Tool code runs in WebAssembly with fuel metering + epoch interruption. A watchdog thread kills runaway code. |
| 2 | **Merkle Hash-Chain Audit Trail** | Every action is cryptographically linked to the previous one. Tamper with one entry and the entire chain breaks. |
| 3 | **Information Flow Taint Tracking** | Labels propagate through execution — secrets are tracked from source to sink. |
| 4 | **Ed25519 Signed Agent Manifests** | Every agent identity and capability set is cryptographically signed. |
| 5 | **SSRF Protection** | Blocks private IPs, cloud metadata endpoints, and DNS rebinding attacks. |
| 6 | **Secret Zeroization** | `Zeroizing<String>` auto-wipes API keys from memory the instant they're no longer needed. |
| 7 | **OFP Mutual Authentication** | HMAC-SHA256 nonce-based, constant-time verification for P2P networking. |
| 8 | **Capability Gates** | Role-based access control — agents declare required tools, the kernel enforces it. |
| 9 | **Security Headers** | CSP, X-Frame-Options, HSTS, X-Content-Type-Options on every response. |
| 10 | **Health Endpoint Redaction** | Public health check returns minimal info. Full diagnostics require authentication. |
| 11 | **Subprocess Sandbox** | `env_clear()` + selective variable passthrough. Process tree isolation with cross-platform kill. |
| 12 | **Prompt Injection Scanner** | Detects override attempts, data exfiltration patterns, and shell reference injection in skills. |
| 13 | **Loop Guard** | SHA256-based tool call loop detection with circuit breaker. Handles ping-pong patterns. |
| 14 | **Session Repair** | 7-phase message history validation and automatic recovery from corruption. |
| 15 | **Path Traversal Prevention** | Canonicalization with symlink escape prevention. `../` doesn't work here. |
| 16 | **GCRA Rate Limiter** | Cost-aware token bucket rate limiting with per-IP tracking and stale cleanup. |

---

## Architecture

14 Rust crates. 137,728 lines of code. Modular kernel design.

```
openfang-kernel      Orchestration, workflows, metering, RBAC, scheduler, budget tracking
openfang-runtime     Agent loop, 3 LLM drivers, 53 tools, WASM sandbox, MCP, A2A
openfang-api         140+ REST/WS/SSE endpoints, OpenAI-compatible API, dashboard
openfang-channels    40 messaging adapters with rate limiting, DM/group policies
openfang-memory      SQLite persistence, vector embeddings, canonical sessions, compaction
openfang-types       Core types, taint tracking, Ed25519 manifest signing, model catalog
openfang-skills      60 bundled skills, SKILL.md parser, FangHub marketplace
openfang-hands       7 autonomous Hands, HAND.toml parser, lifecycle management
openfang-extensions  25 MCP templates, AES-256-GCM credential vault, OAuth2 PKCE
openfang-wire        OFP P2P protocol with HMAC-SHA256 mutual authentication
openfang-cli         CLI with daemon management, TUI dashboard, MCP server mode
openfang-desktop     Tauri 2.0 native app (system tray, notifications, global shortcuts)
openfang-migrate     OpenClaw, LangChain, AutoGPT migration engine
xtask                Build automation
```

### Desktop app and AINL

The Tauri desktop shell can bootstrap an internal Python venv, install **AINL** from a **bundled wheel** (`crates/openfang-desktop/resources/ainl/`) or **PyPI** (see `ARMARAOS_AINL_PYPI_SPEC`), and register **`ainl-mcp`** in **`~/.armaraos/config.toml`** in line with `ainl install armaraos`. **Settings → AINL** (visible only in the desktop app) shows live status and retry actions.

- **Bundle resources for release or local smoke:** `cargo xtask bundle-ainl-wheel` and `cargo xtask bundle-portable-python --target <rust-triple>` (see `crates/openfang-desktop/resources/python/README.md`). CI bundles and verifies Linux resources; the release workflow bundles per matrix target.
- **Overrides:** `ARMARAOS_PYTHON`, `ARMARAOS_AINL_PYPI_SPEC`, and standard pip env vars such as `PIP_INDEX_URL` for private indexes.

Short manual checklist: [docs/DESKTOP_AINL_SMOKE.md](docs/DESKTOP_AINL_SMOKE.md).

---

## 40 Channel Adapters

Connect your agents to every platform your users are on.

**Core:** Telegram, Discord, Slack, WhatsApp, Signal, Matrix, Email (IMAP/SMTP)
**Enterprise:** Microsoft Teams, Mattermost, Google Chat, Webex, Feishu/Lark, Zulip
**Social:** LINE, Viber, Facebook Messenger, Mastodon, Bluesky, Reddit, LinkedIn, Twitch
**Community:** IRC, XMPP, Guilded, Revolt, Keybase, Discourse, Gitter
**Privacy:** Threema, Nostr, Mumble, Nextcloud Talk, Rocket.Chat, Ntfy, Gotify
**Workplace:** Pumble, Flock, Twist, DingTalk, Zalo, Webhooks

Each adapter supports per-channel model overrides, DM/group policies, rate limiting, and output formatting.

---

## WhatsApp Web Gateway (QR Code)

Connect your personal WhatsApp account to ArmaraOS via QR code — just like WhatsApp Web. No Meta Business account required.

### Prerequisites

- **Node.js >= 18** installed ([download](https://nodejs.org/))
- ArmaraOS installed and initialized

### Setup

**1. Install the gateway dependencies:**

```bash
cd packages/whatsapp-gateway
npm install
```

**2. Configure `config.toml`:**

```toml
[channels.whatsapp]
mode = "web"
default_agent = "assistant"
```

**3. Set the gateway URL (choose one):**

Add to your shell profile for persistence:

```bash
# macOS / Linux
echo 'export WHATSAPP_WEB_GATEWAY_URL="http://127.0.0.1:3009"' >> ~/.zshrc
source ~/.zshrc
```

Or set it inline when starting the gateway:

```bash
export WHATSAPP_WEB_GATEWAY_URL="http://127.0.0.1:3009"
```

**4. Start the gateway:**

```bash
node packages/whatsapp-gateway/index.js
```

The gateway listens on port `3009` by default. Override with `WHATSAPP_GATEWAY_PORT`.

**5. Start ArmaraOS:**

```bash
armaraos start
# Dashboard at http://localhost:4200
```

> If you built from source and only have an `openfang` binary, `openfang start` is equivalent.

**6. Scan the QR code:**

Open the dashboard → **Channels** → **WhatsApp**. A QR code will appear. Scan it with your phone:

> **WhatsApp** → **Settings** → **Linked Devices** → **Link a Device**

Once scanned, the status changes to `connected` and incoming messages are routed to your configured agent.

### Gateway Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `WHATSAPP_WEB_GATEWAY_URL` | Gateway URL for ArmaraOS to connect to | _(empty = disabled)_ |
| `WHATSAPP_GATEWAY_PORT` | Port the gateway listens on | `3009` |
| `OPENFANG_URL` | ArmaraOS API URL the gateway reports to (legacy name) | `http://127.0.0.1:4200` |
| `OPENFANG_DEFAULT_AGENT` | Agent that handles incoming messages (legacy name) | `assistant` |

### Gateway API Endpoints

| Method | Route | Description |
|--------|-------|-------------|
| `POST` | `/login/start` | Generate QR code (returns base64 PNG) |
| `GET` | `/login/status` | Connection status (`disconnected`, `qr_ready`, `connected`) |
| `POST` | `/message/send` | Send a message (`{ "to": "5511999999999", "text": "Hello" }`) |
| `GET` | `/health` | Health check |

### Alternative: WhatsApp Cloud API

For production workloads, use the [WhatsApp Cloud API](https://developers.facebook.com/docs/whatsapp/cloud-api) with a Meta Business account.



---

## Ultra Cost-Efficient Mode

ArmaraOS compresses **user input** in Rust before each LLM call (typical **40–56 %** input savings on conversational text in **Balanced** mode; **Aggressive** targets higher savings with a wider gap on prose than on dense opcode-heavy prompts). End-to-end latency target: **under 30 ms**.

**Full reference:** [docs/prompt-compression-efficient-mode.md](docs/prompt-compression-efficient-mode.md) (preserve rules, API fields, Eco Diff, tests).

### Configuration

In `~/.armaraos/config.toml`:

```toml
# "balanced" (default) | "aggressive" | "off"
efficient_mode = "balanced"
```

**Dashboard:** **Settings → Budget** — card **Ultra Cost-Efficient Mode** (dropdown + guidance). **While chatting:** header button **⚡ eco** / **⚡ eco bal** / **⚡ eco agg** cycles modes and persists globally.

Per-agent override (manifest **metadata** wins over global):

```toml
# In the agent manifest — example fragment
[metadata]
efficient_mode = "off"
```

### Before / After Example

**Category:** Verbose support question about ArmaraOS dashboard agent errors (~85 tokens).  
This is the most common pattern that gains 40–55 %: natural-language questions padded with hedging words and politeness filler.

**Before** (what the user typed):
> *"I think I would like to understand basically why the dashboard is showing me a red error badge on the agents page. Essentially, it seems like the agent is not responding and I'm not sure what steps I should take to investigate this issue. Please note that I have already tried restarting the daemon. To be honest, I'm not really sure where to look next."*

**After** balanced compression (59 tokens, ↓34% — live value shown as `⚡ eco ↓34%` in chat):
> *"Understand why the dashboard is showing me a red error badge on the agents page. it seems like the agent is not responding and I am not sure what steps I should take to investigate this issue. I have already tried restarting the daemon"*

All critical context (error badge, agents page, daemon restart, investigation intent) is preserved verbatim.  
Filler stripped: `I think I would like to`, ` basically `, `Essentially,`, `Please note that`, `To be honest,`.  
The user's response is unchanged — only the LLM-bound copy is compressed.  
Users continue typing normally; ArmaraOS handles compression transparently behind the scenes.

> **Note:** This exact output is verified by `cargo test -p openfang-runtime -- prompt_compressor::tests::readme_dashboard_example_ratio`.

### Benchmarks

Tested on a 200-word React debugging question (typical real-world prompt):

```
Original:       ~67 tokens   ($0.000201 @ Sonnet 4.6)
Balanced:       ~37 tokens   ($0.000111 @ Sonnet 4.6)  — 45 % reduction
Aggressive:     ~27 tokens   ($0.000081 @ Sonnet 4.6)  — 60 % reduction
Code preserved: 100 % verbatim
Intent preserved: ✓ React, state, useEffect, re-render, infinite loop
Latency:        under 30 ms (hot path; often much less on short prompts)
```

Live savings are logged at `INFO` level (`prompt:compressed`) with structured fields such as `savings_pct` and optional `est_savings_usd` (see runtime for exact keys).

### Dashboard telemetry

When compression runs, the chat meta can show **`⚡ eco ↓X%`** and a **diff** control opens the **Eco Diff** modal (original vs compressed prompt). HTTP and WebSocket responses may include **`compression_savings_pct`** and **`compressed_input`** (see [API reference](docs/api-reference.md)).

---

## 27 LLM Providers — 123+ Models

3 native drivers (Anthropic, Gemini, OpenAI-compatible) route to 27 providers:

Anthropic, Gemini, OpenAI, Groq, DeepSeek, OpenRouter, Together, Mistral, Fireworks, Cohere, Perplexity, xAI, AI21, Cerebras, SambaNova, HuggingFace, Replicate, Ollama, vLLM, LM Studio, Qwen, MiniMax, Zhipu, Moonshot, Qianfan, Bedrock, and more.

Intelligent routing with task complexity scoring, automatic fallback, cost tracking, and per-model pricing.

---

## Migrate from OpenClaw

Already running OpenClaw? One command:

```bash
# Migrate everything — agents, memory, skills, configs
armaraos migrate --from openclaw

# Migrate from a specific path
armaraos migrate --from openclaw --path ~/.openclaw

# Dry run first to see what would change
armaraos migrate --from openclaw --dry-run
```

The migration engine imports your agents, conversation history, skills, and configuration. ArmaraOS reads SKILL.md natively and is compatible with the ClawHub marketplace.

---

## OpenAI-Compatible API

Drop-in replacement. Point your existing tools at ArmaraOS:

```bash
curl -X POST localhost:4200/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "researcher",
    "messages": [{"role": "user", "content": "Analyze Q4 market trends"}],
    "stream": true
  }'
```

140+ REST/WS/SSE endpoints covering agents, memory, workflows, channels, models, skills, A2A, Hands, and more.

---

## Quick Start

```bash
# 1. Install (macOS/Linux)
curl -fsSL https://raw.githubusercontent.com/sbhooley/armaraos/main/scripts/install.sh | bash

# 2. Initialize — walks you through provider setup
armaraos init

# 3. Start the daemon
armaraos start

# 4. Dashboard is live at http://localhost:4200

# 5. Activate a Hand — it starts working for you
armaraos hand activate researcher

# 6. Chat with an agent
armaraos chat researcher
> "What are the emerging trends in AI agent frameworks?"

# 7. Spawn a pre-built agent
armaraos agent spawn coder
```

<details>
<summary><strong>Windows (PowerShell)</strong></summary>

```powershell
irm https://raw.githubusercontent.com/sbhooley/armaraos/main/scripts/install.ps1 | iex
armaraos init
armaraos start
```

</details>

---

## Development

```bash
# Build the workspace
cargo build --workspace --lib

# Run all tests (1,767+)
cargo test --workspace

# Lint (must be 0 warnings)
cargo clippy --workspace --all-targets -- -D warnings

# Format
cargo fmt --all -- --check
```

---

## Stability Notice

ArmaraOS is pre-1.0. The architecture is solid, the test suite is comprehensive, and the security model is comprehensive. That said:

- **Breaking changes** may occur between minor versions until v1.0
- **Some Hands** are more mature than others (Browser and Researcher are the most battle-tested)
- **Edge cases** exist — if you find one, open an issue in this repo
- **Pin to a specific commit** for production deployments until v1.0

We ship fast and fix fast. The goal is a rock-solid v1.0 by mid-2026.

---

## Security

To report a security vulnerability, email **ainativelang@gmail.com**. We take all reports seriously and will respond within 48 hours.

---

## License

Licensed under **MIT or Apache 2.0** (your choice). See [`LICENSE-MIT`](LICENSE-MIT) and [`LICENSE-APACHE`](LICENSE-APACHE).

ArmaraOS is based on [OpenFang](https://github.com/RightNow-AI/openfang) (MIT/Apache-2.0, Copyright © 2024 OpenFang Contributors). See [`NOTICE`](NOTICE) for full upstream attribution.

---

## Links

- [ArmaraOS on GitHub](https://github.com/sbhooley/armaraos)
- [AI Native Lang (AINL)](https://github.com/sbhooley/ainativelang)
- [Twitter / X — @ainativelang](https://x.com/ainativelang)

---

<p align="center">
  <strong>Built with Rust. Secured with 16 layers. Agents that actually work for you.</strong>
</p>
