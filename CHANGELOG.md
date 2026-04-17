# Changelog

All notable changes to ArmaraOS will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

<!-- Next release changes go here -->

### Documentation

- **`CONTRIBUTING.md`**, **`CLAUDE.md`** — document **`api_boundary_contracts_test`**, **`sse_stream_auth`**, and WS unit tests under **`openfang-api`**.
- **`docs/api-reference.md`** — SSE section points at **`sse_stream_auth`** for loopback / Bearer parity on event and log streams.
- **`docs/mcp-a2a.md`** — HTTP **`POST /mcp`** verification note + **`api_boundary_contracts_test`**.

### Changed

- **ainl-runtime-engine defaults:** `openfang-runtime` now includes **`ainl-runtime-engine`** in default Cargo features, and new `AgentManifest` defaults (including wizard-created manifests) set **`ainl_runtime_engine = true`** by default.
- **Legacy agent migration:** On daemon boot, agents with no explicit `ainl_runtime_engine` key in on-disk `agent.toml` are safely migrated to `true` and persisted to SQLite, while explicit on-disk `true`/`false` values remain untouched.

### Fixed

- **Agent Config → AINL runtime engine toggle:** `PATCH /api/agents/{id}/config` now writes **`ainl_runtime_engine`** into **`agent.toml`** (the TOML patch path previously omitted it), restores the flag from SQLite when merging a newer on-disk template at daemon boot, and includes the boolean on **`GET /api/agents`** so the dashboard does not reset the checkbox from list payloads.

### Documentation

- **`docs/api-reference.md`** — `GET /api/agents` includes **`workspace`** / **`workspace_rel_home`**; correct **`GET /api/usage`** and **`GET /api/usage/summary`** response shapes (persistent metering); **`GET /api/agents/{id}/tools`** documents default merged tool names for non-empty allowlists; **`/api/ui-prefs`** documents **`agent_eco_modes`**.
- **`docs/dashboard-testing.md`** — eco **7b** per-agent checks; Get started usage hero + Analytics parity; Home folder **full-viewport** preview; Config tab default allowlist merge; **Open workspace** QA; **`ui-prefs`** curl note.
- **`docs/dashboard-home-folder.md`**, **`docs/dashboard-overview-ui.md`**, **`docs/dashboard-settings-runtime-ui.md`**, **`docs/prompt-compression-efficient-mode.md`**, **`docs/README.md`**, **`CLAUDE.md`** — dashboard copy aligned with persistent usage, **`ui-prefs`** eco map, workspace pill → **Home folder**, and full-screen file preview.

## [0.7.4] - 2026-04-14

### Documentation

