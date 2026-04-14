# AINL graph memory (runtime integration)

ArmaraOS records **typed graph nodes** from live agent execution using the standalone **`ainl-memory`** crate (`GraphMemory` + SQLite). This complements **`openfang-memory`** (`data/openfang.db`: sessions, vector/text recall, orchestration trace ring, audit, etc.). Design intent: **execution is the memory**—turns, tools, delegations, and persona traits become graph data for recall, bundles, and future retrieval.

**Primary code:** `crates/openfang-runtime/src/graph_memory_writer.rs` (async-safe wrapper), `crates/ainl-memory/` (store + schema).

**Operator quick links:** optional env toggles for richer graph writes are summarized in **[persona-evolution.md](persona-evolution.md)** (persona axis hook) and **`crates/openfang-runtime/README.md`** (extractor + tagger features and the same variables).

**Optional orchestration crate:** **`ainl-runtime`** layers the same SQLite **`GraphMemory`** with a full turn pipeline (memory context, procedural **`PatchAdapter`** dispatch, optional **`ainl-graph-extractor`** scheduling). The live dashboard loop does **not** call it yet; use it for tooling, tests, or future **`openfang-runtime`** embedding. Tokio hosts can enable crate feature **`async`** and **`AinlRuntime::run_turn_async`** (SQLite on **`spawn_blocking`**, graph under **`Arc<std::sync::Mutex<_>>`**). Hub: **[ainl-runtime.md](ainl-runtime.md)** and **`crates/ainl-runtime/README.md`**.

---

## On-disk layout

| Path | Purpose |
|------|---------|
| **`~/.armaraos/agents/<agent_id>/ainl_memory.db`** | Per-agent SQLite DB. Parent dirs are created on first open. |
| **`~/.armaraos/agents/<agent_id>/ainl_graph_memory_inbox.json`** | Append-only envelope from **Python** (`ainativelang` **`AinlMemorySyncWriter`**) for graph nodes the Rust daemon should import into **`ainl_memory.db`**. See *Python inbox (write-back)* below. |
| **`~/.armaraos/agents/<agent_id>/ainl_graph_memory_export.json`** (or under `AINL_GRAPH_MEMORY_ARMARAOS_EXPORT`) | JSON snapshot written after persona evolution passes so Python **`ainl_graph_memory`** can refresh. See **`GraphMemoryWriter::armaraos_graph_memory_export_json_path`** in `graph_memory_writer.rs`. |

`<agent_id>` is the kernel’s stable agent id string (same value passed to **`GraphMemoryWriter::open`**).

**Overrides:** **`ARMARAOS_HOME`** / **`OPENFANG_HOME`** relocate the whole tree — see [data-directory.md](data-directory.md).

**Scheduled AINL:** cron **`ainl run`** may also use **`bundle.ainlbundle`** JSON plus the Python **`ainl_graph_memory`** bridge; that path is separate from this Rust DB. See [scheduled-ainl.md](scheduled-ainl.md).

### Python inbox (write-back)

When **`ARMARAOS_AGENT_ID`** is set, **ainativelang** can append **`MemoryNode`** rows to **`ainl_graph_memory_inbox.json`** (same directory as **`ainl_memory.db`**). On each chat turn, **`run_agent_loop` / `run_agent_loop_streaming`** calls **`GraphMemoryWriter::drain_python_graph_memory_inbox`** immediately after opening graph memory:

1. Read and parse the inbox JSON (`nodes`, `edges`, optional **`source_features`** / **`schema_version`**).
2. Map rows into **`ainl_memory::AgentGraphSnapshot`** and run **`GraphMemory::import_graph(..., allow_dangling_edges = true)`** (same pattern as forensic snapshot import).
3. Reset the inbox to an empty envelope so Python can append again.

**Capability hints:** if the inbox lists **`requires_ainl_tagger`** under **`source_features`** but this binary was built without the **`ainl-tagger`** feature, semantic nodes with non-empty **`tags`** are skipped (logged at **debug**). Default ArmaraOS builds include **`ainl-tagger`**; distributors can disable it via **`openfang-runtime`** features. Even when the feature is present, **runtime** tagging for **Rust-originated** episode/fact writes only runs when **`AINL_TAGGER_ENABLED=1`** (see **`crates/openfang-runtime/README.md`**).

**Schema (cross-repo):** **ainativelang** ships **`armaraos/bridge/ainl_graph_memory_inbox_schema_v1.json`** and a CI workflow that type-checks against upstream **armaraos** (`cargo build -p openfang-runtime --lib`).

---

## What gets written (runtime)

