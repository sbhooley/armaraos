# ArmaraOS Architecture - Three-Layer Lineage

This document describes the architectural layering of ArmaraOS, from upstream OpenFang foundations through ArmaraOS enhancements to the AINL graph-memory substrate.

---

## Layer 1: OpenFang (Upstream Foundation)

**Source**: OpenFang open-source agent operating system  
**License**: Apache-2.0 OR MIT  
**Repository**: https://github.com/sbhooley/armaraos (forked from OpenFang)

### Core Components

- **openfang-types**: Core type definitions for agents, tools, memory, events
- **openfang-memory**: SQLite-backed memory substrate (episodic, semantic, knowledge graph)
- **openfang-runtime**: Agent execution engine, tool runner, loop guard; optional Cargo feature **`ainl-runtime-engine`** routes a turn through **`ainl-runtime`** (`AinlRuntime::run_turn`) instead of the LLM loop when enabled per manifest / env — see **`docs/ainl-runtime-integration.md`**. Nested **`run_turn`** depth is enforced **inside** **`ainl-runtime`** (**`AinlRuntimeError::DelegationDepthExceeded`**; **`TurnInput::depth`** is metadata only — **`docs/ainl-runtime.md`**).
- **openfang-kernel**: Agent lifecycle, orchestration, kernel handle trait
- **openfang-api**: HTTP API server (Axum) for dashboard and programmatic access
- **openfang-cli**: Command-line interface and interactive TUI

### Key Abstractions

- **Memory trait**: Unified interface for structured KV, semantic memories, knowledge graph
- **KernelHandle trait**: Agent-to-kernel communication without circular dependencies
- **AgentManifest**: TOML-based agent configuration with capabilities, tools, schedules
- **OrchestrationContext**: Delegation chains, trace IDs, budget tracking

### Schema

SQLite database schema (`~/.armaraos/memory.db`):
- `agents`: Agent manifests and state
- `sessions`: Chat sessions with message history
- `memories`: Semantic and episodic memories
- `entities` + `relations`: Knowledge graph (triples)
- `kv_store`: Structured key-value storage per agent
- `task_queue`: Asynchronous task distribution
- `usage_events`: LLM token/cost metering
- `audit_entries`: Merkle-chained audit log

---

## Layer 2: ArmaraOS (Enhancements)

**Project**: ArmaraOS - Agent Operating System  
**License**: Apache-2.0 OR MIT  
**Maintainer**: Steven Hooley

### Enhancements Over OpenFang

1. **Orchestration Tracing**
   - `OrchestrationTraceEvent` types for multi-agent debugging
   - Trace collection in `openfang-memory` with TTL
   - API endpoints: `GET /api/orchestration/traces`, `/traces/:id`, `/traces/:id/tree`, `/traces/:id/cost`
   - Dashboard visualization of delegation chains and cost rollups

2. **Ultra Cost-Efficient Mode**
   - Prompt compression pipeline (remove stopwords, deduplicate context, semantic chunking)
   - Three modes: Off, Balanced, Aggressive
   - API: `POST /api/config/set` with `efficient_mode`
   - Dashboard: Settings → Budget card, chat header eco pill

3. **Dashboard Enhancements**
   - Notification center (bell) with approval pending, budget alerts, kernel events
   - Command palette (Cmd/Ctrl+K) for global search
   - Pinned agents with UI-prefs persistence
   - Setup wizard for first-run onboarding
   - Home folder browser with preview/download

4. **Scheduled AINL**
   - Schedule syntax: `@daily run research.ainl`
   - Kernel injects `AINL_ALLOW_IR_DECLARED_ADAPTERS=1` for scheduled runs
   - Integration with cron-like scheduling

5. **Agent Pools**
   - `[[agent_pools]]` in config.toml for auto-scaling worker agents
   - `agent_pool_spawn` tool respects `max_instances`
   - Pool workers inherit base manifest

6. **Approval System**
   - Per-agent `require_approval_for` (tool patterns, cost thresholds)
   - API: `GET /api/approvals`, `POST /api/approvals/{id}/decision`
   - Dashboard: persistent approval queue in notification center

