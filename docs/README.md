# ArmaraOS Documentation

Welcome to the ArmaraOS documentation. ArmaraOS is the open-source Agent Operating System -- **15** Rust library crates in this workspace plus **`xtask`** (16 Cargo members), 40 channels, 60 skills, 20 LLM providers, 80+ HTTP/WebSocket/SSE endpoints (see [API Reference](api-reference.md)), and 16 security systems in a single binary.

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
| [Architecture](architecture.md) | Workspace crate graph, kernel boot, agent lifecycle, memory + graph-memory substrates |
| [Agent Templates](agent-templates.md) | 30 pre-built agents across 4 performance tiers |
| [Workflows](workflows.md) | Multi-agent pipelines with branching, fan-out, loops, and triggers |
| [Agent automation hardening](agent-automation-hardening.md) | Tool args, persist vs re-scrape, loop guard interaction, phases, workspace habits; skill-mint stub cron reference |
| [Security](security.md) | 16 defense-in-depth security systems |

## Multi-agent orchestration and observability

| Guide | Description |
|-------|-------------|
| [Orchestration guide](orchestration-guide.md) | Dashboard **Monitor → Orchestration traces**, API + SSE, quotas |
| [Orchestration walkthrough](orchestration-walkthrough.md) | Hands-on URL hash **`#orchestration-traces`** and curl patterns |
| [Task queue + orchestration](task-queue-orchestration.md) | **`task_post` / `task_claim`** stickiness with **`trace_id`** |
| [Workflow examples](workflow-examples.md) | Compact JSON + register/run recipes |
| [Orchestration implementation audit](orchestration-implementation-audit.md) | Gaps, tests, and follow-ups |
| [Agent orchestration design](agent-orchestration-design.md) | Deep design (§ phases, status) |
| [Agent orchestration phases](agent-orchestration-phases.md) | Phased delivery checklist |

## Prompt caching and proactive learning (reference)

| Guide | Description |
|-------|-------------|
| [Prompt caching (multi-provider)](prompt-caching-multi-provider.md) | Cache behavior across providers |
| [Prompt caching (Anthropic)](prompt-caching-anthropic.md) | Anthropic-specific notes |
| [Proactive learning design](proactive-learning-design.md) | Design narrative |
| [Proactive learning phases](proactive-learning-phases.md) | Phased implementation notes |

## Integrations

| Guide | Description |
|-------|-------------|
| [ainl-runtime + GraphPatch (Rust)](ainl-runtime-graph-patch.md) | Patch **`PatchAdapter`** registry, **`GraphPatchAdapter`** summary JSON, semantic ranking migration, crates.io dependency matrix |
| [ainl-runtime in OpenFang (optional)](ainl-runtime-integration.md) | Feature **`ainl-runtime-engine`**, bridge behavior, **`TurnOutcome`** mapping, approvals |
| [Channel Adapters](channel-adapters.md) | 40 messaging channels -- setup, configuration, custom adapters |
| [LLM Providers](providers.md) | 20 providers, 51 models, 23 aliases -- setup and model routing |
| [OpenRouter defaults & fallbacks](openrouter.md) | Product default `:free` model, rate-limit fallback chain, chat error banner behavior |
| [Skills](skill-development.md) | 60 bundled skills, custom skill development, FangHub marketplace |
| [MCP & A2A](mcp-a2a.md) | Model Context Protocol and Agent-to-Agent protocol integration |

## Reference