- **`crates/ainl-memory/README.md`** — fifth memory family (**`RuntimeStateNode`** / `runtime_state`), **`read_runtime_state` / `write_runtime_state`** on **`GraphMemory`** + **`GraphQuery`**, legacy JSON key compatibility; episodic / semantic **`tags`** on exported nodes.
- **`crates/ainl-runtime/README.md`** — session persistence: DB location, **`persona_snapshot_json`**, **`TurnPhase::RuntimeStatePersist`**, test command for **`test_session_persistence`**; documentation map; **`TurnPhase`** vs **`ExtractionReport`** slot mapping; **`run_graph_extraction_pass`** **Result** semantics; async / delegation headings.
- **`docs/ainl-runtime.md`** — hub: intro paragraph (**`TurnInput::depth`** vs internal cap); *Where to read next* / **architecture.md** cell (**delegation depth**); **`RuntimeStateNode`** + **`ExtractionReport`**, **`run_turn` / `run_turn_async`**, **`async`**, Mutex vs Tokio, hooks; delegation / **`AinlRuntimeError`** / **`test_delegation_depth`**; verification (**`test_session_persistence`**, **`test_delegation_depth`**, **`test_turn_phase_granularity`**) + **`required-features`**, embedding caveats.
- **`docs/ainl-runtime-graph-patch.md`** — **`RuntimeStateNode`** / **`TurnPhase::RuntimeStatePersist`**; **Delegation depth and hard errors** subsection; cross-links **`docs/graph-memory.md`**, **`docs/ainl-runtime.md`**, **`crates/ainl-runtime/README.md`**.
- **`docs/ainl-runtime-integration.md`** — routed-turn table (delegation cap vs **`TurnInput::depth`**), WAL / **`runtime_state`**, default-loop graph env vars; troubleshooting / **`map_ainl_turn_outcome`** notes for **`DelegationDepthExceeded`** and granular **`TurnPhase`** **`TurnWarning`**s; hub intro cross-links delegation.
- **`docs/graph-memory.md`** — **`runtime_state`** when **`AinlRuntime`** shares **`ainl_memory.db`**; optional **ainl-runtime** nested delegation (**`max_delegation_depth`**, **`DelegationDepthExceeded`**); **`run_persona_evolution_pass`** / **`ExtractionReport`** ↔ **`TurnPhase`** **TurnWarning** mapping; **`AINL_EXTRACTOR_ENABLED`** vs **`AINL_TAGGER_ENABLED`** (**only** literal **`1`**) vs **`AINL_PERSONA_EVOLUTION`**; EndTurn write table; default vs **`AINL_GRAPH_MEMORY_ARMARAOS_EXPORT`**; developer map; **See also** cross-links.
- **`docs/data-directory.md`** — **`ainl_memory.db`** (**`runtime_state`**) and **`ainl_graph_memory_export.json`** / **`AINL_GRAPH_MEMORY_ARMARAOS_EXPORT`**.
- **`docs/persona-evolution.md`** — evolution pass return type / stub wording; **Same report shape in ainl-runtime** subsection (**`ExtractionReport`** → **`TurnWarning`** phases); related-docs and operator links to graph-memory env semantics and **`openfang-runtime/README.md`**.
- **`docs/README.md`** — graph-memory blurb (**per-phase** **`ExtractionReport`** / **`warn!`**); Integrations hub row (**delegation** / **`AinlRuntimeError`**, **`TurnOutcome`** / **`TurnPhase`** / **ExtractionReport** slot names); Reference table (**ainl-runtime** row: **`DelegationDepthExceeded`**).
- **`docs/architecture.md`** — graph-memory subsection + **`openfang-runtime`** / **`ainl-runtime`** crate rows (bridges, env toggles, hub link); **`ainl-runtime`** row notes internal delegation depth / **`DelegationDepthExceeded`**.
- **`docs/configuration.md`** — graph-memory toggles are **process environment** variables (not `config.toml` keys); pointer to **`docs/graph-memory.md`**.
- **`crates/openfang-runtime/README.md`** — default features; **`AINL_TAGGER_ENABLED`** (**only** literal **`1`**) vs truthy **`AINL_EXTRACTOR_ENABLED`**; **`run_persona_evolution_pass`** return type with vs without **`ainl-extractor`** (**warn!** per **ExtractionReport** slot, **TurnPhase** parity with **`AinlRuntime::run_turn`**); test commands for **`ainl-tagger`**.
- **`crates/openfang-runtime/src/graph_memory_writer.rs`** — module doc: post-turn episode + batch semantic/procedural writes.
- **Root `ARCHITECTURE.md`** — Layer 3 **`openfang-runtime`** wiring and episode / semantic **`tags`** on exported nodes; **`ainl-runtime`** bullets / crate table: internal delegation depth (**`DelegationDepthExceeded`**), **`TurnInput::depth`** metadata.
- **`CLAUDE.md`** — graph-memory + **`ainl-runtime`** blurb: **`ExtractionReport`** per-slot **`warn!`** vs **`TurnPhase`** **`TurnWarning`**s; internal **`max_delegation_depth`** / **`DelegationDepthExceeded`**; **`TurnInput::depth`** metadata only.
- **`crates/ainl-graph-extractor/README.md`** — **`GraphExtractorTask`** vs **`run_extraction_pass`**, **`ExtractionReport`** per-phase error slots, example + test commands.
- **`crates/ainl-graph-extractor/src/lib.rs`** / **`crates/ainl-memory/src/lib.rs`** — crate-level rustdoc for **`ExtractionReport`** and runtime state nodes.
- **`.env.example`** — commented graph-memory toggles with cross-links to **`docs/graph-memory.md`**, **`docs/persona-evolution.md`**, **`crates/openfang-runtime/README.md`**.
- **`crates/ainl-runtime/src/engine.rs`** — **`TurnPhase`** rustdoc: per-variant meaning + mapping from **`ainl_graph_extractor::ExtractionReport`** error slots.
- **`crates/ainl-runtime/src/lib.rs`** — async paragraph links **`docs/ainl-runtime.md`**; crate-level note on **`ExtractionReport`** → **`TurnWarning`** / **`TurnPhase`** tagging.
- **`crates/ainl-runtime/tests/test_async_runtime.rs`** — module doc points to hub doc.
- **`CONTRIBUTING.md`** — **`ainl-runtime`** crate row: delegation depth + **`cargo test -p ainl-runtime --test test_delegation_depth`**.