7. **Audit Chain**
   - Merkle tree audit log (SHA-256 chaining)
   - `GET /api/audit/verify` for tamper detection
   - Export: `GET /api/audit/export?format=json`

### Architecture Decisions

- **Kernel as Singleton**: Single `Kernel` instance per daemon, no multi-tenancy
- **SQLite + WAL**: Write-ahead logging for concurrent reads, busy timeouts for writes
- **Axum + Alpine.js**: Minimal SPA dashboard, no React/Vue dependencies
- **r2d2 Pooling**: Connection pooling for openfang-memory SQLite access
- **Event-Driven**: SSE streams (`/api/events/stream`, `/api/logs/stream`) for real-time updates

---

## Layer 3: AINL Graph-Memory Substrate

**Date**: April 12, 2026  
**Integration**: `openfang-runtime` agent loop + `tool_runner` (delegation, A2A, tools, persona)

**Execution engine (Rust):** `openfang-runtime` is the daemon's **current** Rust execution engine—agent loop, tool runner, loop guard, and graph-memory wiring all live here. The separately published **`ainl-runtime`** crate (also in this workspace) is **not** wired into the ArmaraOS daemon today; treating **`openfang-runtime` ↔ `ainl-runtime`** convergence as a single execution stack is a **roadmap** item, not current architecture. **`ainl-runtime`** enforces nested **`run_turn`** / **`run_turn_async`** depth internally (**`max_delegation_depth`**, **`DelegationDepthExceeded`**). Its optional **`async`** path (`run_turn_async`) uses **`tokio::task::spawn_blocking`** for SQLite while keeping graph memory under **`Arc<std::sync::Mutex<_>>`** (not **`tokio::sync::Mutex`**) so embedders can construct the runtime and take short store borrows from any thread without Tokio async-mutex **`blocking_lock`** pitfalls—see **`docs/ainl-runtime.md`** and **`crates/ainl-runtime/README.md`**.

### Vision

**Graph-as-memory paradigm**: The execution graph IS the memory, not a separate retrieval layer. Every agent turn, tool call, and delegation becomes a typed graph node. No episodic/semantic/procedural silos—unified graph traversal for context retrieval.

### Implementation

- **ainl-memory crate**: Standalone graph-memory substrate (zero ArmaraOS dependencies)
  - `src/node.rs`: AinlNodeType enum (Episode, Semantic, Procedural, Persona)
  - `src/store.rs`: GraphStore trait + SQLite implementation
  - `src/query.rs`: Graph traversal (walk_from, recall_recent, find_patterns)
  - `src/lib.rs`: GraphMemory API (write_episode, write_fact, store_pattern, recall_recent)

- **Schema**: Dedicated SQLite file per agent (`ainl_graph_*` tables inside `ainl_memory.db`), **not** inside `data/openfang.db`.

- **Two parallel graph projections (do not conflate them)**
  - **Primary (ArmaraOS agent path):** Rust **`GraphMemoryWriter`** over **`ainl-memory`** SQLite at `~/.armaraos/agents/<agent_id>/ainl_memory.db`. This is what the dashboard chat loop reads and writes for episodes, facts, delegations, and persona lines in the system prompt.
  - **AINL Python `ainl_graph_memory` adapter:** Persists to a **separate** JSON file (default `~/.armaraos/ainl_graph_memory.json`, override `AINL_GRAPH_MEMORY_PATH`). Scheduled **`ainl run`** may additionally use **`bundle.ainlbundle`** via **`AINL_BUNDLE_PATH`**. These files are **not** the same store as `ainl_memory.db`; nothing automatically merges them. Use each stack for its intended runner (Rust daemon vs. Python `ainl`).

