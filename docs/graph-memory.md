# AINL graph memory (runtime integration)

ArmaraOS records **typed graph nodes** from live agent execution using the standalone **`ainl-memory`** crate (`GraphMemory` + SQLite). This complements **`openfang-memory`** (`data/openfang.db`: sessions, vector/text recall, orchestration trace ring, audit, etc.). Design intent: **execution is the memory**—turns, tools, delegations, and persona traits become graph data for recall, bundles, and future retrieval.

**Primary code:** `crates/openfang-runtime/src/graph_memory_writer.rs` (async-safe wrapper), `crates/ainl-memory/` (store + schema).

---

## On-disk layout

| Path | Purpose |
|------|---------|
| **`~/.armaraos/agents/<agent_id>/ainl_memory.db`** | Per-agent SQLite DB. Parent dirs are created on first open. |

`<agent_id>` is the kernel’s stable agent id string (same value passed to **`GraphMemoryWriter::open`**).

**Overrides:** **`ARMARAOS_HOME`** / **`OPENFANG_HOME`** relocate the whole tree — see [data-directory.md](data-directory.md).

**Scheduled AINL:** cron **`ainl run`** may also use **`bundle.ainlbundle`** JSON plus the Python **`ainl_graph_memory`** bridge; that path is separate from this Rust DB. See [scheduled-ainl.md](scheduled-ainl.md).

---

## What gets written (runtime)

| Source | Node / behavior | Notes |
|--------|-----------------|-------|
| **`run_agent_loop` / streaming** — EndTurn success | **Episode** via **`record_turn`** | Tool names used in the turn; optional trace JSON only where wired (e.g. delegate path). |
| Same loops — after each successful **`execute_tool`** | **Semantic** via **`record_fact`** | Short “tool ran” fact; **`source_turn_id`** follow-up: not yet the parent episode UUID (see *Follow-ups*). |
| **`tool_agent_delegate`** | **Episode** via **`GraphMemoryWriter::record_turn`** | Includes serialized **`OrchestrationTraceEvent`** when JSON serialization succeeds. |
| **`tool_a2a_send`** (after **`A2aClient::send_task`** OK) | **Episode** via **`record_delegation`** | Implemented in **`tool_runner.rs`** (not **`a2a.rs`**) so **`caller_agent_id`** is available. |
| Persona recall (each LLM call setup) | **`GraphMemoryWriter::recall_persona`** → **`[Persona traits active: …]`** on **system prompt** | After manifest prompt, **openfang-memory** recall, and optional **orchestration** appendix, **`run_agent_loop` / `run_agent_loop_streaming`** query **Persona** nodes in the last **90** days with strength ≥ **0.1**, format **`trait (strength=0.xx)`**, append before Ultra Cost-Efficient Mode compression. |

**Non-fatal open:** if home resolution or SQLite creation fails, **`GraphMemoryWriter::open`** returns **`Err`** and the agent loop runs without graph writes.

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
| Wrapper | **`openfang_runtime::graph_memory_writer::GraphMemoryWriter`** — **`open`**, **`record_turn`**, **`record_fact`**, **`record_delegation`**, **`recall_recent`**, **`recall_persona`** |
| Blocking + streaming loops | **`agent_loop.rs`** — writer opened with **`session.agent_id`**. |
| In-process delegation | **`tool_runner.rs`** — **`tool_agent_delegate`** graph write after **`send_to_agent_with_context`**. |
| Outbound A2A | **`tool_runner.rs`** — **`tool_a2a_send`** after **`send_task`**. |
| HTTP client only | **`a2a.rs`** — **`A2aClient::send_task`** (no graph dependency; keeps crate boundaries clean). |

**Tests:** `cargo test -p openfang-runtime graph_memory_writer` (includes **`test_recall_persona_returns_persona_nodes`**).

---

## Follow-ups

1. **`record_fact`**: link **`source_turn_id`** to the episode id produced for the same user turn (today a fresh UUID is used in some paths).
2. **Episodes at prompt time**: optional injection of **`recall_recent`** episode summaries into the system prompt is not implemented yet (persona-only today).

---

## See also

- [data-directory.md](data-directory.md) — path table + migration
- [architecture.md](architecture.md) — crate graph + graph memory subsection
- [mcp-a2a.md](mcp-a2a.md#ainl-graph-memory-outbound-a2a) — A2A send + graph note
- Repo root **[ARCHITECTURE.md](../ARCHITECTURE.md)** — three-layer narrative
- **`crates/ainl-memory/README.md`** — crate-level API
- **[PRIOR_ART.md](../PRIOR_ART.md)** — lineage / attribution
