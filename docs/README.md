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
| [Dashboard UI design system](dashboard-design-system.md) | Canonical tokens/rules for dashboard typography, spacing, radii, shadows, and agent-safe design changes |
| [Agent Templates](agent-templates.md) | 30 pre-built agents across 4 performance tiers |
| [Workflows](workflows.md) | Multi-agent pipelines with branching, fan-out, loops, and triggers |
| [Agent automation hardening](agent-automation-hardening.md) | Tool args, persist vs re-scrape, loop guard interaction, phases, workspace habits; skill-mint stub cron reference |
| [Security](security.md) | 16 defense-in-depth security systems |

## Multi-agent orchestration and observability

| Guide | Description |
|-------|-------------|
| [Orchestration guide](orchestration-guide.md) | Dashboard **Agents → Orchestration**, API + SSE, quotas |
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
| [ainl-runtime doc hub](ainl-runtime.md) | **`AinlRuntime`** overview: **Orientation FAQ** (public API, workspace deps, MCP/CLI vs this crate, Python `RuntimeEngine` overlap); sync vs **`run_turn_async`** (**`async`** feature), internal **delegation depth** / **`AinlRuntimeError`**, **`TurnOutcome`** / **`TurnPhase`** (including per-slot **`ExtractionReport`** → **`ExtractionPass`** / **`PatternPersistence`** / **`PersonaEvolution`**), session **`runtime_state`**, mutex / **`spawn_blocking`** design; **episodic tool canonicalization**, **episode node id** vs **`turn_id`**, **direct `EvolutionEngine`** vs extractor; `cargo test` / clippy matrix |
| [ainl-runtime + GraphPatch (Rust)](ainl-runtime-graph-patch.md) | Patch **`PatchAdapter`** registry, **`GraphPatchAdapter`** summary JSON, semantic ranking migration, delegation depth / hard errors, crates.io dependency matrix |
| [ainl-runtime in OpenFang](ainl-runtime-integration.md) | Default-built **`ainl-runtime-engine`** bridge, runtime activation rules, **`TurnOutcome`** mapping, approvals |
| [Channel Adapters](channel-adapters.md) | 40 messaging channels -- setup, configuration, custom adapters |
| [LLM Providers](providers.md) | 20 providers, 51 models, 23 aliases -- setup and model routing |
| [OpenRouter defaults & fallbacks](openrouter.md) | Product default `:free` model, rate-limit fallback chain, chat error banner behavior |
| [Skills](skill-development.md) | 60 bundled skills, custom skill development, FangHub marketplace |
| [MCP & A2A](mcp-a2a.md) | Model Context Protocol and Agent-to-Agent protocol integration |

## Reference

| Guide | Description |
|-------|-------------|
| [Data directory](data-directory.md) | `~/.armaraos/`, env overrides, migration from `~/.openfang` |
| [Local voice](local-voice.md) | Whisper.cpp + Piper: first-launch `~/.armaraos/voice/`, `[local_voice]`, STT for user audio in WebChat, **text replies by default** (optional Piper voice reply) |
| [AINL graph memory](graph-memory.md) | Runtime wiring: `GraphMemoryWriter`, per-agent `ainl_memory.db`, Python inbox drain, post-turn **`ExtractionReport`** (per-phase errors + **`warn!`** slots), optional **`runtime_state`** when **`ainl-runtime`** shares the DB; **`AINL_EXTRACTOR_ENABLED`** (opt-out) / **`AINL_TAGGER_ENABLED`** (**tagger: only** `1`, opt-in) / **`AINL_PERSONA_EVOLUTION`** (opt-out); vs orchestration traces |
| [Persona evolution (axis hook)](persona-evolution.md) | **`PersonaEvolutionHook`** (on by default; opt out via **`AINL_PERSONA_EVOLUTION=0`**), axis snapshot growth/decay |
| [ainl-runtime crate](ainl-runtime.md) | Standalone graph orchestration (`run_turn` / optional `run_turn_async`), **Orientation FAQ**, delegation depth (**`DelegationDepthExceeded`**), Tokio `async` feature, verification vs daemon path |
| [API Reference](api-reference.md) | REST/WebSocket/SSE endpoints (see doc + quick-reference table; includes audit/daemon log routes) |
| [Ultra Cost-Efficient Mode](prompt-compression-efficient-mode.md) | Input prompt compression (`efficient_mode`), preserve rules, dashboard/API/telemetry, Eco Diff |
| [Desktop App](desktop.md) | Tauri 2.0 native app -- build, features, architecture |
| [Desktop code signing](desktop-code-signing.md) | Install-time trust (macOS / Windows), Tauri updater vs OS signing, Azure / SignPath notes |

## Release & Operations