- **Integration (`openfang-runtime`)**
  - `graph_memory_writer.rs` — `GraphMemoryWriter` (`Arc<Mutex<GraphMemory>>`); open is non-fatal; `run_persona_evolution_pass`, export path for Python refresh.
  - `agent_loop.rs` — `record_turn` on EndTurn (with optional semantic tags), fact/pattern extraction via **`ainl_graph_extractor_bridge`** when **`AINL_EXTRACTOR_ENABLED=1`** else **`graph_extractor.rs`**; spawned post-turn **`run_persona_evolution_pass`** + optional **`persona_evolution::PersonaEvolutionHook`** when **`AINL_PERSONA_EVOLUTION=1`**; persona lines merged into system prompt each LLM call.
  - `ainl_semantic_tagger_bridge.rs` — optional episode/fact tags when **`ainl-tagger`** feature + **`AINL_TAGGER_ENABLED=1`**.
  - `tool_runner.rs` — `tool_agent_delegate`: after successful send, `record_turn` with optional serialized `OrchestrationTraceEvent` JSON; `tool_a2a_send`: `record_delegation` after `A2aClient::send_task` (stays in `tool_runner` so `caller_agent_id` exists).
  - **`graph_extractor.rs` (local heuristic):** Default post-turn **regex / structural** extraction when **`AINL_EXTRACTOR_ENABLED`** is unset or falsey. The published **`ainl-graph-extractor`** crate is linked behind the **`ainl-extractor`** Cargo feature and selected at runtime by **`AINL_EXTRACTOR_ENABLED=1`** (`ainl_graph_extractor_bridge.rs`).

- **`ainl-runtime` crate (workspace, optional host):** Turn orchestration over the same **`ainl-memory`** SQLite (`run_turn`, optional **`run_turn_async`** + feature **`async`**). Internal **delegation depth** cap (**`AinlRuntimeError::DelegationDepthExceeded`**); **`TurnInput::depth`** is metadata only. Hub doc: **`docs/ainl-runtime.md`**. GraphPatch / patch adapters: **`docs/ainl-runtime-graph-patch.md`**.

Operator reference: **`docs/graph-memory.md`**, **`docs/graph-memory-sync.md`** (Python inbox), **`docs/persona-evolution.md`**, **`crates/openfang-runtime/README.md`**, **`docs/ainl-runtime.md`**.

### Node Types

1. **Episode**: What happened during an agent turn
   - `turn_id`, `timestamp`, `tool_calls`, `delegation_to`
   - Optional `trace_event` (OrchestrationTraceEvent as JSON)
   - Optional `tags` (string list): e.g. deterministic **`ainl-semantic-tagger`** tool labels when **`AINL_TAGGER_ENABLED=1`** (see **`docs/graph-memory.md`**)

2. **Semantic**: Facts learned with confidence scores
   - `fact`, `confidence` (0.0-1.0), `source_turn_id`
   - Optional `tags` (string list): orchestration correlation strings plus optional tagger output when **`AINL_TAGGER_ENABLED=1`**

3. **Procedural**: Reusable compiled workflow patterns
   - `pattern_name`, `compiled_graph` (binary format)

4. **Persona**: Agent traits learned over time
   - `trait_name`, `strength` (0.0-1.0), `learned_from` (turn IDs)

### Design constraints

- **Standalone crate**: `ainl-memory` stays free of kernel imports; publishable to crates.io.
- **Parallel substrate**: `openfang-memory` / `Memory` trait unchanged; graph DB is a separate file per agent.
- **Follow-ups**: tighter `source_turn_id` links for semantic nodes; optional richer prompt retrieval from `recall_recent`.

### Query Capabilities

- `query_episodes_since(timestamp, limit)`: Recent episodes by time
- `find_by_type(type_name)`: All nodes of a given type
- `walk_edges(from_id, label)`: Graph traversal via labeled edges
- `find_high_confidence_facts(min_confidence)`: Semantic facts above threshold
- `find_patterns(name_prefix)`: Procedural patterns by name

### Database Location