| Source | Node / behavior | Notes |
|--------|-----------------|-------|
| **`run_agent_loop` / streaming** — EndTurn success | **Episode** via **`record_turn`** | Tool names used in the turn; optional trace JSON only where wired (e.g. delegate path). Episode **`tags`** may include semantic tagger output when **`ainl-tagger`** + **`AINL_TAGGER_ENABLED=1`**. |
| Same — after **`record_turn`** | **Semantic** via **`record_fact_with_tags`** | **`graph_memory_turn_extraction`** picks structured **`ainl_graph_extractor_bridge`** vs legacy **`graph_extractor`** based on **`AINL_EXTRACTOR_ENABLED`** (requires **`ainl-extractor`**). Fact tag lists merge orchestration correlation strings + optional **`SemanticTaggerBridge::tag_fact`**. **`source_turn_id`** is the **episode** UUID returned from **`record_turn`**. |
| Same — after facts | **Procedural** via **`record_pattern`** (optional) | When a workflow or repeated-tool pattern is detected; may carry orchestration **`trace_id`**. |
| **`tool_agent_delegate`** | **Episode** via **`GraphMemoryWriter::record_turn`** | Includes serialized **`OrchestrationTraceEvent`** when JSON serialization succeeds. |
| **`tool_a2a_send`** (after **`A2aClient::send_task`** OK) | **Episode** via **`record_delegation`** | Implemented in **`tool_runner.rs`** (not **`a2a.rs`**) so **`caller_agent_id`** is available. |
| Persona recall (each LLM call setup) | **`GraphMemoryWriter::recall_persona`** → **`[Persona traits active: …]`** on **system prompt** | After manifest prompt, **openfang-memory** recall, and optional **orchestration** appendix, **`run_agent_loop` / `run_agent_loop_streaming`** query **Persona** nodes in the last **90** days with strength ≥ **0.1**, format **`trait (strength=0.xx)`**, append before Ultra Cost-Efficient Mode compression. |
| Post-turn (spawned, after EndTurn writes) | **`run_persona_evolution_pass`** | Runs **`ainl-graph-extractor`** when the **`ainl-extractor`** Cargo feature is on (default): semantic **`recurrence_count`** bumps, merged **`RawSignal`** ingest, optional persona snapshot write, cold-graph **`correction_tick`** when enabled (**`ainl-persona-evolution`**). Then refreshes the ArmaraOS graph JSON export path above. |
| Post-turn (same spawn, optional) | **`PersonaEvolutionHook::evolve_from_turn`** | When **`AINL_PERSONA_EVOLUTION=1`** and **`ainl-persona-evolution`** is compiled in, layers explicit tool / delegation signals on the latest axis snapshot so evolution still moves when episode **`trace_event.outcome`** is missing. Failures are logged only. See **[persona-evolution.md](persona-evolution.md)**. |

**Non-fatal open:** if home resolution or SQLite creation fails, **`GraphMemoryWriter::open`** returns **`Err`** and the agent loop runs without graph writes.

---

## Optional extraction and tagging (env + features)

These control **extra** graph richness on top of the always-on episode + heuristic path. All are **opt-in at runtime** except where noted.

| Variable | Cargo feature | When `1` / truthy (`true`, `yes`, `on`) |
|----------|---------------|----------------------------------------|
| **`AINL_EXTRACTOR_ENABLED`** | **`ainl-extractor`** (default) | EndTurn fact + pattern extraction uses **`ainl_graph_extractor_bridge`** (published **`ainl-graph-extractor`** pipeline) instead of the legacy **`graph_extractor`** heuristics. Persona evolution pass still runs when the feature is enabled. |
| **`AINL_TAGGER_ENABLED`** | **`ainl-tagger`** (optional in some builds) | Episode and fact nodes get **`SemanticTaggerBridge`** tag lists at write time. |
| **`AINL_PERSONA_EVOLUTION`** | **`ainl-persona-evolution`** (default) | After **`run_persona_evolution_pass`**, runs the incremental **`PersonaEvolutionHook::evolve_from_turn`** write (see **[persona-evolution.md](persona-evolution.md)**). |

Slim builds: **`cargo build -p openfang-runtime --no-default-features --features ainl-persona-evolution`** (see crate README).

---

## Orchestration traces vs graph memory

| Subsystem | Storage | UI / API |
|-----------|---------|----------|
| **Orchestration traces** | Kernel / **`openfang-memory`** ring + APIs | Dashboard **`#orchestration-traces`**, **`GET /api/orchestration/traces`**, SSE |
| **AINL graph** | Per-agent **`ainl_memory.db`** | No dedicated dashboard page yet; query via **`ainl-memory`** APIs / future tooling |

They are **different** stores; correlating IDs (e.g. **`trace_id`**) is intentional for cross-debugging.

---

## Developer map