| Guide | Description |
|-------|-------------|
| [Release candidate validation](release-candidate-validation.md) | Pre-tag `cargo` / `check-version-consistency.sh` / `verify-dashboard-smoke.sh` + pointers to manual QA and post-tag updater checks |
| [GA sign-off checklist](ga-signoff-checklist.md) | Human approvals (product, runtime, security/privacy, data/ML): exact steps, evidence, sign-off template for graph memory GA |
| [Releasing (semver)](RELEASING.md) | Routine bump → `CHANGELOG` → `cargo fmt` / test / clippy → tag → GitHub Release; **ainativelangweb** `latestArmaraosReleaseTag`; audit/API notes |
| [Docker](docker.md) | Image layout, `OPENSSL_NO_VENDOR`, cargo-chef caching, build args, multi-arch |
| [Production Checklist](production-checklist.md) | First-ship gate before tagging v0.1.0 — signing keys, secrets, verification |
| [Desktop code signing](desktop-code-signing.md) | Gatekeeper, SmartScreen, `TAURI_SIGNING_PRIVATE_KEY` vs Authenticode / notarization, GitHub Actions secrets, Azure Artifact Signing, SignPath OSS |
| [Desktop release smoke](release-desktop.md) | Tauri build, updater, optional PostHog (`ARMARAOS_POSTHOG_KEY` / `AINL_POSTHOG_KEY`), AINL tab, SSE badge, API tests; **ainativelang.com** homepage/`/download` installer block (see “Marketing site installers” in that doc) |
| [Desktop AINL bootstrap smoke](DESKTOP_AINL_SMOKE.md) | Venv, wheel, PyPI, first-launch AINL checks |
| [Dashboard testing](dashboard-testing.md) | Smoke script, support diagnostics zip (create/download; **`README.txt`** + **`diagnostics_snapshot.json`** triage), Home folder **full-viewport** preview vs download, **chat unread** (sidebar + **Fleet Status** + **bell**; digest + `notifyChatReplies`), **LLM error banner** (`humanizeChatError`, 401 vs 403 vs billing), kernel SSE, **Orchestration traces** (`#orchestration-traces`), **Logs** tabs, **Get started** (`#overview`) checklist + Quick actions (seven tiles incl. **Daemon & runtime**) + **`/api/usage/summary`** hero totals + **Operations snapshot** (integration row + optional kernel KPIs) + Setup Wizard visibility + **end-to-end `#wizard` QA**, **App Store** section title, **Settings / Runtime** layout smoke (**Settings** at-a-glance config schema line + mismatch suffix), **daemon lifecycle** + **GitHub-latest** QA, **Agents → Agent detail modal (gear)** + **Config** QA (incl. default **tool allowlist** merge), **`/api/ui-prefs`** pinned agents + **`agent_eco_modes`**, chat **workspace** pill → **Home folder**, **HTTP chat fallback** (`POST …/message` **`tools`** + tool-cluster UI; rebuild/restart after embedded asset changes), Playwright notes |
| [Dashboard Home folder](dashboard-home-folder.md) | Home browser API + **dashboard UI** (full-viewport **View** modal, row/modal Download, symlinks, large files when preview hits 512 KiB cap) |
| [Dashboard Get started UI](dashboard-overview-ui.md) | `#overview` landing: layout, **Quick actions** (incl. **App Store**, **Daemon & runtime** → `#runtime`), **Operations snapshot** (channels/skills/MCP/tools/providers + observability when available), **Comms** under **Monitor**, Setup Wizard gating (`openfang-onboarded`, `navigateOverview`), setup checklist, skeleton (Quick actions + snapshot shape), CSS and source map |
| [Dashboard Setup Wizard](dashboard-setup-wizard.md) | `#wizard` first-run flow: provider test / Next rules, flat `manifest_toml` for `POST /api/agents`, valid `ToolProfile` values, static embed + rebuild note, links to overview and API |
| [Dashboard Settings & Runtime UI](dashboard-settings-runtime-ui.md) | `#settings` / `#runtime` plus shared **`dashboard-page-*`** shell on **Skills**, **Channels**, **Hands**, **Home folder**, **Analytics**; **`dashboard-toolbar-tabs`**, **Channels** filter card, **Analytics** stat grid; **Settings** summary line (config schema, API, log, home); **Budget** tab **Ultra Cost-Efficient Mode** card + **Chat** header **⚡ eco** toggle (**`ui-prefs.json`** **`agent_eco_modes`**) + **workspace** pill → **Home folder**; daemon **Reload** / **Shut down**, **`daemon_lifecycle.js`** — class map and files |
| [Scheduled AINL](scheduled-ainl.md) | Cron **`ainl run`**, secrets / adapter env, **which job stdout is appended to Chat** vs **quiet success** (health/budget monitors), **`AINL_BUNDLE_PATH`** + **`bundle.ainlbundle`** round-trip, per-agent Rust graph memory vs Python bridge |
| [Out-of-the-box AINL](ootb-ainl.md) | Embedded **`armaraos-programs`**, **curated cron** catalog, env overrides, App Store integration |

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
| Tests | 2,793+ |

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

---

## Maintainers (releases + version samples)

| Doc | Description |
|-----|-------------|
| [RELEASING.md](RELEASING.md) | Version bump, changelog, tag, marketing-site fallback |
| [release-candidate-validation.md](release-candidate-validation.md) | Pre-tag `cargo` checks, **`scripts/check-version-consistency.sh`**, **`verify-dashboard-smoke.sh`**, manual QA pointer |
| [release-desktop.md](release-desktop.md) | Desktop smoke, **`latest.json`**, post-tag updater verification |

**Documentation version samples:** Example JSON in **[api-reference.md](api-reference.md)** must match the current workspace version from the repo-root **`Cargo.toml`**; CI runs **`scripts/check-version-consistency.sh`** from the repository root. Update those samples on every semver bump.