- **ArmaraOS agent loop (`GraphMemoryWriter`, primary):** `~/.armaraos/agents/<agent_id>/ainl_memory.db` (per-agent SQLite; schema created on first open).
- **AINL Python `ainl_graph_memory` (parallel JSON store):** Default `~/.armaraos/ainl_graph_memory.json` (override `AINL_GRAPH_MEMORY_PATH`); scheduled **`ainl run`** may also read/write **`~/.armaraos/agents/<agent_id>/bundle.ainlbundle`** via **`AINL_BUNDLE_PATH`** — see **`docs/scheduled-ainl.md`**. Same *concept* (graph memory), **different persistence** than `ainl_memory.db`.

---

## Interoperability

### OpenFang ↔ ArmaraOS

ArmaraOS maintains API compatibility with OpenFang:
- `POST /api/agents/{id}/message` - same payload format
- `GET /api/agents` - same response structure
- Tool definitions (`builtin_tool_definitions()`) - superset of OpenFang

### ArmaraOS ↔ AINL

- **Delegate path**: `tool_agent_delegate` embeds a serialized `OrchestrationTraceEvent` into the graph episode when JSON serialization succeeds (correlate with dashboard / API traces via `trace_id`).
- **Trace ring**: hot orchestration events still live under **`openfang-memory`** / kernel HTTP; the graph is an additional durable projection, not a replacement store.
- **Parallel graph stores**: The Rust daemon persists AINL-style graph nodes to per-agent **`ainl_memory.db`** via **`GraphMemoryWriter`** (`ainl-memory`). The Python **`ainl_graph_memory`** adapter (JSON default path, optional **`AINL_BUNDLE_PATH`** bundle) is a **separate** persistence layer for **`ainl run`** workflows—not the same file as `ainl_memory.db`. See Layer 3.

### Memory layers

```
┌─────────────────────────────────────────┐
│ AINL graph memory — Rust primary        │
│ ~/.armaraos/agents/<id>/ainl_memory.db  │
│ GraphMemoryWriter + ainl-memory crate   │
└─────────────────────────────────────────┘
              ↕ (correlate by trace_id, agent id, time)
┌─────────────────────────────────────────┐
│ OpenFang memory (Layer 1)               │
│ ~/.armaraos/data/openfang.db            │
│ Sessions, semantic recall, traces, …   │
└─────────────────────────────────────────┘

┌─────────────────────────────────────────┐
│ AINL graph memory — Python adapter      │
│ ainl_graph_memory.json / bundle path  │
│ Parallel store; not merged with .db   │
└─────────────────────────────────────────┘
```

**Different SQLite files.** `ainl-memory` tables live only inside each per-agent `ainl_memory.db`. The Python JSON / bundle files are a **separate** projection used by the Python runtime, not a second view of the same file.

---

## Build System

- **Workspace**: Cargo workspace with 20 member crates
- **Shared dependencies**: `workspace.dependencies` in root `Cargo.toml`
- **Release profile**: LTO enabled, stripped binaries, opt-level 3
- **CI**: GitHub Actions (check, test, clippy, fmt) on push/PR

### Key Crates