| Area | File / symbol |
|------|----------------|
| Wrapper | **`openfang_runtime::graph_memory_writer::GraphMemoryWriter`** — **`open`**, **`drain_python_graph_memory_inbox`**, **`record_turn`**, **`record_fact`**, **`record_delegation`**, **`recall_recent`**, **`recall_persona`**, **`recall_persona_for_agent`**, **`run_persona_evolution_pass`**, **`export_graph_json`** |
| Optional turn orchestration | **`ainl_runtime::AinlRuntime`** — **`run_turn`**, **`run_turn_async`** (feature **`async`**); not used by the daemon loop today — **[ainl-runtime.md](ainl-runtime.md)** |
| Graph extractor bridge | **`openfang_runtime::ainl_graph_extractor_bridge`** — turn payload formatting, **`graph_memory_turn_extraction`**, **`ainl_extractor_runtime_enabled`** |
| Semantic tagger bridge | **`openfang_runtime::ainl_semantic_tagger_bridge::SemanticTaggerBridge`** — **`tag_episode`**, **`tag_fact`**, gated by **`AINL_TAGGER_ENABLED`** |
| Persona turn hook | **`openfang_runtime::persona_evolution`** — **`PersonaEvolutionHook`**, **`TurnOutcome`**, **`persona_turn_evolution_env_enabled`** |
| Legacy heuristics | **`graph_extractor.rs`** — used when **`AINL_EXTRACTOR_ENABLED`** is unset or the **`ainl-extractor`** feature is off |
| Python inbox import | **`openfang_runtime::ainl_inbox_reader::drain_inbox`** — invoked from **`GraphMemoryWriter::drain_python_graph_memory_inbox`**. |
| Blocking + streaming loops | **`agent_loop.rs`** — writer opened with **`session.agent_id`**; EndTurn graph block + spawned evolution + optional persona hook |
| In-process delegation | **`tool_runner.rs`** — **`tool_agent_delegate`** graph write after **`send_to_agent_with_context`**. |
| Outbound A2A | **`tool_runner.rs`** — **`tool_a2a_send`** after **`send_task`**. |
| HTTP client only | **`a2a.rs`** — **`A2aClient::send_task`** (no graph dependency; keeps crate boundaries clean). |

**Tests:** `cargo test -p openfang-runtime graph_memory_writer` (includes **`test_recall_persona_returns_persona_nodes`**). With default features, **`cargo test -p openfang-runtime test_persona_strength_increases_after_repeated_tool`** covers the persona turn hook (**`AINL_PERSONA_EVOLUTION=1`** inside the test). Bridge tests live under **`crates/openfang-runtime/src/tests/`** when **`ainl-tagger`** / **`ainl-extractor`** are enabled. For **`ainl-runtime`**: **`cargo test -p ainl-runtime`** and **`cargo test -p ainl-runtime --features async`** (see **[ainl-runtime.md](ainl-runtime.md)**).

---

## Follow-ups

1. **Episodes at prompt time**: optional injection of **`recall_recent`** episode summaries into the system prompt is not implemented yet (persona-only today).
2. **Inbox ingest on the Rust side**: confirm your ArmaraOS build includes the kernel / runtime component that drains **`ainl_graph_memory_inbox.json`** into SQLite if you rely on Python **`AinlMemorySyncWriter`** in production.

---

## See also

- [ainl-runtime GraphPatch + patches](ainl-runtime-graph-patch.md) — **`PatchAdapter`** / **`GraphPatchAdapter`**, semantic ranking migration, crates.io version matrix
- [ainl-runtime in OpenFang (optional)](ainl-runtime-integration.md) — feature **`ainl-runtime-engine`**, **`AinlRuntimeBridge`**, **`TurnOutcome`** mapping
- **`crates/ainl-runtime/README.md`** — crate hub (`run_turn` / **`run_turn_async`**, **`async`** feature, `cargo test -p ainl-runtime`)
- **ainativelang:** [`AINL_GRAPH_MEMORY.md`](https://github.com/sbhooley/ainativelang/blob/main/docs/adapters/AINL_GRAPH_MEMORY.md) — Python **`GraphStore`**, export merge, **`AinlMemorySyncWriter`** / inbox envelope
- [persona-evolution.md](persona-evolution.md) — axis snapshot hook (**`AINL_PERSONA_EVOLUTION`**), **`ainl-persona-evolution`** feature
- **`crates/openfang-runtime/README.md`** — **`AINL_EXTRACTOR_ENABLED`**, **`AINL_TAGGER_ENABLED`**, default Cargo features
- [data-directory.md](data-directory.md) — path table + migration
- [architecture.md](architecture.md) — crate graph + graph memory subsection
- [mcp-a2a.md](mcp-a2a.md#ainl-graph-memory-outbound-a2a) — A2A send + graph note
- Repo root **[ARCHITECTURE.md](../ARCHITECTURE.md)** — three-layer narrative
- **`crates/ainl-memory/README.md`** — crate-level API
- **[PRIOR_ART.md](../PRIOR_ART.md)** — lineage / attribution
