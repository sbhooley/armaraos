# AINL graph memory (runtime integration)

ArmaraOS records **typed graph nodes** from live agent execution using the standalone **`ainl-memory`** crate (`GraphMemory` + SQLite). This complements **`openfang-memory`** (`data/openfang.db`: sessions, vector/text recall, orchestration trace ring, audit, etc.). Design intent: **execution is the memory**—turns, tools, delegations, and persona traits become graph data for recall, bundles, and future retrieval.

**Primary code:** `crates/openfang-runtime/src/graph_memory_writer.rs` (async-safe wrapper), `crates/ainl-memory/` (store + schema).

**Operator quick links:** optional env toggles for richer graph writes are summarized in **[persona-evolution.md](persona-evolution.md)** (persona axis hook) and **`crates/openfang-runtime/README.md`** (extractor + tagger features and the same variables). Python inbox (**`ARMARAOS_AGENT_ID`**): **[graph-memory-sync.md](graph-memory-sync.md)**. Pre-release human sign-off (controls, gates, scope, evaluation): **[ga-signoff-checklist.md](ga-signoff-checklist.md)**.

**Further reading (narrative + timeline):** [When Your AI Agent Actually Remembers: Introducing AINL’s Graph-as-Memory Architecture](https://ainativelang.com/blog/graph-as-memory-architecture-ainl) on ainativelang.com (Python AINL, Rust `ainl-*`, ArmaraOS). Verified chronology and citations: **[`PRIOR_ART.md`](../PRIOR_ART.md)** in this repository.

**Optional orchestration crate:** **`ainl-runtime`** layers the same SQLite **`GraphMemory`** with a full turn pipeline (memory context, procedural **`PatchAdapter`** dispatch, optional **`ainl-graph-extractor`** scheduling). Nested **`run_turn`** / **`run_turn_async`** depth is enforced **inside** the runtime (**`RuntimeConfig::max_delegation_depth`**, default **8**; overruns are **`AinlRuntimeError::DelegationDepthExceeded`**, not a soft **`TurnStatus`**). The **default** dashboard chat path remains **`GraphMemoryWriter`** + OpenFang loop fallback; `openfang-runtime` now ships with **`ainl-runtime-engine`** in default builds and routes a turn through **`AinlRuntime::run_turn`** when runtime switches are active (`ainl_runtime_engine` / `AINL_RUNTIME_ENGINE=1`) — **[ainl-runtime-integration.md](ainl-runtime-integration.md)**. Use the crate standalone for tests and tooling. Tokio hosts can enable crate feature **`async`** and **`AinlRuntime::run_turn_async`** (SQLite on **`spawn_blocking`**, graph under **`Arc<std::sync::Mutex<_>>`**). Hub: **[ainl-runtime.md](ainl-runtime.md)** (*Orientation FAQ* for MCP/CLI/Python overlap) and **`crates/ainl-runtime/README.md`**.

---

## On-disk layout

| Path | Purpose |
|------|---------|
| **`~/.armaraos/agents/<agent_id>/ainl_memory.db`** | Per-agent SQLite DB. Parent dirs are created on first open. |
| **`~/.armaraos/agents/<agent_id>/ainl_graph_memory_inbox.json`** | Append-only envelope from **Python** (`ainativelang` **`AinlMemorySyncWriter`**) for graph nodes the Rust daemon should import into **`ainl_memory.db`**. See *Python inbox (write-back)* below. |
| **`~/.armaraos/agents/<agent_id>/ainl_graph_memory_export.json`** (default when **`AINL_GRAPH_MEMORY_ARMARAOS_EXPORT`** is unset) **or** **`$AINL_GRAPH_MEMORY_ARMARAOS_EXPORT/<agent_id>_graph_export.json`** when the env is set (value is always treated as a **directory** on the Rust side) | JSON snapshot written after persona evolution passes so Python **`ainl_graph_memory`** can refresh without one global file clobbering other agents. Resolver: **`openfang_runtime::graph_memory_writer::armaraos_graph_memory_export_json_path`** (`graph_memory_writer.rs`). **Python** accepts the same env as either a directory (per-agent **`{ARMARAOS_AGENT_ID}_graph_export.json`**) or a single **`.json`** file path — see **ainativelang** [`docs/adapters/AINL_GRAPH_MEMORY.md`](https://github.com/sbhooley/ainativelang/blob/main/docs/adapters/AINL_GRAPH_MEMORY.md). |

`<agent_id>` is the kernel’s stable agent id string (same value passed to **`GraphMemoryWriter::open`**).

**Overrides:** **`ARMARAOS_HOME`** / **`OPENFANG_HOME`** relocate the whole tree — see [data-directory.md](data-directory.md).

**Scheduled AINL:** cron **`ainl run`** may also use **`bundle.ainlbundle`** JSON plus the Python **`ainl_graph_memory`** bridge; that path is separate from this Rust DB. See [scheduled-ainl.md](scheduled-ainl.md).

### Python inbox (write-back)

When **`ARMARAOS_AGENT_ID`** is set, **ainativelang** can append **`MemoryNode`** rows to **`ainl_graph_memory_inbox.json`** (same directory as **`ainl_memory.db`**). On each chat turn, **`run_agent_loop` / `run_agent_loop_streaming`** calls **`GraphMemoryWriter::drain_python_graph_memory_inbox`** immediately after opening graph memory:

1. Read and parse the inbox JSON (`nodes`, `edges`, optional **`source_features`** / **`schema_version`**).
2. Map rows into **`ainl_memory::AgentGraphSnapshot`** and run **`GraphMemory::import_graph(..., allow_dangling_edges = true)`** (same pattern as forensic snapshot import).
3. Reset the inbox to an empty envelope so Python can append again.

**Capability hints:** if the inbox lists **`requires_ainl_tagger`** under **`source_features`** but this binary was built without the **`ainl-tagger`** feature, semantic nodes with non-empty **`tags`** are skipped (logged at **debug**). Default ArmaraOS builds include **`ainl-tagger`**; distributors can disable it via **`openfang-runtime`** features. When the feature is present, **runtime** tagging for Rust-originated episode/fact writes is **enabled by default**; opt out with **`AINL_TAGGER_ENABLED=0`** (see **`crates/openfang-runtime/README.md`**).

**Schema (cross-repo):** **ainativelang** ships **`armaraos/bridge/ainl_graph_memory_inbox_schema_v1.json`** and a CI workflow that type-checks against upstream **armaraos** (`cargo build -p openfang-runtime --lib`).

---

## What gets written (runtime)

| Source | Node / behavior | Notes |
|--------|-----------------|-------|
| **`run_agent_loop` / streaming** — EndTurn success | **Episode** via **`record_turn`** | Canonical tool names for the turn; optional trace JSON when wired (e.g. orchestration). Episode **`tags`** include tagger strings when **`ainl-tagger`** is compiled in (default on); opt out via **`AINL_TAGGER_ENABLED=0`**. |
| Same — after **`record_turn`** | **Semantic** via **`record_fact_with_tags`** | **`graph_memory_turn_extraction`** picks structured **`ainl_graph_extractor_bridge`** vs legacy **`graph_extractor`** based on **`AINL_EXTRACTOR_ENABLED`** (requires **`ainl-extractor`**). Fact tag lists merge orchestration correlation strings + optional **`SemanticTaggerBridge::tag_fact`**. **`source_turn_id`** is the **episode** UUID returned from **`record_turn`**. |
| Same — after facts | **Procedural** via **`record_pattern`** (optional) | When a workflow or repeated-tool pattern is detected; may carry orchestration **`trace_id`**. |
| Same — after patterns (same `record_turn` success path) | **Trajectory** via **`record_trajectory_for_episode`** | One **`Trajectory`** row per turn with coarse **`TrajectoryStep`** entries derived from canonical tool names; edge **`trajectory_of`** → episode node id. Gated by **`AINL_TRAJECTORY_ENABLED`** (**opt-out**, same falsy set as **`AINL_EXTRACTOR_ENABLED`**). Optional **`AINL_MEMORY_PROJECT_ID`** supplies `project_id` on the node when set. |
| **`tool_agent_delegate`** | **Episode** via **`GraphMemoryWriter::record_turn`** | Includes serialized **`OrchestrationTraceEvent`** when JSON serialization succeeds. |
| **`tool_a2a_send`** (after **`A2aClient::send_task`** OK) | **Episode** via **`record_delegation`** | Implemented in **`tool_runner.rs`** (not **`a2a.rs`**) so **`caller_agent_id`** is available. |
| Persona recall (each LLM call setup) | **`GraphMemoryWriter::recall_persona`** → **`[Persona traits active: …]`** on **system prompt** | After manifest prompt, **openfang-memory** recall, and optional **orchestration** appendix, **`run_agent_loop` / `run_agent_loop_streaming`** query **Persona** nodes in the last **90** days with strength ≥ **0.1**, format **`trait (strength=0.xx)`**, append before Ultra Cost-Efficient Mode compression. |
| Post-turn (spawned, after EndTurn writes) | **`run_persona_evolution_pass`** → **`ainl_graph_extractor::ExtractionReport`** | Runs **`GraphExtractorTask::run_pass`** when the **`ainl-extractor`** Cargo feature is on (default): semantic **`recurrence_count`** bumps, merged **`RawSignal`** ingest, optional persona snapshot write, cold-graph **`correction_tick`** when enabled (**`ainl-persona-evolution`**). The method returns a structured report (not `Result<(), String>`): **`extract_error`**, **`pattern_error`**, and **`persona_error`** surface partial failures; **`has_errors()`** is the single guard. OpenFang **`warn!`**s each populated slot (signal merge vs pattern flush vs persona write). **`AinlRuntime::run_turn`** on the same DB maps the same three fields to **`TurnPhase`** **`TurnWarning`**s — see **[ainl-runtime.md](ainl-runtime.md)** (*Persona evolution pass*). Then refreshes the ArmaraOS graph JSON export path above. |
| Post-turn (same spawn, optional) | **`PersonaEvolutionHook::evolve_from_turn`** | When **`AINL_PERSONA_EVOLUTION=1`** and **`ainl-persona-evolution`** is compiled in, layers explicit tool / delegation signals on the latest axis snapshot so evolution still moves when episode **`trace_event.outcome`** is missing. Failures are logged only. See **[persona-evolution.md](persona-evolution.md)**. |
| **MCP AINL session (learning path)** | **Semantic** (hash-gated) | When **`mcp_ainl_ainl_get_started`** (or an updated wizard JSON) changes digest, a fact tagged **`mcp:ainl:wizard_state`** with **`v:<sha16>`** may be written for **`read_wizard_state_from_graph`**. Pairs with **`mcp:ainl:capabilities`** / **`mcp:ainl:recommended_next`**. See **[mcp-a2a.md](mcp-a2a.md)** (*AINL MCP digest*); in-repo map: **ainativelang** `docs/operations/MCP_AINL_WIZARD_AND_CORPUS.md`. |
| **Optional `ainl-runtime`** on the same DB | **`runtime_state`** (**`RuntimeStateNode`**) | When **`AinlRuntime`** opens this **`ainl_memory.db`**, it upserts one stable row per agent with **`turn_count`**, **`last_extraction_at_turn`**, optional **`persona_snapshot_json`**, and **`updated_at`**. **`GraphMemoryWriter`** does not write this node; WAL coexists with OpenFang’s writers. Deleting **`ainl_memory.db`** clears it with the rest of the graph. |

**Non-fatal open:** if home resolution or SQLite creation fails, **`GraphMemoryWriter::open`** returns **`Err`** and the agent loop runs without graph writes.

---

## Extraction, tagging, and persona evolution (env + features)

These control **extra** graph richness on top of the always-on **episode** row. `AINL_EXTRACTOR_ENABLED`, `AINL_PERSONA_EVOLUTION`, and `AINL_TAGGER_ENABLED` are **opt-out** (on by default when their Cargo features are compiled in).

| Variable | Cargo feature | Semantics |
|----------|---------------|-----------|
| **`AINL_EXTRACTOR_ENABLED`** | **`ainl-extractor`** (default) | **Opt-out.** When the feature is compiled in, the crate path (`ainl_graph_extractor_bridge`) is **on by default**. Set to a falsy value (**`0`**, **`false`**, **`no`**, **`off`**, case-insensitive) to fall back to legacy `graph_extractor` heuristics. `run_persona_evolution_pass` does **not** read this env var. |
| **`AINL_TAGGER_ENABLED`** | **`ainl-tagger`** (default in ArmaraOS) | **Opt-out.** Set to a falsy value (`0`, `false`, `no`, `off`) to disable tag writes from `SemanticTaggerBridge`. When unset (normal case) or set to any other value, tagging stays enabled. |
| **`AINL_PERSONA_EVOLUTION`** | **`ainl-persona-evolution`** (default) | **Opt-out.** When the feature is compiled in, `PersonaEvolutionHook::evolve_from_turn` runs after each turn by default. Set to a falsy value (**`0`**, **`false`**, **`no`**, **`off`**) to disable (see **[persona-evolution.md](persona-evolution.md)**). |
| **`AINL_TRAJECTORY_ENABLED`** | n/a (always compiled) | **Opt-out** for **`Trajectory`** nodes after **`record_turn`**. Unset ⇒ on; falsy (**`0`**, **`false`**, **`no`**, **`off`**) ⇒ off. See **`crates/openfang-runtime/README.md`**. |
| **`AINL_IMPROVEMENT_PROPOSALS_ENABLED`** | n/a | **Opt-out** for the improvement-proposal **HTTP** surface and **ledger** (`~/.armaraos/agents/<id>/.graph-memory/improvement_proposals.db`). Unset ⇒ on; falsy (**`0`**, **`false`**, **`no`**, **`off`**) ⇒ off (list/submit/validate/adopt return **503**). See **[SELF_LEARNING_INTEGRATION_MAP.md](SELF_LEARNING_INTEGRATION_MAP.md) §15.7**. |
| **`AINL_MEMORY_PROJECT_ID`** | n/a | Optional string stored on **`Trajectory`** payloads when trajectory writes run; helps future multi-project scoping. |

Slim builds: **`cargo build -p openfang-runtime --no-default-features --features ainl-persona-evolution`** (see crate README).

---

## Orchestration traces vs graph memory

| Subsystem | Storage | UI / API |
|-----------|---------|----------|
| **Orchestration traces** | Kernel / **`openfang-memory`** ring + APIs | Dashboard **`#orchestration-traces`**, **`GET /api/orchestration/traces`**, SSE |
| **AINL graph** | Per-agent **`ainl_memory.db`** | Dashboard **Graph Memory** (`#graph-memory`): **`GET /api/graph-memory`** returns nodes with **`explain`** (what / why / evidence / typed edges) **`SystemEvent::GraphMemoryWrite`** SSE events include optional **`provenance`** (summary, node ids, reason). See **[GRAPH_MEMORY_EXPLAINABILITY.md](GRAPH_MEMORY_EXPLAINABILITY.md)**. |
| **Trajectories** (detail table) | Same DB: table **`ainl_trajectories`** (see *What gets written*) | **`GET /api/trajectories?agent_id=`**; operator dashboard **`#trajectories`** + SSE (**`TrajectoryRecorded`**, **`GraphMemoryWrite` trajectory**, cross-feed **`FailureLearned` / failure `GraphMemoryWrite`**, and headline `GET /api/graph-memory/failures/recent`); offline CLI **`openfang trajectory list`**, **`search`**, **`analyze`**, **`export`** (same `ainl_memory.db`; list/search/analyze support `--json`; export emits JSONL replay lines). **Learning failures** tab: **`#graph-failures`**. **Improvement proposals** tab: **`#graph-proposals`**. |

They are **different** stores; correlating IDs (e.g. **`trace_id`**) is intentional for cross-debugging.

---

## Developer map

| Area | File / symbol |
|------|----------------|
| Wrapper | **`openfang_runtime::graph_memory_writer::GraphMemoryWriter`** — **`open`**, **`drain_python_graph_memory_inbox`**, **`record_turn`**, **`record_fact`** / **`record_fact_with_tags`**, **`record_pattern`**, **`record_trajectory_for_episode`**, **`record_delegation`**, **`recall_recent`**, **`recall_persona`**, **`recall_persona_for_agent`**, **`run_persona_evolution_pass`** → **`ainl_graph_extractor::ExtractionReport`**, **`export_graph_json`**, **`emit_write_observed`** (live-event bridge for secondary writers), plus free function **`armaraos_graph_memory_export_json_path`** (per-agent JSON path for **`AINL_GRAPH_MEMORY_ARMARAOS_EXPORT`** / default **`ainl_graph_memory_export.json`**) |
| Trajectory + graph CLI (operator) | **`openfang-cli`** — trajectories: `openfang trajectory list|search|analyze|export …` (same `ainl_memory.db`). Graph memory: `openfang memory graph-export|graph-search|graph-persona|graph-validate|graph-audit|graph-inspect|graph-remember|graph-forget …` (`graph-export` / `graph-validate` offline on `ainl_memory.db`; other graph subcommands use the daemon’s `/api/graph-memory*`). KV: `openfang memory list|get|set|delete` (daemon `/api/memory/...`). |
| Prompt compression CLI (operator) | **`openfang-cli`** — `openfang compression test|score|detect` plus **`profiles`** (`list`, `show`, `map-project`), **`adaptive suggest`**, **`cache ttl|policy`** — all call **`ainl-compression`** (`profiles` / `adaptive` / `cache` modules + core compressor). No daemon. |
| Optional turn orchestration | **`ainl_runtime::AinlRuntime`** — **`run_turn`**, **`run_turn_async`** (feature **`async`**); may persist **`runtime_state`** in the same DB when embedded — not the default daemon loop — **`crates/ainl-runtime/README.md`**, **[ainl-runtime-integration.md](ainl-runtime-integration.md)**. When this path records episodes, **`tools_invoked`** are **canonicalized** at write time (**`ainl-semantic-tagger`**); episode **ids** in turn results are **graph node ids** (see **[ainl-runtime.md](ainl-runtime.md)** *Episodic tools* / *Episode identity*). The host emits an observed **`GraphMemoryWrite`** signal after successful runtime-engine prelude turns so the dashboard live timeline reflects secondary-writer mutations promptly. |
| Graph extractor bridge | **`openfang_runtime::ainl_graph_extractor_bridge`** — turn payload formatting, **`graph_memory_turn_extraction`**, **`ainl_extractor_runtime_enabled`** |
| Semantic tagger bridge | **`openfang_runtime::ainl_semantic_tagger_bridge::SemanticTaggerBridge`** — **`tag_episode`**, **`tag_fact`**, gated by **`AINL_TAGGER_ENABLED`** |
| Persona turn hook | **`openfang_runtime::persona_evolution`** — **`PersonaEvolutionHook`**, **`TurnOutcome`**, **`persona_turn_evolution_env_enabled`** (opt-out; default on) |
| Cognitive vitals | **`openfang_runtime::vitals_classifier`** — `classify_vitals`, `CognitiveVitals` (`gate`, `phase`, `trust`, `mean_logprob`, `entropy`); logprobs-based, fail-open (no logprobs → `None`). Used by OpenAI driver; vitals stored on `EpisodicNode` and propagated to persona signals + AINL frame. |
| Legacy heuristics | **`graph_extractor.rs`** — fallback when `AINL_EXTRACTOR_ENABLED` is set to a falsy value, or when the **`ainl-extractor`** feature is off; also fires when the crate path returns no candidates |
| Python inbox import | **`openfang_runtime::ainl_inbox_reader::drain_inbox`** — invoked from **`GraphMemoryWriter::drain_python_graph_memory_inbox`**. |
| Blocking + streaming loops | **`agent_loop.rs`** — writer opened with **`session.agent_id`**; EndTurn graph block + spawned evolution + optional persona hook |
| In-process delegation | **`tool_runner.rs`** — **`tool_agent_delegate`** graph write after **`send_to_agent_with_context`**. |
| Outbound A2A | **`tool_runner.rs`** — **`tool_a2a_send`** after **`send_task`**. |
| HTTP client only | **`a2a.rs`** — **`A2aClient::send_task`** (no graph dependency; keeps crate boundaries clean). |

**Tests:** `cargo test -p openfang-runtime graph_memory_writer` (includes **`test_recall_persona_returns_persona_nodes`**). **`cargo test -p openfang-runtime --test armaraos_graph_export_json_path`** — per-agent export paths under a shared directory + default layout when **`AINL_GRAPH_MEMORY_ARMARAOS_EXPORT`** is unset. With default features, **`cargo test -p openfang-runtime test_persona_strength_increases_after_repeated_tool`** covers the persona turn hook (no env var needed — on by default). **`cargo test -p openfang-runtime --test test_persona_evolution`** covers opt-out, mismatch, noop. **`cargo test -p openfang-runtime --test test_graph_extractor`** covers the crate-primary extraction path and fallback. Semantic tagger bridge unit tests: **`cargo test -p openfang-runtime --lib --features ainl-tagger test_tag_fact_returns_strings`** (and **`test_tag_episode_from_tool_sequence`**). Extractor bridge integration tests: **`crates/openfang-runtime/src/tests/ainl_graph_extractor_bridge.rs`** when **`ainl-extractor`** is enabled. For **`ainl-runtime`**: **`cargo test -p ainl-runtime`** and **`cargo test -p ainl-runtime --features async`** (see **[ainl-runtime.md](ainl-runtime.md)** and **`crates/ainl-runtime/README.md`**).

---

## Prompt-time memory blocks

OpenFang now assembles bounded graph-memory context sections during prompt build (non-streaming + streaming):

- **`RecentAttempts`** — recent episodic/tool summaries (session-first with strict caps)
- **`KnownFacts`** — ranked semantic facts (`confidence`, `recurrence_count`, `reference_count`, recency)
- **`KnownConflicts`** — contradiction notes for high-confidence conflicting semantic rows
- **`SuggestedProcedure`** — advisory procedural hints (`fitness`/`success_rate`, non-retired only)

All sections are fail-closed (low-quality memory is skipped), include conservative truncation, and may be disabled by temporary mode.

### Temporary mode and defaults

- **Default**: memory context injection is on with conservative limits.
- **Temporary mode** (`memory_temporary_mode` metadata or `AINL_MEMORY_TEMPORARY_MODE` env): disables graph-memory recall and graph-memory writes for that turn path.
- **Global gate**: `AINL_MEMORY_ENABLED` can disable memory context integration quickly.
- **Rollout gate**: `AINL_MEMORY_ROLLOUT` supports `off`, `internal`, `opt_in`, `default`.
 - `internal` requires manifest metadata `memory_internal_agent = true`.
 - `opt_in` requires manifest metadata `memory_opt_in = true` (or internal).

### Background consolidation

After turn writes, runtime schedules a lightweight background consolidation pass that removes
duplicate semantic rows (same normalized fact text) for the same agent while preserving the
highest-confidence row. The pass is rate-limited per agent.

### Control-plane endpoints (dashboard/API)

- `GET/PUT /api/graph-memory/controls`
- `POST /api/graph-memory/remember`
- `POST /api/graph-memory/forget`
- `GET /api/graph-memory/inspect`
- `GET /api/graph-memory/what-do-you-remember` (alias)
- `POST /api/graph-memory/clear-scope`

`controls` now supports per-block kill switches:
- `include_episodic_hints`
- `include_semantic_facts`
- `include_conflicts`
- `include_procedural_hints`

### GA provenance gate metrics

`GET /api/status` includes (among other top-level fields documented in [api-reference.md](api-reference.md)) `graph_memory_context_metrics` with:
- `provenance_coverage_ratio`
- `provenance_coverage_floor`
- `provenance_coverage_min_lines`
- `provenance_gate_pass`
- `conflict_ratio`
- `conflict_ratio_max`
- `conflict_ratio_min_semantic`
- `contradiction_gate_pass`

Defaults: floor `0.95` and min sampled lines `20` (override with
`AINL_MEMORY_PROVENANCE_COVERAGE_FLOOR` and `AINL_MEMORY_PROVENANCE_MIN_LINES`).
Contradiction ratio default max is `0.75` once semantic sample size reaches `20`
(override with `AINL_MEMORY_CONFLICT_RATIO_MAX` and
`AINL_MEMORY_CONFLICT_RATIO_MIN_SEMANTIC`).

---

## See also

- [dashboard-overview-ui.md](dashboard-overview-ui.md#measured-vs-estimated-savings) — same **`GET /api/status`** response also includes **`eco_compression`** and **`quota_enforcement`** (7d) for Get started **est.** savings; orthogonal to graph-memory KPIs
- [graph-memory-sync.md](graph-memory-sync.md) — Python **`AinlMemorySyncWriter`** → **`ainl_graph_memory_inbox.json`** (when **`ARMARAOS_AGENT_ID`**), envelope + CI
- [ainl-runtime.md](ainl-runtime.md) — doc hub (links crate README, GraphPatch, OpenFang integration, verification)
- **`crates/ainl-runtime/README.md`** — crate hub (`run_turn` / **`run_turn_async`**, session **`runtime_state`**, **`async`** feature, `cargo test -p ainl-runtime`)
- [ainl-runtime GraphPatch + patches](ainl-runtime-graph-patch.md) — **`PatchAdapter`** / **`GraphPatchAdapter`**, semantic ranking migration, **`RuntimeStateNode`** persistence, crates.io version matrix
- [ainl-runtime in OpenFang (optional)](ainl-runtime-integration.md) — feature **`ainl-runtime-engine`**, **`AinlRuntimeBridge`**, **`TurnOutcome`** mapping
- **ainativelang:** [`AINL_GRAPH_MEMORY.md`](https://github.com/sbhooley/ainativelang/blob/main/docs/adapters/AINL_GRAPH_MEMORY.md) — Python **`GraphStore`**, export merge, **`AinlMemorySyncWriter`** / inbox envelope
- [persona-evolution.md](persona-evolution.md) — axis snapshot hook (**`AINL_PERSONA_EVOLUTION`**), **`ainl-persona-evolution`** feature
- **`crates/openfang-runtime/README.md`** — **`AINL_EXTRACTOR_ENABLED`**, **`AINL_TAGGER_ENABLED`**, default Cargo features
- [data-directory.md](data-directory.md) — path table + migration
- [architecture.md](architecture.md) — crate graph + graph memory subsection
- [mcp-a2a.md](mcp-a2a.md#ainl-graph-memory-outbound-a2a) — A2A send + graph note
- Repo root **[ARCHITECTURE.md](../ARCHITECTURE.md)** — three-layer narrative
- **`crates/ainl-memory/README.md`** — crate-level API
- **[PRIOR_ART.md](../PRIOR_ART.md)** — lineage / attribution
