# ArmaraOS Documentation

Welcome to the ArmaraOS documentation. ArmaraOS is the open-source Agent Operating System -- 14 Rust crates, 40 channels, 60 skills, 20 LLM providers, 80+ HTTP/WebSocket/SSE endpoints (see [API Reference](api-reference.md)), and 16 security systems in a single binary.

---

## Getting Started

| Guide | Description |
|-------|-------------|
| [Getting Started](getting-started.md) | Installation, first agent, first chat session |
| [Configuration](configuration.md) | Complete `config.toml` reference with every field |
| [CLI Reference](cli-reference.md) | Every command and subcommand with examples |
| [Troubleshooting](troubleshooting.md) | Common issues, FAQ, diagnostics |

## Core Concepts

| Guide | Description |
|-------|-------------|
| [AINL first (default language)](ainl-first-language.md) | AINL as default for programs/apps; Rust host; when to use other languages |
| [Architecture](architecture.md) | 12-crate structure, kernel boot, agent lifecycle, memory substrate |
| [Agent Templates](agent-templates.md) | 30 pre-built agents across 4 performance tiers |
| [Workflows](workflows.md) | Multi-agent pipelines with branching, fan-out, loops, and triggers |
| [Agent automation hardening](agent-automation-hardening.md) | Tool args, persist vs re-scrape, loop guard interaction, phases, workspace habits; skill-mint stub cron reference |
| [Security](security.md) | 16 defense-in-depth security systems |

## Integrations

| Guide | Description |
|-------|-------------|
| [Channel Adapters](channel-adapters.md) | 40 messaging channels -- setup, configuration, custom adapters |
| [LLM Providers](providers.md) | 20 providers, 51 models, 23 aliases -- setup and model routing |
| [OpenRouter defaults & fallbacks](openrouter.md) | Product default `:free` model, rate-limit fallback chain, chat error banner behavior |
| [Skills](skill-development.md) | 60 bundled skills, custom skill development, FangHub marketplace |
| [MCP & A2A](mcp-a2a.md) | Model Context Protocol and Agent-to-Agent protocol integration |

## Reference

| Guide | Description |
|-------|-------------|
| [Data directory](data-directory.md) | `~/.armaraos/`, env overrides, migration from `~/.openfang` |
| [API Reference](api-reference.md) | REST/WebSocket/SSE endpoints (see doc + quick-reference table; includes audit/daemon log routes) |
| [Ultra Cost-Efficient Mode](prompt-compression-efficient-mode.md) | Input prompt compression (`efficient_mode`), preserve rules, dashboard/API/telemetry, Eco Diff |
| [Desktop App](desktop.md) | Tauri 2.0 native app -- build, features, architecture |
| [Desktop code signing](desktop-code-signing.md) | Install-time trust (macOS / Windows), Tauri updater vs OS signing, Azure / SignPath notes |

## Release & Operations