| Guide | Description |
|-------|-------------|
| [Data directory](data-directory.md) | `~/.armaraos/`, env overrides, migration from `~/.openfang` |
| [AINL graph memory](graph-memory.md) | Runtime wiring: `GraphMemoryWriter`, per-agent `ainl_memory.db`, Python inbox drain, post-turn **`ExtractionReport`**, optional **`runtime_state`** when **`ainl-runtime`** shares the DB, extractor/tagger bridges + env toggles, vs orchestration traces |
| [Persona evolution (axis hook)](persona-evolution.md) | **`AINL_PERSONA_EVOLUTION`**, `PersonaEvolutionHook`, vs `run_persona_evolution_pass` |
| [ainl-runtime crate](ainl-runtime.md) | Standalone graph orchestration (`run_turn` / optional `run_turn_async`), Tokio `async` feature, verification vs daemon path |
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
| [Dashboard testing](dashboard-testing.md) | Smoke script, support diagnostics zip (create/download; **`README.txt`** + **`diagnostics_snapshot.json`** triage), Home folder preview vs download, chat unread + digest, **LLM error banner** (`humanizeChatError`, 401 vs 403 vs billing), kernel SSE, **Orchestration traces** (`#orchestration-traces`), **Logs** tabs, **Get started** (`#overview`) checklist + Quick actions (seven tiles incl. **Daemon & runtime**) + Setup Wizard visibility + **end-to-end `#wizard` QA**, **App Store** section title, **Settings / Runtime** layout smoke (**Settings** at-a-glance config schema line + mismatch suffix), **daemon lifecycle** + **GitHub-latest** QA, **Agents → Agent detail modal (gear)** + **Config** QA, **`/api/ui-prefs`** pinned agents, **HTTP chat fallback** (`POST …/message` **`tools`** + tool-cluster UI; rebuild/restart after embedded asset changes), Playwright notes |
| [Dashboard Home folder](dashboard-home-folder.md) | Home browser API + **dashboard UI** (row/modal Download, symlinks, large files when preview hits 512 KiB cap) |
| [Dashboard Get started UI](dashboard-overview-ui.md) | `#overview` landing: layout, **Quick actions** (incl. **App Store**, **Daemon & runtime** → `#runtime`), **Comms** under **Monitor**, Setup Wizard gating (`openfang-onboarded`, `navigateOverview`), setup checklist, seven-tile skeleton, CSS and source map |
| [Dashboard Setup Wizard](dashboard-setup-wizard.md) | `#wizard` first-run flow: provider test / Next rules, flat `manifest_toml` for `POST /api/agents`, valid `ToolProfile` values, static embed + rebuild note, links to overview and API |
| [Dashboard Settings & Runtime UI](dashboard-settings-runtime-ui.md) | `#settings` / `#runtime` plus shared **`dashboard-page-*`** shell on **Skills**, **Channels**, **Hands**, **Home folder**, **Analytics**; **`dashboard-toolbar-tabs`**, **Channels** filter card, **Analytics** stat grid; **Settings** summary line (config schema, API, log, home); **Budget** tab **Ultra Cost-Efficient Mode** card + **Chat** header **⚡ eco** toggle; daemon **Reload** / **Shut down**, **`daemon_lifecycle.js`** — class map and files |
| [Scheduled AINL](scheduled-ainl.md) | Cron **`ainl run`**, secrets / adapter env, **`AINL_BUNDLE_PATH`** + **`bundle.ainlbundle`** round-trip, per-agent Rust graph memory vs Python bridge |

## Additional Resources

| Resource | Description |
|----------|-------------|
| [CONTRIBUTING.md](../CONTRIBUTING.md) | Development setup, code style, PR guidelines |
| [MIGRATION.md](../MIGRATION.md) | Migrating from OpenClaw, LangChain, or AutoGPT |
| [SECURITY.md](../SECURITY.md) | Security policy and vulnerability reporting |
| [CHANGELOG.md](../CHANGELOG.md) | Release notes and version history |
| [ARCHITECTURE.md](../ARCHITECTURE.md) | Repo layering: OpenFang crates, **`ainl-memory`**, **`ainl-runtime`**, integration roadmap |
| [PRIOR_ART.md](../PRIOR_ART.md) | Graph-as-memory timeline and attribution notes |

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
| Library crates (`crates/*` excl. `xtask`) | 15 |
| Cargo workspace members (incl. `xtask`) | 16 |
| Agent templates | 30 |
| Messaging channels | 40 |
| Bundled skills | 60 |
| Built-in tools | 38 |
| LLM providers | 20 |
| Models in catalog | 51 |
| Model aliases | 23 |
| API endpoints | 77 |
| Security systems | 16 |
| Tests | 1,767+ |

### Important paths

See **[data-directory.md](data-directory.md)** for overrides and migration from `~/.openfang`.

| Path | Description |
|------|-------------|
| `~/.armaraos/config.toml` | Main configuration file |
| `~/.armaraos/data/openfang.db` | Main SQLite database (kernel / memory substrate) |
| `~/.armaraos/agents/<id>/ainl_memory.db` | Optional per-agent **`ainl-memory`** graph store (episodes, facts, persona for LLM prompt); see [graph-memory.md](graph-memory.md), [data-directory.md](data-directory.md) |
| `~/.armaraos/agents/<id>/ainl_graph_memory_inbox.json` | Optional Python→Rust graph inbox (drained each agent turn); [graph-memory.md](graph-memory.md), [data-directory.md](data-directory.md) |
| `~/.armaraos/agents/<id>/bundle.ainlbundle` | Optional **AINL bundle** for scheduled **`ainl run`** round-trip; see [scheduled-ainl.md](scheduled-ainl.md) |
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