### Changed (workspace crates)

- **Published AINL crate chain (crates.io):** **`ainl-memory` 0.1.8-alpha**, **`ainl-persona` 0.1.4** (bumps `ainl-memory` lower bound for resolver compatibility), **`ainl-graph-extractor` 0.1.5**, then **`ainl-runtime` 0.3.5-alpha**. Workspace pins updated in **`openfang-runtime`**, **`ainl-runtime`**, **`ainl-graph-extractor`**.

- **`ainl-runtime` 0.3.5-alpha** (crates.io / git): **Turn pipeline** — `run_turn` / `run_turn_async` return **`Result<TurnOutcome, AinlRuntimeError>`** (`Complete` vs `PartialSuccess` + **`TurnWarning`** list with **`TurnPhase`**). **Delegation** — nested `run_turn` past **`max_delegation_depth`** is a hard **`AinlRuntimeError::DelegationDepthExceeded`** (default **8**); `TurnInput::depth` is metadata only. **Session** — **`RuntimeStateNode`** persists turn count, extraction cadence, and persona cache hints across restarts. **Semantic ranking** — **`MemoryContext::relevant_semantic`** uses **`infer_topic_tags`** when a non-empty user message is supplied; **`compile_memory_context_for(None)`** no longer falls back to the latest episode’s text for ranking (pass **`Some(message)`** for topic-aware order, or use **`run_turn`** which always passes the current turn text). **Patches** — **`PatchAdapter`** registry + **`GraphPatchAdapter`** fallback JSON summary (`label`, `patch_version`, `frame_keys`); **`PatchDispatchResult`** gains **`adapter_name` / `adapter_output`**. **`sqlite_store()`** returns **`SqliteStoreRef<'_>`** (short-lived guard). Re-export **`infer_topic_tags`**. Workspace **`scopeguard`** pin. See **`crates/ainl-runtime/README.md`** and **`docs/ainl-runtime-graph-patch.md`**.

- **`ainl-runtime` 0.3.2-alpha:** `AinlRuntimeError` is now an enum (`Message`, `DelegationDepthExceeded`). Nested `run_turn` beyond `RuntimeConfig::max_delegation_depth` returns `Err(DelegationDepthExceeded { depth, max })` instead of a completed turn with `TurnStatus::DepthLimitExceeded` (that status variant was removed). Migration: use `AinlRuntimeError::from(s)` / `?` for string errors; match or use `is_delegation_depth_exceeded`, `delegation_depth_exceeded`, and `message_str` helpers. Default `max_delegation_depth` is **8**. See **`crates/ainl-runtime/README.md`**.

- **AINL crate chain bumped to integration-verified versions:** **`ainl-memory` 0.1.9-alpha**, **`ainl-persona` 0.1.6**, **`ainl-graph-extractor` 0.1.6**, **`ainl-semantic-tagger` 0.1.6**, **`ainl-runtime` 0.3.6-alpha** — all published to crates.io and workspace-pinned in **`openfang-runtime`**.