| Crate | Layer | Purpose | AINL integration status |
|-------|-------|---------|-------------------------|
| openfang-types | 1 | Core type definitions | — |
| openfang-memory | 1 | SQLite memory substrate | — |
| openfang-runtime | 1+2 | **Current Rust execution engine** for the daemon: agent loop, tools, `GraphMemoryWriter`, Python inbox drain, `ainl_graph_extractor_bridge` + legacy `graph_extractor`, optional `ainl_semantic_tagger_bridge` / `PersonaEvolutionHook` | **`ainl-memory`:** integrated (`GraphMemoryWriter`) |
| openfang-kernel | 2 | Agent lifecycle, orchestration tracing | — |
| openfang-api | 2 | HTTP API with dashboard enhancements | — |
| ainl-memory | 3 | Graph-memory substrate (standalone); SQLite `GraphMemory` | **Integrated** — primary graph store for `GraphMemoryWriter` |
| ainl-persona | 3 | Persona model + evolution APIs (`EvolutionEngine`, etc.) | **Integrated** — feature `ainl-persona-evolution` (**default ON**); post-turn evolution + prompt merge in `agent_loop.rs` / `graph_memory_writer.rs` |
| ainl-graph-extractor | 3 | Structured graph extraction (published / workspace crate) | **Integrated** — feature `ainl-extractor` (**default ON**); `ainl_graph_extractor_bridge.rs` + `graph_extractor.rs` fallback; runtime gate `AINL_EXTRACTOR_ENABLED` |
| ainl-semantic-tagger | 3 | Semantic tagging for extraction pipeline | **Integrated** — feature `ainl-tagger` (**default ON**); `ainl_semantic_tagger_bridge.rs`; runtime gate `AINL_TAGGER_ENABLED` |
| ainl-runtime | 3 | Standalone AINL execution stack (workspace + **crates.io**); procedural **`PatchAdapter`** registry + **`GraphPatchAdapter`** fallback JSON (`label`, `patch_version`, `frame_keys`); topic-ranked **`MemoryContext::relevant_semantic`**; nested **`run_turn`** depth enforced internally (**`DelegationDepthExceeded`**); **`TurnOutcome`** / **`TurnWarning`** / **`TurnPhase`**; docs in **`docs/ainl-runtime-graph-patch.md`**, **`docs/ainl-runtime-integration.md`**, **`crates/ainl-runtime/README.md`** | **Optional shim** — feature `ainl-runtime-engine` (**default OFF**); `ainl_runtime_bridge.rs`; approval gate pseudo-tool `ainl_runtime_engine`; step-based `cost_estimate` + kernel metering rollup |

---

## Future Work

### AINL Memory Roadmap

1. **Optional kernel handle** (Week 2+)
   - Optionally expose a `GraphMemory` / writer hook on `KernelHandle` for tools that lack `caller_agent_id`
   - Today: per-loop `GraphMemoryWriter` in `agent_loop` + ad-hoc `GraphMemoryWriter::open` in `tool_runner` for delegate/A2A

2. **Retrieval at Agent Loop Start** (Week 3)
   - Query recent episodes before LLM call
   - Inject graph context into system prompt
   - A/B test: graph retrieval vs. traditional semantic search

3. **Semantic Fact Extraction** (Week 4)
   - Post-turn hook: parse assistant response for facts
   - Confidence scoring via LLM self-eval
   - Write Semantic nodes with links to source Episode

4. **Procedural Pattern Learning** (Month 2)
   - Detect repeated tool sequences
   - Compile to Procedural nodes
   - One-click "apply pattern" in dashboard

5. **Persona traits: evolution and learning** (Month 2+)
   - **Already live:** Persona **reading** — `agent_loop.rs` recalls **`PersonaNode`** rows from **`ainl-memory`** (via `GraphMemoryWriter`) and injects trait lines into the system prompt.
   - **Roadmap:** Persona **evolution** — wire **`ainl-persona`**'s **`EvolutionEngine`** (or equivalent) so traits are updated from observed behavior, not only read from stored nodes; aggregate preferences across sessions and persist updated `PersonaNode` / edge data.

6. **Publishing to crates.io** (ongoing)
   - Continue releasing **`ainl-memory`**, **`ainl-persona`**, **`ainl-graph-extractor`**, **`ainl-semantic-tagger`**, **`ainl-runtime`** as the workspace matures.
   - Playbook: **`docs/ainl-crates-publish.md`** (dependency order, dry-run vs live index).
   - Documentation: rustdoc + examples

---

## Lineage Summary

```
OpenFang (upstream)
    ↓ Fork + enhancements
ArmaraOS (orchestration, tracing, dashboard)
    ↓ Graph-memory integration
AINL Memory Substrate (execution IS memory)
```

**Date of AINL Integration**: April 12, 2026  

---

## References

- [AINL graph memory (ArmaraOS)](docs/graph-memory.md)
- [AINL Specification](https://github.com/sbhooley/ainativelang)
- [OpenFang Documentation](https://github.com/sbhooley/armaraos/tree/main/docs)
- [ArmaraOS Dashboard Testing](docs/dashboard-testing.md)
- [Agent Orchestration Design](docs/agent-orchestration-design.md)