| Guide | Description |
|-------|-------------|
| [Releasing (semver)](RELEASING.md) | Routine bump → `CHANGELOG` → `cargo fmt` / test / clippy → tag → GitHub Release; **ainativelangweb** `latestArmaraosReleaseTag`; audit/API notes |
| [Docker](docker.md) | Image layout, `OPENSSL_NO_VENDOR`, cargo-chef caching, build args, multi-arch |
| [Production Checklist](production-checklist.md) | First-ship gate before tagging v0.1.0 — signing keys, secrets, verification |
| [Desktop code signing](desktop-code-signing.md) | Gatekeeper, SmartScreen, `TAURI_SIGNING_PRIVATE_KEY` vs Authenticode / notarization, GitHub Actions secrets, Azure Artifact Signing, SignPath OSS |
| [Desktop release smoke](release-desktop.md) | Tauri build, updater, optional PostHog (`ARMARAOS_POSTHOG_KEY` / `AINL_POSTHOG_KEY`), AINL tab, SSE badge, API tests; **ainativelang.com** homepage/`/download` installer block (see “Marketing site installers” in that doc) |
| [Desktop AINL bootstrap smoke](DESKTOP_AINL_SMOKE.md) | Venv, wheel, PyPI, first-launch AINL checks |
| [Dashboard testing](dashboard-testing.md) | Smoke script, support diagnostics zip (create/download; **`README.txt`** + **`diagnostics_snapshot.json`** triage), Home folder preview vs download, chat unread + digest, **LLM error banner** (`humanizeChatError`, 401 vs 403 vs billing), kernel SSE, **Logs** tabs, **Get started** (`#overview`) checklist + Quick actions (seven tiles incl. **Daemon & runtime**) + Setup Wizard visibility + **end-to-end `#wizard` QA**, **App Store** section title, **Settings / Runtime** layout smoke (**Settings** at-a-glance config schema line + mismatch suffix), **daemon lifecycle** + **GitHub-latest** QA, **Agents → Agent detail modal (gear)** + **Config** QA, **`/api/ui-prefs`** pinned agents, **HTTP chat fallback** (`POST …/message` **`tools`** + tool-cluster UI; rebuild/restart after embedded asset changes), Playwright notes |
| [Dashboard Home folder](dashboard-home-folder.md) | Home browser API + **dashboard UI** (row/modal Download, symlinks, large files when preview hits 512 KiB cap) |
| [Dashboard Get started UI](dashboard-overview-ui.md) | `#overview` landing: layout, **Quick actions** (incl. **App Store**, **Daemon & runtime** → `#runtime`), **Comms** under **Monitor**, Setup Wizard gating (`openfang-onboarded`, `navigateOverview`), setup checklist, seven-tile skeleton, CSS and source map |
| [Dashboard Setup Wizard](dashboard-setup-wizard.md) | `#wizard` first-run flow: provider test / Next rules, flat `manifest_toml` for `POST /api/agents`, valid `ToolProfile` values, static embed + rebuild note, links to overview and API |
| [Dashboard Settings & Runtime UI](dashboard-settings-runtime-ui.md) | `#settings` / `#runtime` plus shared **`dashboard-page-*`** shell on **Skills**, **Channels**, **Hands**, **Home folder**, **Analytics**; **`dashboard-toolbar-tabs`**, **Channels** filter card, **Analytics** stat grid; **Settings** summary line (config schema, API, log, home); **Budget** tab **Ultra Cost-Efficient Mode** card + **Chat** header **⚡ eco** toggle; daemon **Reload** / **Shut down**, **`daemon_lifecycle.js`** — class map and files |
| [Scheduled AINL](scheduled-ainl.md) | Cron `ainl run`, `~/.armaraos/.env`, `AINL_HOST_ADAPTER_ALLOWLIST`, `AINL_ALLOW_IR_DECLARED_ADAPTERS`, editing jobs |

## Additional Resources

| Resource | Description |
|----------|-------------|
| [CONTRIBUTING.md](../CONTRIBUTING.md) | Development setup, code style, PR guidelines |
| [MIGRATION.md](../MIGRATION.md) | Migrating from OpenClaw, LangChain, or AutoGPT |
| [SECURITY.md](../SECURITY.md) | Security policy and vulnerability reporting |
| [CHANGELOG.md](../CHANGELOG.md) | Release notes and version history |

---

## Quick Reference

### Start in 30 Seconds

```bash
export GROQ_API_KEY="your-key"
armaraos init && armaraos start
# Open http://127.0.0.1:4200
```

(The upstream binary name `openfang` is still supported in many builds; paths below use the ArmaraOS default.)

### Key Numbers

| Metric | Count |
|--------|-------|
| Crates | 14 |
| Agent templates | 30 |
| Messaging channels | 40 |
| Bundled skills | 60 |
| Built-in tools | 38 |
| LLM providers | 20 |
| Models in catalog | 51 |
| Model aliases | 23 |
| API endpoints | 77 |
| Security systems | 16 |
| Tests | 967 |

### Important paths

See **[data-directory.md](data-directory.md)** for overrides and migration from `~/.openfang`.

| Path | Description |
|------|-------------|
| `~/.armaraos/config.toml` | Main configuration file |
| `~/.armaraos/data/openfang.db` | SQLite database |
| `~/.armaraos/skills/` | Installed skills |
| `~/.armaraos/daemon.json` | Daemon PID and port info |
| `agents/` | Agent template manifests (repo / dev) |

### Key Environment Variables

| Variable | Provider |
|----------|----------|
| `ANTHROPIC_API_KEY` | Anthropic (Claude) |
| `OPENAI_API_KEY` | OpenAI (GPT-4o) |
| `GEMINI_API_KEY` | Google Gemini |
| `GROQ_API_KEY` | Groq (fast Llama/Mixtral) |
| `DEEPSEEK_API_KEY` | DeepSeek |
| `XAI_API_KEY` | xAI (Grok) |
| `ARMARAOS_HOME` | Override data directory (replaces `~/.armaraos`) |
| `OPENFANG_HOME` | Legacy alias for `ARMARAOS_HOME` |

Only one provider key is needed to get started. Groq offers a free tier.