### Added

- **Cognitive vitals on streaming path:** `crates/openfang-runtime/src/agent_loop.rs` — streaming turns now call `vitals_classifier::classify_from_text` on the final response text after the stream completes, so `vitals_gate`/`vitals_phase`/`vitals_trust` are populated in `ainl_memory.db` episode rows for dashboard chat (previously hardcoded `None`).

- **`GET /api/graph-memory` exposes vitals fields:** `crates/openfang-api/src/graph_memory.rs` — `GraphMemoryNodeOut` now includes `vitals_gate`, `vitals_phase`, and `vitals_trust` (skipped when null) for episode nodes, enabling the dashboard graph panel to colour-code and filter by cognitive vitals.

- **App Store Hand schema warning badges:** `crates/openfang-api/src/routes.rs` + `static/index_body.html` — `GET /api/hands` response includes `schema_version` and `schema_warning` (`"legacy"` when `schema_version` is absent, `"mismatch"` when it doesn't match the expected value, `null` when correct). The dashboard renders a `⚠ Legacy format` or `⚠ Schema mismatch` badge on affected hand cards in the App Store.

- **`ainl_runtime_engine` toggle in Agents → Config:** `PATCH /api/agents/{id}/config` accepts `ainl_runtime_engine: bool`; `GET /api/agents/{id}` returns it; `AgentRegistry::update_ainl_runtime_engine` applies it live. The Config tab shows a labelled checkbox with an "experimental" badge and inline doc link.

### Fixed

- **`AgentManifest` test initializers:** `crates/openfang-kernel/src/heartbeat.rs`, `kernel.rs` (×2), `registry.rs` — four struct literal initializers were missing the `ainl_runtime_engine: false` field added to `AgentManifest`. `cargo test --workspace` now compiles and passes cleanly.

## [0.7.3] - 2026-04-12

### Added

- **Orchestration observability:** Bounded in-memory **orchestration trace** ring, **`GET /api/orchestration/traces`** (+ per-trace events, tree, cost), kernel **`OrchestrationTrace`** events on **`GET /api/events/stream`**, dashboard **`#orchestration-traces`** (Monitor), and **`openfang orchestration`** CLI (`list`, `trace`, `cost`, `tree`, `live`, `quota`, `export`, `watch`). See **`docs/orchestration-guide.md`**, **`docs/api-reference.md`** (*Orchestration traces & quota*), **`docs/workflows.md`** (*Orchestration and traces*).
- **Task queue + traces:** Pending tasks can prefer **`orchestration.trace_id`** in JSON payloads; **`task_claim`** rehydrates **`OrchestrationContext`** for the agent’s next turn (**`docs/task-queue-orchestration.md`**).
- **Graph memory (`ainl-memory`):** Workspace crates **`ainl-memory`** and **`ainl-runtime`** (standalone / future host). **`openfang-runtime`** records graph nodes via **`GraphMemoryWriter`** at **`~/.armaraos/agents/<agent_id>/ainl_memory.db`** (per agent; separate from **`data/openfang.db`**): **EndTurn** episodes, **semantic** rows after successful tool execution, **`agent_delegate`** episodes (optional orchestration trace JSON), **`a2a_send`** episodes after **`A2aClient::send_task`**, plus **persona** recall into the chat **system prompt**. Scheduled **`ainl_run`** jobs use **`bundle.ainlbundle`** + **`AINL_BUNDLE_PATH`** for Python **`ainl_graph_memory`** round-trip (**`crates/openfang-runtime/src/ainl_bundle_cron.rs`**). Operator doc: **`docs/graph-memory.md`**. Crate READMEs: **`crates/ainl-memory/README.md`**, **`crates/ainl-runtime/README.md`**. Layering: repo-root **`ARCHITECTURE.md`**, **`docs/scheduled-ainl.md`**, timeline **`PRIOR_ART.md`**.
- **Graph memory (heuristic extraction):** Post-turn **`graph_extractor`** (regex, no extra LLM) derives **semantic** facts and **procedural** workflow nodes from completed chat turns; **`record_turn`** links successive episodes with **`follows`** edges; dashboard **`GET /api/graph-memory`** preserves **`follows`** rel. See **`crates/openfang-runtime/src/graph_extractor.rs`**, **`graph_memory_writer.rs`**, **`agent_loop.rs`**.
- **HTTP API:** `POST /api/agents/{id}/message` may include a top-level **`tools`** array — one **`ToolTurnRecord`** per tool execution in that blocking turn (`name`, **`input`** as a JSON string, **`result`**, **`is_error`**). Omitted when empty. Populated from **`AgentLoopResult.tool_turns`** in **`openfang-runtime`** (non-streaming and streaming agent loops accumulate the same list for parity).
- **Types:** **`ToolTurnRecord`** in **`openfang_types::message`** (shared by API JSON and runtime).

### Changed

- **Dashboard → Chat:** HTTP fallback (`static/js/pages/chat.js` **`_sendPayload`**) maps **`res.tools`** into the same in-bubble tool-cluster model as WebSocket **`tool_start`** / **`tool_end`** / **`tool_result`**.

### Fixed

- **Dashboard:** Tool cards no longer disappeared when chat fell back to HTTP because the client always pushed **`tools: []`** after **`POST …/message`**.
- **Dashboard → Graph memory:** Agent picker and WebKit-safe loading so the graph panel can show data for the selected agent.

### Documentation

- **Scheduled AINL + bundles:** **`docs/scheduled-ainl.md`** — **`Kernel::cron_run_job`** / **`CronAction::AinlRun`**, **`AINL_BUNDLE_PATH`**, post-run **`AINLBundleBuilder`** export, cross-links to **ainativelang** graph-memory docs; **`docs/graph-memory.md`** (runtime integration hub); **`docs/data-directory.md`**, **`docs/README.md`**, **`docs/architecture.md`**, **`docs/mcp-a2a.md`** (A2A send → graph note), **`CLAUDE.md`**, **`CONTRIBUTING.md`**, repo **`ARCHITECTURE.md`** — per-agent **`ainl_memory.db`**, **`bundle.ainlbundle`**, persona system-prompt hook (**`GraphMemoryWriter`**); **`crates/ainl-memory/README.md`**, **`crates/openfang-runtime/src/ainl_bundle_cron.rs`** module docs.
- **`CHANGELOG`**, **`docs/dashboard-testing.md`**, **`docs/cli-reference.md`**, **`docs/configuration.md`**, root **`README.md`**: orchestration traces QA, **`openfang orchestration`** CLI reference, **`[runtime_limits] orchestration_default_budget_ms`**, workspace crate counts (15 library crates + **`xtask`**), doc index links (**`orchestration-guide.md`**, design/walkthrough, caching, proactive learning), cross-links to **`ARCHITECTURE.md`** / **`PRIOR_ART.md`**.
- **`docs/api-reference.md`**, **`docs/architecture.md`**, **`docs/dashboard-testing.md`**, **`docs/troubleshooting.md`**, **`docs/getting-started.md`**, **`docs/prompt-compression-efficient-mode.md`**, **`CLAUDE.md`**, **`sdk/javascript/index.d.ts`**, **`sdk/javascript/index.js`**, **`sdk/python/openfang_client.py`**: HTTP **`tools`** contract, QA, troubleshooting, SDK hints, and integration-test notes.

## [0.7.2] - 2026-04-10

### Added

- **Audit:** New **`AgentManifestUpdate`** action for successful **`PUT /api/agents/{id}/update`** (persisted as `AgentManifestUpdate` in SQLite; older rows may still show **`ConfigChange`** for the same operation).
- **HTTP API:** **`GET /api/agents/{id}?omit=manifest_toml`** — comma-separated **`omit`** list drops top-level JSON fields; use to avoid the large canonical TOML when listing agent metadata.
- **SDK (JavaScript + Python):** **`agents.get`** accepts optional **`omit`** (e.g. `manifest_toml`) for the same behavior.

### Changed

- **HTTP API:** `PUT /api/agents/{id}/update` applies the parsed manifest to the running kernel (capabilities, scheduler quotas, proactive triggers, SQLite) and syncs or materializes `agents/<name>/agent.toml` under the configured home directory. Successful JSON responses use **`status`: `"ok"`** with **`name`** and **`note`**. Clients or scripts that expected the previous **`status`: `"acknowledged"`** no-op must treat **`"ok"`** as success and read **`note`** for session-clear / restart hints. The kernel **reloads autonomous background loops** (continuous / periodic / proactive triggers) from the new manifest **without a daemon restart** when the standard `Arc` handle is registered. Each successful apply appends an audit **`AgentManifestUpdate`** entry (`detail` includes `PUT agent manifest update`) for compliance trails.
- **Dashboard (Agents → agent → Config):** Explains **Save Config** (partial, session preserved) vs **advanced full manifest** (`PUT …/update`) with a confirmation dialog, client-side manifest checks, reload/apply controls, and success toasts that reference the audit trail. **`GET /api/agents/{id}`** includes **`manifest_toml`** for loading the editor (omit via query when not needed).
- **Dashboard → Monitor → Timeline:** **System** filter and action labels include **`AgentManifestUpdate`** (full manifest apply).
- **SDK (JavaScript + Python):** Documented **`manifest_toml`** on **`agents.get`** / **`agents.update`** (full manifest replace). TypeScript **`AgentDetail`** includes optional **`manifest_toml`**.
- **Tests:** `api_integration_test` covers **`GET /api/agents/{id}`**, **`manifest_toml`**, **`?omit=manifest_toml`**, and **`AgentManifestUpdate`** audit after **`PUT …/update`**.

### Fixed

- **Agent registry / disk paths:** Per-agent directory renames and `agent.toml` sync use the same **`home_dir`** as the kernel config (no divergence from `openfang_home_dir()` when `home_dir` is customized).

### Documentation

- **`docs/api-reference.md`:** Documented **`PATCH /api/agents/{id}`** vs **`PUT …/update`**, **`GET …/agents/{id}`** **`omit`** query, expanded **`PATCH …/config`** request fields, and refreshed the endpoint summary table for common agent routes. **`PUT …/update`** audit text now references **`AgentManifestUpdate`**. **`GET /api/audit/recent`** documents query params **`n`** / **`q`** and the real JSON shape (`seq`, `action`, `tip_hash`, …).
- **`docs/RELEASING.md`:** New semver release checklist (bump, `CHANGELOG`, **ainativelangweb** tag, verify, tag). **`docs/README.md`**, **`docs/release-desktop.md`**, **`docs/production-checklist.md`**, and root **`README.md`** cross-link it.

## [0.7.1] - 2026-04-08

Patch release after **0.7.0** — CLI compile fixes for efficient-mode telemetry, eco mode defaults and dashboard UX, and AINL wheel pin **1.4.4** in release workflow.

### Fixed

- **CLI (`openfang-cli`):** `StreamEvent::CompressionStats` variant not handled in `chat_runner.rs` and `tui/mod.rs` match arms — caused compile failure for `cargo test`. `AgentLoopResult` struct initializers in `tui/event.rs` were also missing the new `compression_savings_pct` / `compressed_input` fields.
- **Dashboard → Chat:** Eco mode quick-toggle pill now persists to **`localStorage`** (`armaraos-eco-mode`) immediately on every click, and the initial value is read from `localStorage` before the async `GET /api/config` resolves — prevents the mode resetting to Balanced on page reload or after app update. Default changed from `'balanced'` to `'off'` for clean installs. **Settings → Budget** `saveEfficientMode` also writes to `localStorage`.
- **Runtime config:** `efficient_mode` Rust default corrected from `"balanced"` to `"off"` — a fresh `config.toml` (or no config) no longer silently enables prompt compression. Dashboard JS and Rust default now agree on `"off"` as the out-of-the-box state.
- **Release workflow:** `AINL_PYPI_VERSION` bumped from `1.4.3` to `1.4.4` so desktop bundles embed the latest AINL wheel.
- **Dashboard → Chat telemetry strip:** Tokens in/out, latency, and message count items were hidden by `x-show` until after the first reply. All items now render immediately with `—` placeholders; they fill in live once data is available.

### Changed

- **Dashboard → Chat:** Eco mode button restyled as a rounded pill (`.eco-pill` / `.eco-pill-off` / `.eco-pill-bal` / `.eco-pill-agg`) with a ⚡ bolt icon — matches the badge/chip visual language of the rest of the dashboard instead of appearing as a gray square.
- **Dashboard → Chat:** Added persistent **telemetry strip** below the chat header: context pressure dot (`ctx ok / mid / high / full`), session tokens in/out, last-turn latency, message count, and rolling eco compression savings % when active.
- **Dashboard → Chat:** Added **LLM error banner** (slides in below telemetry strip on any provider error) using existing `humanizeChatError` friendly-message logic — rate-limit errors show amber, other errors show red; hover for raw technical detail; dismissable with ×.
- **Dashboard → Chat:** Per-message eco diff button restyled as `.eco-savings-badge` green pill.

## [0.7.0] - 2026-04-08

This minor follows the **0.6.6 → 0.6.9** patch line; see those sections below for intervening fixes. **0.7.0** ships the items below (dashboard, API, efficient mode, release tooling).

### Added

- **Ultra Cost-Efficient Mode (runtime):** Heuristic **prompt compression** in **`openfang-runtime`** ([`prompt_compressor.rs`](crates/openfang-runtime/src/prompt_compressor.rs)) — wired into the agent loop; global **`efficient_mode`** in config and per-agent metadata override (`balanced` / `aggressive` / off). Chat shows **eco** indicators; response meta may include compression stats. See [docs/prompt-compression-efficient-mode.md](docs/prompt-compression-efficient-mode.md).
- **HTTP API:** **`GET /api/ui-prefs`** and **`PUT /api/ui-prefs`** — persist dashboard UI preferences to **`~/.armaraos/ui-prefs.json`** (atomic write; same pattern as slash templates). Currently stores **`pinned_agents`** (sidebar Quick open) so pins survive desktop reinstalls that clear WebView `localStorage`.
- **Dashboard:** **Settings** at-a-glance **Config schema** line appends **`⚠ mismatch`** when effective `config_schema_version` ≠ binary constant (`static/js/pages/settings.js`).

### Fixed

- **Dashboard → Chat:** **Agent settings** (gear) opens the Info/Files/Config modal from **inline chat** as well as from the agent picker — single modal in **`agentsPage`** scope (`index_body.html`).

### Changed

- **Release / desktop:** PostHog compile-time env accepts **`ARMARAOS_POSTHOG_KEY`** / **`ARMARAOS_POSTHOG_HOST`** or falls back to **`AINL_POSTHOG_KEY`** / **`AINL_POSTHOG_HOST`** (same `phc_…` as ainativelang.com `NEXT_PUBLIC_POSTHOG_KEY`). See `docs/release-desktop.md`.
- **Desktop bundle:** **`AINL_PYPI_VERSION`** (release workflow), desktop bundle CI, and **`xtask bundle-ainl-wheel`** default pin raised to **`ainativelang` 1.4.3**.

### Documentation

- **Ultra Cost-Efficient Mode:** [docs/prompt-compression-efficient-mode.md](docs/prompt-compression-efficient-mode.md) (canonical); [configuration.md](docs/configuration.md) (`efficient_mode`), [api-reference.md](docs/api-reference.md) (message + WebSocket + config), [dashboard-settings-runtime-ui.md](docs/dashboard-settings-runtime-ui.md) (Budget + chat eco), [dashboard-testing.md](docs/dashboard-testing.md) (QA **7b**), [docs/README.md](docs/README.md), root [README.md](README.md).
- **UI prefs, pinned agents, agent detail modal, config schema mismatch:** [api-reference.md](docs/api-reference.md) (**UI Preferences** section + endpoint summary), [data-directory.md](docs/data-directory.md) (`ui-prefs.json`, `slash-templates.json` row), [troubleshooting.md](docs/troubleshooting.md), [dashboard-settings-runtime-ui.md](docs/dashboard-settings-runtime-ui.md), [configuration.md](docs/configuration.md), [getting-started.md](docs/getting-started.md) (config schema triage), [dashboard-testing.md](docs/dashboard-testing.md) (QA for gear modal + pins), [docs/README.md](docs/README.md), root [CLAUDE.md](CLAUDE.md).
- **Desktop code signing:** [docs/desktop-code-signing.md](docs/desktop-code-signing.md).

## [0.6.9] - 2026-04-08

### Changed

- **Desktop release bundle:** Pinned **`ainativelang`** wheel for Tauri resources to **1.4.2** ([PyPI](https://pypi.org/project/ainativelang/)); **`AINL_PYPI_VERSION`** in [`.github/workflows/release.yml`](.github/workflows/release.yml), the CI desktop bundle step in [`.github/workflows/ci.yml`](.github/workflows/ci.yml), and **`xtask bundle-ainl-wheel`** default now match. (v0.6.8 temporarily used **1.4.1** because **1.4.2** was not yet published.)

## [0.6.8] - 2026-04-08

### Fixed

- **Release / desktop CI:** The **Bundle AINL wheel** step failed because **`ainativelang==1.4.2`** is not published on PyPI (pip reported versions through **1.4.1** only). **`AINL_PYPI_VERSION`** in [`.github/workflows/release.yml`](.github/workflows/release.yml) is pinned to **1.4.1**; [`.github/workflows/ci.yml`](.github/workflows/ci.yml) desktop bundle job and **`xtask bundle-ainl-wheel`** default match. When a newer AINL is uploaded to PyPI, bump this pin and the comment in `release.yml`.

## [0.6.7] - 2026-04-08

### Fixed

- **CI:** Ran `cargo fmt --all` so **`cargo fmt --check`** passes on `main` and tagged releases. The **v0.6.6** tag pointed at a commit that failed the **Format** workflow; **v0.6.7** is the first tag that includes that formatting pass (no intentional runtime behavior change).

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

- **Default models:** Bundled `agents/*/agent.toml`, TUI templates/wizard, and related surfaces align on **OpenRouter** with **`nvidia/nemotron-3-super-120b-a12b:free`** (or provider-appropriate fallbacks) for new-agent defaults.
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

- **LLM resilience:** When rate limited or overloaded after retries, agents automatically attempt OpenRouter free-model fallbacks (see `OPENROUTER_FREE_FALLBACK_MODELS` in `openfang-types`, e.g. Nemotron then Llama 3.1 8B `:free`) to keep the UX flowing.

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

[0.1.0]: https://github.com/sbhooley/armaraos/releases/tag/v0.1.0
[0.7.4]: https://github.com/sbhooley/armaraos/releases/tag/v0.7.4
[0.7.3]: https://github.com/sbhooley/armaraos/releases/tag/v0.7.3
[0.7.2]: https://github.com/sbhooley/armaraos/releases/tag/v0.7.2
[0.7.1]: https://github.com/sbhooley/armaraos/releases/tag/v0.7.1
[0.7.0]: https://github.com/sbhooley/armaraos/releases/tag/v0.7.0
[0.6.9]: https://github.com/sbhooley/armaraos/releases/tag/v0.6.9
[0.6.8]: https://github.com/sbhooley/armaraos/releases/tag/v0.6.8
[0.6.7]: https://github.com/sbhooley/armaraos/releases/tag/v0.6.7
[0.6.6]: https://github.com/sbhooley/armaraos/releases/tag/v0.6.6
[0.6.5]: https://github.com/sbhooley/armaraos/releases/tag/v0.6.5
[0.6.4]: https://github.com/sbhooley/armaraos/releases/tag/v0.6.4
