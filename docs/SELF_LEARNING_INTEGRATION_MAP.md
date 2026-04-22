# Self-Learning Integration Map (AINL Graph Memory ⇄ ArmaraOS)

> **Source plugin:** `~/.claude/plugins/ainl-graph-memory` (Python MCP, ≈ v0.2)
> **Target:** Rust crates under `crates/ainl-*` and the embedding host in `openfang-runtime`.
> **Goal:** Bring Hermes-style self-learning (trajectory capture, failure learning, adaptive compression, persona evolution, closed-loop validation) into ArmaraOS as first-class crates so every embedded host (`openfang-runtime`, scheduled `ainl run`, future hosts) gets it for free.

The plugin explicitly attributes its design to ArmaraOS (`ainl-memory`, `ainl-persona`, `ainl-runtime`, `ainl-compression`, `ainl-graph-extractor`, `ainl-semantic-tagger`) — so this document is a **back-port plan** rather than a green-field design. We already own the Rust spine; what is missing is a small set of **node kinds, store tables, and orchestration modules**.

Cross-reference docs:
- **[graph-memory.md](graph-memory.md)** — runtime integration + on-disk layout
- **[persona-evolution.md](persona-evolution.md)** — `PersonaEvolutionHook`, `AINL_PERSONA_EVOLUTION`
- **[ainl-runtime.md](ainl-runtime.md)** + `crates/ainl-runtime/README.md` — turn pipeline
- **[GRAPH_MEMORY_EXPLAINABILITY.md](GRAPH_MEMORY_EXPLAINABILITY.md)** — provenance fields
- **[ainl-runtime-graph-patch.md](ainl-runtime-graph-patch.md)** — `PatchAdapter`
- **[learning-frame-v1.md](learning-frame-v1.md)** — vitals + persona signal contract

> **Architecture invariant:** every new crate proposed below must compile with **only `ainl-*` dependencies** (no `openfang-*`, no `armaraos-*`). Integration with the ArmaraOS host happens through **trait-based telemetry sinks** (mirroring the existing `ainl-compression::CompressionTelemetrySink` pattern) and through the deps-free **`ainl-contracts`** vocabulary. This is what lets the same crates be reused by `ainl-inference-server`, the standalone `ainativelang` Python tooling, the AINL MCP server, and any future host without dragging openfang in.

---

## 1. Capability Gap Audit

| Plugin capability (Python) | ArmaraOS today (Rust) | Gap | Owner crate (proposed) |
|---|---|---|---|
| **Typed graph memory** (`ainl_graph_nodes`, `ainl_graph_edges`, FTS5) | `ainl-memory::GraphMemory` (SQLite, WAL) + `Failure` kind + `ainl_failures_fts` (FTS5) + typed edges; 6+ kinds including `Failure` and `Trajectory`. | **FTS5:** on failures (`search_failures_for_agent`). Generic `ainl_nodes_fts` over all embedding text — **not** shipped; see [graph-memory](graph-memory.md) / §2 below for optional future table. | `ainl-memory` (extend) |
| **Project isolation** (`project_id` column) | Per-agent DB at `~/.armaraos/agents/<id>/ainl_memory.db` | Already isolated **per-agent**. We do **not** namespace by *project* (cwd/repo). For multi-project agents this leaks. | `ainl-memory` (add column) + `openfang-runtime` (resolver) |
| **Context-aware retrieval** (`AINLContextCompiler`, 500-tok budget, priority 1-3) | Prompt-time blocks (`RecentAttempts`, `KnownFacts`, `KnownConflicts`, `SuggestedProcedure`) inside `openfang-runtime` | Blocks exist but lack an explicit **token-budget compiler** with priority queue and per-block kill switches surfaced as a reusable type. | `ainl-context-compiler` (new) or module in `ainl-runtime` |
| **Graceful degradation** (hooks never crash CC) | `GraphMemoryWriter::open` is non-fatal; persona pass returns `ExtractionReport`, not `Result<(), String>` | ✅ parity. Apply same pattern to all new modules. | n/a |
| **CLI inspection** (`memory_cli.py`, `compression_cli.py`, `trajectory_cli.py`) | Dashboard `#graph-memory`, `GET /api/graph-memory*`, `openfang orchestration …` | **Near parity:** `openfang trajectory …`, `openfang memory …`, **`openfang compression`** (`test|score|detect`, `profiles|adaptive|cache` subcommands on `ainl-compression`; no daemon). Host-side persistence for per-project profile overrides remains future work. | `openfang-cli` |
| **Zero-LLM persona evolution** (5 axes, EMA α=0.3) | `ainl-persona` (5 axes: `Instrumentality`, `Verbosity`, `Persistence`, `Systematicity`, `Curiosity`, `EMA_ALPHA = 0.2`) + `PersonaEvolutionHook` | ✅ feature parity. Note α delta: armaraos = **0.2**, plugin = **0.3**. Keep armaraos value (more conservative; less overfitting on short sessions). | n/a (alignment only) |
| **Trajectory capture** (`ExecutionTrajectory` w/ `ainl_source_hash`, frame vars, adapters, per-step latency, outcome, fitness delta) | `ainl-memory` persists `Trajectory` nodes + `ainl_trajectories` rows (`persist_trajectory_for_episode`). `AINL_SOURCE_HASH` / `session_id` / `project_id` flow from env + trace JSON. **`ainl-runtime::run_turn` / `run_turn_async`** record **per procedural patch** (adapter name, `execute_patch` duration, success, output preview) **then** normalized **tool** steps. **`openfang-runtime`** still owns per-host tool-call traces in `tool_runner` / EndTurn. | **Partial gap:** frame vars + fitness delta on the trajectory body, richer host-only tool telemetry, and dashboard/SSE surfacing (Phase 8) remain. | `ainl-trajectory` (replay/JSONL) + `ainl-memory` (store) + hosts |
| **Pattern promotion** (min observations + EMA fitness ≥ 0.7; optional env `AINL_PATTERN_PROMOTION_*`) | `ainl_memory::pattern_promotion`, `GraphMemory::find_procedural_by_tool_sequence`, `GraphMemoryWriter::record_pattern` (merge + EMA), `ProceduralNode::{pattern_observation_count, prompt_eligible}`. | **Shipped (host):** prompt-time `SuggestedProcedure` in `graph_memory_context` only lists `prompt_eligible` procedurals; candidates stay in the graph until the gate trips. | `ainl-memory` + `openfang-runtime` |
| **Failure learning** (`failure_resolutions` table, FTS5 lookup, prevention prompt) | **`ainl-memory`:** `Failure` nodes, **`ainl_failures_fts`**, `search_failures_for_agent` — same failure surface as the plugin’s FTS idea, scoped per agent DB. **Host:** `LearningRecorder`, **`## FailureRecall`**, **`/api/graph-memory/failures/*`**, **`FailureLearned`** + graph writes. | Ecosystem: optional portable **`ainl-failure-learning`**; optional **`failure_resolutions`** edge table. | `ainl-memory` + `openfang-runtime` (shipped) |
| **Smart suggestions** (context-aware recommendations from history) | Surface area exists via prompt-time blocks; lacks explicit "suggested next action" channel. | Plumbing-only. | `ainl-context-compiler` |
| **Closed-loop validation** (proposal → strict validate → adopt; tracks accept rate) | `ainl-improvement-proposals` (SQLite ledger) + `openfang-runtime::{improvement_proposals_host, improvement_proposals_validators}`; HTTP + dashboard on Graph Memory. | **Shipped** (opt-in: `AINL_IMPROVEMENT_PROPOSALS_ENABLED`). | `ainl-improvement-proposals` + `openfang-runtime` + `openfang-api` |
| **Adaptive compression** (per-project mode learning, semantic scoring, cache TTL coordination, adaptive eco by content type) | `ainl-compression`: `EfficientMode`, `compress()`, `CompressionMetrics`, `estimate_semantic_preservation_score`, plus `profiles` / `adaptive` / `cache` (built-ins + heuristics). **Host:** kernel `[adaptive_eco]` + `resolve_adaptive_eco_turn` + **`AINL_ADAPTIVE_COMPRESSION`** (merges content recommendation, profile hint, cache stretch into `adaptive_eco` metadata). **Dashboard / config:** `efficient_mode = "adaptive"` (chat **eco ada** pill, Settings → Budget) forces the adaptive path per request even if `[adaptive_eco].enabled` is off. | **Shipped (core).** On-disk EMA per-project profile JSON + richer cache policies remain optional. | `ainl-compression` + `openfang-runtime` + `openfang-kernel` |

**Summary:** 3 net-new crates, 2 in-place extensions, 0 conceptual rewrites — **plus** a Phase 0 `ainl-contracts` uplift (lifts `CognitiveVitals` to the deps-free contract crate, adds shared `TrajectoryStep` / `FailureKind` / `ProposalEnvelope` vocabulary). See §15 for the cross-crate integration plan that keeps every new crate usable outside armaraos via trait-based telemetry sinks.

---

## 2. Node-kind extension to `ainl-memory`

Add two new variants to `AinlNodeKind` in `crates/ainl-memory/src/node.rs`:

```rust
pub enum AinlNodeKind {
    Episode,
    Semantic,
    Procedural,
    Persona,
    RuntimeState,
    Failure,     // NEW — error + resolution ledger (FTS5-indexed)
    Trajectory,  // NEW — per-execution step trace, keyed to Episode
}
```

Schema migration (bump `PRAGMA user_version` from current → +1; gate behind a `ainl_memory::migrate::up_to_v…` step that adds:

```sql
ALTER TABLE ainl_graph_nodes ADD COLUMN project_id TEXT;  -- nullable; null = agent-wide
CREATE INDEX IF NOT EXISTS idx_nodes_project_kind
    ON ainl_graph_nodes(project_id, node_type, created_at DESC);

CREATE VIRTUAL TABLE IF NOT EXISTS ainl_nodes_fts USING fts5(
    node_id UNINDEXED,
    embedding_text,
    tokenize='porter unicode61'
);
```

Backwards compat: `project_id IS NULL` → unchanged behaviour for existing single-project agents.

Edge-type whitelist gains: `RESOLVES`, `FIXED_BY`, `PATTERN_FOR`, `LEARNED_FROM` (subset of plugin's enum) on `ainl_graph_edges` — these are needed for failure→resolution and trajectory→pattern relations.

---

## 3. New crate: `ainl-trajectory`

**Purpose:** Capture every AINL/agent execution as a replayable JSONL-friendly trace, keyed to its `Episode` node. This is the **substrate for every other learner** — pattern recognition, failure learning, fitness deltas, and adaptive compression all read from trajectories.

```text
crates/ainl-trajectory/
├── Cargo.toml
└── src/
    ├── lib.rs              # public API
    ├── step.rs             # TrajectoryStep { adapter, op, inputs, outputs, duration_ms, success, error }
    ├── trajectory.rs       # ExecutionTrajectory { id, episode_id, ainl_source_hash, frame_vars, adapters, outcome, steps, tags, fitness_delta }
    ├── store.rs            # TrajectoryStore (SQLite, same DB as ainl_memory.db, separate `trajectories` table)
    └── replay.rs           # serialize → JSONL for debug / replay (mirrors `cli/trajectory_cli.py`)
```

**Wiring points in `openfang-runtime`:**

| Site | What it captures |
|---|---|
| `tool_runner.rs::execute_tool` (per-call) | append a `TrajectoryStep` to an open `TrajectoryBuilder` keyed by `(session_id, turn_id)` |
| `agent_loop.rs` EndTurn block | finalize via `TrajectoryStore::record(builder, episode_id)`, hash AINL source if present (`AINL_BUNDLE_PATH` / scheduled `ainl run`) |
| `ainl-runtime::run_turn` / `run_turn_async` | When `AINL_TRAJECTORY_ENABLED` is on (default), after `record_turn_episode` the runtime builds `TrajectoryStep`s from **`PatchDispatchResult`** (adapter wall time, label, success) **then** `normalize_tools_for_episode` tool names, and calls `ainl_memory::persist_trajectory_for_episode`. See [ainl-runtime-integration.md](ainl-runtime-integration.md) / `crates/ainl-runtime/src/runtime.rs`. |

**Why a sibling table, not a JSON column on `Episode`:** trajectories can grow large (10–100 steps × bytes of I/O). Keeping them in a `trajectories` table (FK to episode UUID, with `ON DELETE CASCADE`) preserves the cheap-`Episode`-recall pattern that prompt-time blocks rely on.

---

## 4. New crate: `ainl-failure-learning`

**Purpose:** Convert errors (validator misses, adapter typos, tool failures) into recall-ranked `Failure` graph nodes, with FTS5-backed lookup and a "prevention" prompt block.

```text
crates/ainl-failure-learning/
└── src/
    ├── lib.rs
    ├── node.rs             # FailureNode { error_type, error_message, ainl_source, context, resolution, resolution_diff, prevented_count }
    ├── ingest.rs           # from_validator_error / from_tool_failure / from_loop_guard
    ├── search.rs           # FTS5 query → Vec<FailureMatch> ranked by recurrence × recency × confidence
    └── suggest.rs          # SuggestionBlock for context-compiler injection ("I've seen this error N times — fix: …")
```

**Hooks into existing surface:**

| Existing site | New behaviour |
|---|---|
| `crates/ainl-runtime/src/engine.rs` (validator) | on validation error → `FailureLearningStore::record(error, ainl_source, context)` |
| `crates/openfang-runtime/src/loop_guard.rs` | when `loop_guard` fires repeated identical failure → check `FailureLearningStore::similar` and surface resolution into next prompt via `BTW`-style injection |
| `crates/openfang-runtime/src/tool_runner.rs::execute_tool` (Err arm) | record `FailureNode` with `node_type = 'failure'` and `RESOLVES` edge once a successful retry follows |

**FTS5 query example:** `SELECT node_id FROM ainl_nodes_fts WHERE embedding_text MATCH ? ORDER BY bm25(ainl_nodes_fts) LIMIT 5;`

Performance target (matching plugin): **<30 ms FTS5 lookup**, well within the per-turn budget.

---

## 5. New crate: `ainl-improvement-proposals`

**Purpose:** Closed-loop validation gate. The runtime can *propose* an improvement to an AINL workflow (e.g. "promote this 3-step pattern into a named procedure"); the proposal is hashed, validated through the normal `ainl-runtime` strict checker, and only adopted if validation passes. Tracks accept/reject ratio per proposal type to prevent regression.

```text
crates/ainl-improvement-proposals/
└── src/
    ├── lib.rs
    ├── proposal.rs         # ImprovementProposal { original_hash, proposed_hash, kind, rationale, validated, accepted }
    ├── store.rs            # SQLite ledger
    ├── validate.rs         # thin wrapper over ainl-runtime strict validate
    └── policy.rs           # accept rules (e.g. "auto-adopt if fitness > 0.7 AND validation_passed AND last 5 of same kind accepted")
```

This is the Hermes "closed loop" piece. It belongs in its own crate because it depends on **both** `ainl-runtime` (for validation) and `ainl-graph-extractor` (for pattern detection) — putting it in either creates a cycle.

---

## 6. Extend `ainl-compression`

`ainl-compression` already has the **hard parts**: `EfficientMode`, `compress()` with code-fence preservation, hard/soft preserve lists, `CompressionMetrics` telemetry, `estimate_semantic_preservation_score`. We add three modules **without breaking the existing public API**:

```text
crates/ainl-compression/src/
├── lib.rs                  # existing
├── profiles.rs             # NEW — ProjectProfile { project_id, optimal_mode, avg_savings, quality_score, correction_count }
├── adaptive.rs             # NEW — content classifier → ModeRecommendation (mirrors `adaptive_eco.py`)
└── cache.rs                # NEW — CacheCoordinator (5min TTL hysteresis; mirrors `cache_awareness.py`)
```

| Module | Plugin reference | Behaviour |
|---|---|---|
| `profiles.rs` | `mcp_server/project_profiles.py` + `compression_profiles.py` | Persists per-project preferred mode under `~/.armaraos/agents/<id>/compression_profiles.json` (or table inside `ainl_memory.db` for atomicity). EMA-tunes from `CompressionMetrics` + correction signals. |
| `adaptive.rs` | `mcp_server/adaptive_eco.py` | Classifies content (code ratio, technical density, question vs command, URLs, file paths) → recommends `Off`/`Balanced`/`Aggressive` with confidence. **Pure function**, no I/O. |
| `cache.rs` | `mcp_server/cache_awareness.py` | Avoids switching modes inside the provider's prompt-cache TTL window (Anthropic/OpenAI ≈ 5 min). Returns `CacheDecision { use_mode, cache_preserved, recommended_mode }`. |

**Wiring (Phase 5):** `agent_loop` applies `compress_with_metrics` using `efficient_mode` on the manifest (injected by the kernel from global / orchestration / adaptive enforce). **`[adaptive_eco].enabled`** makes the kernel call `openfang_runtime::eco_mode_resolver::resolve_adaptive_eco_turn`. Set **`AINL_ADAPTIVE_COMPRESSION=1`** (truthy) so the resolver merges **`ainl_compression::recommend_mode_for_content`** into shadow `recommended_mode`, records **`suggest_profile_id_for_project`** when `metadata["project_id"]` is set, and emits **`effective_ttl_with_hysteresis`** for operator visibility. Per-agent JSON **EMA profile files** on disk remain optional future work (see Phase 5 table).

---

## 7. New crate (optional but recommended): `ainl-context-compiler`

Hoist the prompt-time block builders out of `openfang-runtime` into a reusable crate so `ainl-runtime` and any other host can use the same compiler.

```text
crates/ainl-context-compiler/
└── src/
    ├── lib.rs
    ├── block.rs            # ContextBlock { name, content, priority: 1..3, token_estimate }
    ├── budget.rs           # greedy fill within `max_tokens` (default 500), priority-first, conservative truncation
    └── sources/
        ├── recent_attempts.rs       # already in openfang-runtime — move here
        ├── known_facts.rs           # ditto
        ├── known_conflicts.rs       # ditto
        ├── suggested_procedure.rs   # ditto
        ├── active_persona.rs        # NEW — formats `recall_persona` output
        ├── failure_warnings.rs      # NEW — from ainl-failure-learning
        └── trajectory_recap.rs      # NEW — from ainl-trajectory
```

The control surface (`GET/PUT /api/graph-memory/controls`) keeps its per-block kill switches (`include_episodic_hints`, `include_semantic_facts`, `include_conflicts`, `include_procedural_hints`) and gains four more (`include_active_persona`, `include_failure_warnings`, `include_trajectory_recap`, `include_suggested_patterns`).

---

## 8. Phased rollout

Each phase is **independently shippable** and gated by a Cargo feature so partial builds keep working. All phases must respect the standard verification trio from `CLAUDE.md`:

```bash
cargo build --workspace --lib
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

| Phase | Deliverable | Crates touched | Feature flag | Gate |
|---|---|---|---|---|
| **0 — Contracts uplift** *(prerequisite, see §15)* | ✅ **Done:** `ainl-contracts` hosts `vitals`, `TrajectoryStep` / `TrajectoryOutcome` / `FailureKind` / `ProposalEnvelope`, `telemetry::*` learner keys, `CONTRACT_SCHEMA_VERSION` / `LEARNER_SCHEMA_VERSION`; `openfang-types::vitals` re-exports canonical types. **Gate:** JSON round-trip tests in `ainl-contracts`; `TypeId` + JSON wire tests in `openfang-types` proving re-export parity. | `ainl-contracts`, `openfang-types` | n/a (always on) | `cargo test -p ainl-contracts --lib` + `cargo test -p openfang-types vitals_` |
| **1 — Trajectory foundation** | ✅ **Done:** same as prior row **plus** `ainl-trajectory` **`replay` JSONL**; **`openfang trajectory list|search|analyze|export`** (offline DB). **`ainl-runtime`** mirrors graph-memory persistence: after each episode it writes **patch-dispatch steps then tool steps** via `persist_trajectory_for_episode` (not coarse-tools-only). *Still future:* dashboard `#trajectories` + SSE (Phase 8). | `ainl-memory`, `ainl-trajectory`, `openfang-runtime`, `ainl-runtime`, `openfang-api`, `openfang-cli` | n/a | §11 curl + `cargo test -p ainl-trajectory --lib` + `cargo test -p ainl-runtime --all-features` |
| **2 — Failure learning** | **Shipped (host):** same persistence + API as prior row. **Host spine:** `openfang-runtime::graph_memory_learning::LearningRecorder` centralizes failure ingest (loop-guard, tool errors, hook/param precheck, **`ainl-runtime` graph validation** when `run_turn` fails with `graph validation` in the error text) with **message sanitization** + **metrics** (`graph_memory_learning_metrics` / `GET /api/status`). **In-prompt recall:** `graph_memory_context::build_prompt_memory_context` injects a **`## FailureRecall`** block from FTS (`search_failures_for_agent`), gated by the same learning policy (`learning.policy().failures`) + `MemoryContextPolicy::include_failure_recall` + `AINL_MEMORY_INCLUDE_FAILURE_RECALL` / `controls.json` **`include_failure_recall`**. **Operator API:** `GET /api/graph-memory/failures/recent` lists recent `failure` nodes; **`GET /api/graph-memory/failures/search`** remains FTS. **SSE:** `FailureLearned` + `GraphMemoryWrite` (`kind=failure`) already publish from the kernel on graph writes. **Deferred (ecosystem portability):** standalone **`ainl-failure-learning`** crate (trait sinks + non-OpenFang hosts) and a full **`ainl-context-compiler`** “suggestion” channel — not required for ArmaraOS desktop safety. | `ainl-memory`, `openfang-runtime`, `openfang-api`, `openfang-kernel` | `AINL_LEARNING` + subsystem envs | `cargo test -p openfang-runtime graph_memory_learning` + `cargo test -p ainl-memory` |
| **3 — Pattern promotion gate** | ✅ **Done:** `ainl_memory::pattern_promotion` (min observations + EMA + env tunables), `ProceduralNode::{pattern_observation_count, prompt_eligible}`, `GraphMemoryWriter::record_pattern` merge, vitals `gate=fail` skips persist, prompt blocks **`## SuggestedProcedure`** (promoted) + **`## SuggestedPatternCandidates`** (pre-promotion) + `controls` / `AINL_MEMORY_INCLUDE_SUGGESTED_PATTERN_CANDIDATES`. | `ainl-memory`, `openfang-runtime`, `openfang-api` (controls UI) | n/a | `cargo test` graph_memory_context + `ainl-memory` |
| **4 — Closed-loop validation** | ✅ **Shipped (end-to-end):** crate `ainl-improvement-proposals` (no `openfang` deps) + **`openfang-runtime`**: `improvement_proposals_host` + `improvement_proposals_validators` (`structural` / `strict` / `external` + `AINL_IMPROVEMENT_PROPOSALS_EXTERNAL_VALIDATE` `%p`). **HTTP** (`openfang-api` `graph_memory`): `GET/POST .../improvement-proposals` (list, submit, validate, adopt). **Dashboard** Graph Memory table. **Idempotency + ledger↔graph repair** + metrics. Details: **§15.7** and env `AINL_IMPROVEMENT_PROPOSALS_ENABLED`, `…DEFAULT_VALIDATE_MODE`. | `ainl-improvement-proposals`, `openfang-runtime`, `openfang-api` | opt-in via env (see §15.7) | `cargo test -p ainl-improvement-proposals` + `cargo test -p openfang-runtime` relevant modules |
| **5 — Adaptive compression** | ✅ **Crates:** `ainl-compression` ships `profiles`, `adaptive` (`recommend_mode_for_content`), `cache` (`effective_ttl_with_hysteresis`). ✅ **Host:** `[adaptive_eco]` in kernel + `openfang_runtime::eco_mode_resolver::resolve_adaptive_eco_turn` — when `AINL_ADAPTIVE_COMPRESSION=1` (and `[adaptive_eco].enabled`), merges `ainl_compression` content recommendations into `recommended_mode`, adds profile id hint from `metadata.project_id` / `suggest_profile_id_for_project`, and **cache stretched TTL** metadata for prompt-cache lines. `agent_loop` still applies `compress_with_metrics` with manifest `efficient_mode` (kernel-injected). Per-agent JSON profile persistence (full EMA) remains future. | `ainl-compression`, `openfang-runtime`, `openfang-kernel` | `AINL_ADAPTIVE_COMPRESSION` + `config.toml` `[adaptive_eco]` | `cargo test -p openfang-runtime eco_mode_resolver` + adaptive eco harness; cache hysteresis tests in `ainl-compression` |
| **6 — Context compiler hoist** *(pulled forward; runs ahead of Phases 3–5 — see Cursor plan `ainl_context_engine_d20f60d3` for sequencing rationale)* | Move blocks into `ainl-context-compiler` crate; add new sources; expose token budget knob. **Plan-extended scope:** widens from graph-memory blocks (~500 tok) to whole-prompt orchestration (System/User/RecentTurn/OlderTurn/ToolDefinitions/ToolResult); adds SOTA capability tiers (heuristic → LLM-driven anchored summarization → embedding rerank). Sources requiring unbuilt crates (`ainl-failure-learning`, `ainl-improvement-proposals`) stay behind off-by-default cargo features; light up automatically when those crates land. **M1 status — landed:** Tier 0 compiler runs in *measurement mode* on every turn in `openfang_runtime::agent_loop` (both `run_agent_loop` + `run_agent_loop_streaming`); whole-prompt token estimates flow to the kernel via the new `openfang_runtime::compose_telemetry` side-channel (`record_compose_turn` / `take_compose_turn`, mirroring the `eco_telemetry` pattern), and `openfang-kernel`'s compression-event recorder prefers them over the legacy user-message-only math when present. The LLM input is **not** mutated yet (M2 swaps the assembled prompt for `ComposedPrompt.segments`). | `ainl-context-compiler`, `openfang-runtime`, `ainl-runtime`, `openfang-kernel` | `ainl-context-compiler` (default-on); per-source features (`sources-bulk`, `sources-graph-memory`, `sources-failure-warnings`, `sources-suggested-patterns`, `sources-trajectory-recap`); `summarizer` (M2); `embedder` (M3) | byte-for-byte equivalence test vs current prompts before flipping default; whole-prompt savings reflected in `original_tokens_est` / `compressed_tokens_est` on `usage_events`. **M1 acceptance met** when `ainl-context-compiler::ContextCompiler::compose` is invoked once per turn from `openfang-runtime` and the kernel's `eco_compression_events` row uses the resulting `total_original_tokens` / `total_compressed_tokens` (verified by `cargo test -p openfang-runtime compose_telemetry::measure_and_record_records_nonzero_for_realistic_prompt`). |
| **7 — CLI surfaces** | **Mostly done:** trajectory + memory as prior rows; **`openfang compression`** wraps `ainl-compression` core + **`profiles` / `adaptive` / `cache`** CLI surfaces. Optional later: persist per-project profile overrides in `openfang-runtime` / env wiring. | `openfang-cli` | n/a | live integration tests per CLAUDE.md *Live Integration Testing* |
| **8 — Dashboard panels** | **`#trajectories`** — lists `GET /api/trajectories`, **`GET /api/graph-memory/failures/recent`**, and subscribes to SSE **`TrajectoryRecorded`**, **`FailureLearned`**, **`GraphMemoryWrite`** (`trajectory` / `failure`). **`#failures` as a separate nav tab** and **`#proposals`** + **`ProposalAdopted`** remain future polish. | `openfang-api`, `static/` | n/a | dashboard smoke (`scripts/verify-dashboard-smoke.sh`) |

Estimated effort: 2 engineer-weeks per phase; phases 1+2 are the critical path because every other learner reads trajectories and writes failures.

---

## 9. Sequencing risks & mitigations

| Risk | Mitigation |
|---|---|
| **Schema migration on existing user DBs** (`~/.armaraos/agents/<id>/ainl_memory.db`) | All `ALTER TABLE` ops are nullable-column adds; FTS5 virtual table is `IF NOT EXISTS`. Bump `user_version` and add `migrate::up_to_v…`. Keep `import_graph(allow_dangling_edges=true)` semantics. |
| **WAL coexistence** with `ainl-runtime` `RuntimeStateNode` writer | Already-tested pattern (see graph-memory.md *Optional `ainl-runtime`*). Trajectory writer follows same `Arc<Mutex<_>>` discipline. |
| **Project isolation regression** for single-project agents | `project_id` defaults to `None`; resolver in `openfang-runtime` lazily computes from `ARMARAOS_HOME` + cwd hash and is **opt-in** via `AINL_MEMORY_PROJECT_SCOPE=1`. |
| **Persona α drift** (plugin uses 0.3, we use 0.2) | Keep ArmaraOS at `EMA_ALPHA = 0.2` — empirically less reactive on 5–10 turn sessions. Document in `persona-evolution.md`. |
| **Trajectory bloat** | Background consolidation (already exists for semantic dedup) extended with `trajectories WHERE created_at < now - 30d` purge unless referenced by an adopted proposal. |
| **Hook crash safety** | Mirror plugin's golden rule (`hooks never break the host`): every new writer wraps in `let _ = … ;` at the top-level call site; errors logged at `warn!` only, like `ExtractionReport`. |
| **Provider cache thrash** during phase 5 | `CacheCoordinator` ships with a kill switch (`AINL_COMPRESSION_CACHE_AWARE=0`). |

---

## 10. Direct file map (plugin → armaraos)

| Plugin file (Python) | ArmaraOS target (Rust) | Notes |
|---|---|---|
| `mcp_server/node_types.py` | `crates/ainl-memory/src/node.rs` | Add `Failure`, `Trajectory` variants |
| `mcp_server/graph_store.py` | `crates/ainl-memory/src/store.rs` | Add `record_failure`, `record_trajectory`, `search_failures_fts5` |
| `mcp_server/trajectory_capture.py` | `crates/ainl-trajectory/` (`TrajectoryDraft`, `replay::TrajectoryReplayLine`) + `openfang-runtime` / `ainl-memory` writers + `crates/ainl-runtime/src/runtime.rs` (`maybe_persist_trajectory_after_episode`) | Per-step capture in host + embedded graph engine; JSONL replay line encoding in crate |
| `mcp_server/failure_learning.py` | `crates/ainl-memory` (`Failure` + FTS) + `openfang-runtime` (`LearningRecorder`, **`## FailureRecall`**, **`ainl-runtime` graph-validation ingest**) + `openfang-api` (`/failures/search`, **`/failures/recent`**) + `openfang-kernel` (SSE **`FailureLearned`**). Optional later: **`crates/ainl-failure-learning/`** for non-OpenFang embedders. | Host path is operational; standalone crate remains optional for ecosystem packaging |
| `mcp_server/persona_evolution.py` | `crates/ainl-persona/src/{engine,signals,extractor}.rs` (existing) | Already complete; keep α=0.2 |
| `mcp_server/extractor.py` | `crates/ainl-graph-extractor/…` + **`ainl_memory::pattern_promotion`** + **`openfang_runtime::graph_memory_writer::record_pattern` merge** (existing) | Promotion math + `prompt_eligible` live in **`ainl-memory` / `openfang-runtime`**, not only in the extractor task. |
| `mcp_server/improvement_proposals.py` | **new** `crates/ainl-improvement-proposals/` | 1:1 + uses `ainl-runtime` strict validator |
| `mcp_server/context_compiler.py` | **new** `crates/ainl-context-compiler/` | Hoist existing prompt blocks from `openfang-runtime` |
| `mcp_server/compression.py` | `crates/ainl-compression/src/lib.rs` (existing) | ✅ already implemented (richer than plugin) |
| `mcp_server/compression_profiles.py` + `project_profiles.py` | `crates/ainl-compression/src/profiles.rs` | **`BUILTIN_PROFILES`**, **`resolve_builtin_profile`**, **`suggest_profile_id_for_project`**; CLI **`openfang compression profiles list|show|map-project`**. |
| `mcp_server/adaptive_eco.py` | `crates/ainl-compression/src/adaptive.rs` | **`recommend_mode_for_content`**; CLI **`openfang compression adaptive suggest`**. |
| `mcp_server/cache_awareness.py` | `crates/ainl-compression/src/cache.rs` | **`effective_ttl_with_hysteresis`**, **`cache_policy_summary`**; CLI **`openfang compression cache ttl|policy`**. |
| `mcp_server/semantic_scoring.py` | `crates/ainl-compression::estimate_semantic_preservation_score` (existing) | ✅ already implemented |
| `mcp_server/output_compression.py` | `crates/ainl-compression/src/lib.rs` (extend) | Add response-side compression behind `AINL_OUTPUT_COMPRESSION=1` |
| `cli/memory_cli.py` | `crates/openfang-cli/src/main.rs` (`MemoryCommands` in `openfang memory …`) | KV: **`list` / `get` / `set` / `delete`** (daemon). Graph: **`graph-export`**, **`graph-search`**, **`graph-persona`**, **`graph-validate`** (offline DB), **`graph-audit`**, **`graph-inspect`**, **`graph-remember`**, **`graph-forget`** (daemon `/api/graph-memory/*`). |
| `cli/trajectory_cli.py` | `crates/openfang-cli/src/main.rs` (`TrajectoryCommands::{List, Search, Analyze, Export}`) | **`openfang trajectory list`** (table / `--json`), **`search`** (substring over recent rows, `--scan-limit`), **`analyze`** (aggregate stats, `--json`), **`export`** (JSONL) |
| `cli/compression_advanced_cli.py` | `crates/openfang-cli/src/main.rs` (`CompressionCommands`) | **`compression test|score|detect`**; **`compression profiles`** `list|show|map-project`; **`compression adaptive suggest`**; **`compression cache ttl|policy`** — backed by `ainl-compression` `profiles` / `adaptive` / `cache` modules. |
| `hooks/post_tool_use.py` | `crates/openfang-runtime/src/tool_runner.rs` + `agent_loop.rs` | Trajectory step append (`trajectory_turn`); tool **error** → `GraphMemoryWriter::record_tool_execution_failure` |
| `hooks/stop.py` | `crates/openfang-runtime/src/agent_loop.rs` EndTurn (extend) | Trajectory finalize; persona evolution already runs here |

---

## 11. Acceptance criteria (per CLAUDE.md *Live Integration Testing*)

After each phase, in addition to the workspace test trio, run the live verification loop from CLAUDE.md (start daemon → curl new endpoints → verify side effects → cleanup). Phase-specific live checks:

```bash
# Phase 1 — trajectories (requires ?agent_id=<your-agent>)
curl -s 'http://127.0.0.1:4200/api/trajectories?agent_id=YOUR_AGENT' | jq '.trajectories[] | .ainl_source_hash' | head

# Phase 2 — failures
# Trigger a known typo, verify suggestion block appears next turn
curl -sX POST http://127.0.0.1:4200/api/agents/$ID/message \
  -d '{"message":"run AINL with httP.GET (typo)"}'  # first call → records failure
curl -sX POST http://127.0.0.1:4200/api/agents/$ID/message \
  -d '{"message":"run AINL with httP.GET again"}'   # second call should include "I've seen this error before"

# Phase 5 — adaptive compression
curl -s http://127.0.0.1:4200/api/compression/profiles | jq '.[$PROJECT].optimal_mode'
```

Performance budgets (mirroring plugin targets, achievable in Rust):

| Operation | Plugin (Python) target | ArmaraOS (Rust) target |
|---|---|---|
| Trajectory capture | <50 ms | **<5 ms** |
| Persona update | <20 ms | **<2 ms** (already met by `ainl-persona`) |
| Failure FTS5 search | <50 ms | **<10 ms** |
| Pattern ranking | <100 ms | **<20 ms** |
| Context compilation | <200 ms | **<50 ms** |

---

## 12. What NOT to port

The plugin is built around the Claude Code MCP hook model. The following pieces have **no analog** in ArmaraOS (deliberately):

- `mcp_server/server.py` — MCP transport. ArmaraOS exposes the same data via `GET /api/graph-memory*` and dashboard, not via MCP.
- `hooks/ainl_detection.py` / `ainl_validator.py` — Claude-Code-specific user-prompt routing. ArmaraOS already validates AINL inside `ainl-runtime`.
- `templates/` and `profiles/test_*.json` — example-only.

These are reference implementations only; do not port.

---

## 13. Linkage back to existing ArmaraOS docs

When this plan ships, update the following docs in lockstep:

- **`docs/graph-memory.md`** — extend the *What gets written* table with `Failure` and `Trajectory` rows; document `project_id` column in *On-disk layout*
- **`docs/persona-evolution.md`** — add note about α=0.2 vs plugin's 0.3
- **`docs/architecture.md`** — add the three new crates to the crate graph
- **`ARCHITECTURE.md`** — add a *Self-learning loop* subsection mirroring the plugin README's diagram
- **`crates/openfang-runtime/README.md`** — list env toggles: **`AINL_LEARNING`** (master off for trajectory + failure stack), `AINL_TRAJECTORY_ENABLED`, `AINL_FAILURE_LEARNING_ENABLED`, `AINL_IMPROVEMENT_PROPOSALS_ENABLED`, `AINL_ADAPTIVE_COMPRESSION`, `AINL_COMPRESSION_CACHE_AWARE`, `AINL_MEMORY_PROJECT_SCOPE`; `GET /api/status` field **`graph_memory_learning_metrics`**
- **`PRIOR_ART.md`** — credit the plugin as the reference implementation that prototyped these patterns end-to-end in Python
- **`CLAUDE.md`** — add `/btw`-style examples for failure recall and trajectory replay

---

## 14. Quick "is this worth it?" calculus

The plugin claims, on its own data:

- **Pattern reuse rate:** >40 % of workflows use recalled patterns
- **Failure prevention:** >60 % of similar errors prevented
- **Token savings:** >40 % via adaptive compression
- **Time savings:** >30 % via pattern reuse

ArmaraOS already harvests the persona and partial-pattern win (`ainl-persona` + `ainl-graph-extractor`). The **delta** that this back-port unlocks is roughly:

- ~60 % of currently-repeated user errors prevented (Failure learning)
- An auditable closed loop for AINL workflow improvement (Proposals)
- Per-project compression tuning (today: agent-wide manual setting)
- A debuggable replay surface for every turn (Trajectories)

These are exactly the capabilities the **dashboard `#orchestration-traces`** and **`docs/ga-signoff-checklist.md`** ask for in the ArmaraOS GA story — so this work pays double duty as plugin parity *and* GA evidence.

---

## 15. Cross-crate integration (keeping new crates host-agnostic)

The new crates must integrate with the existing `ainl-*` ecosystem **without** depending on `openfang-runtime` / `openfang-types`. Two mechanisms make this work: **shared types in `ainl-contracts`** and **trait-based telemetry sinks**. This mirrors the pattern `ainl-compression` already uses (`CompressionTelemetrySink` + `with_telemetry_callback`), so embedding hosts wire their event bus / SSE / OpenTelemetry exporter once per sink trait and the crates stay portable.

### 15.1 Phase 0 prerequisite — `ainl-contracts` uplift

Today `ainl-contracts` carries `RepoIntelCapabilityProfile`, `ContextFreshness`, `ImpactDecision`, `RecommendedNextTools`, and the `telemetry::*` field-name constants. Add the following so every new learner has a deps-free vocabulary to speak:

```rust
// crates/ainl-contracts/src/lib.rs (additions, all serde-roundtrippable)

pub mod vitals {
    /// Lifted from openfang-types::vitals so non-openfang hosts can consume vitals.
    /// openfang-types::vitals becomes a thin re-export shim for backwards compat.
    pub use crate::vitals_inner::{CognitiveVitals, CognitivePhase, VitalsGate};
}

/// Stable trajectory step record — shared by ainl-trajectory and any host
/// that wants to assemble traces (ainl-runtime, ainl-inference-server, MCP server).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryStep {
    pub step_id: String,
    pub timestamp_ms: i64,
    pub adapter: String,           // canonicalised via ainl-semantic-tagger
    pub operation: String,
    pub duration_ms: u64,
    pub success: bool,
    pub error_kind: Option<FailureKind>,
    /// Optional vitals snapshot at step boundary.
    pub vitals: Option<vitals::CognitiveVitals>,
    /// Freshness at the moment of execution (see ainl-context-freshness).
    pub freshness_at_step: Option<ContextFreshness>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrajectoryOutcome { Success, PartialSuccess, Failure, Aborted }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "details")]
pub enum FailureKind {
    AdapterTypo { offered: String, suggestion: Option<String> },
    ValidatorReject { rule: String },
    AdapterTimeout { adapter: String, ms: u64 },
    ToolError { tool: String, message: String },
    LoopGuardFire { tool: String, repeat_count: u32 },
    Other { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposalEnvelope {
    pub schema_version: u32,
    pub original_hash: String,
    pub proposed_hash: String,
    pub kind: String,                       // "promote_pattern", "rewrite_typo", "split_step", …
    pub rationale: String,
    pub freshness_at_proposal: ContextFreshness,
    pub impact_decision: ImpactDecision,    // gate from ainl-impact-policy
}

pub mod telemetry {
    // Existing constants stay…
    pub const CAPABILITY_PROFILE_STATE: &str = "capability_profile_state";
    pub const FRESHNESS_STATE_AT_DECISION: &str = "freshness_state_at_decision";
    pub const IMPACT_CHECKED_BEFORE_WRITE: &str = "impact_checked_before_write";

    // NEW for the learner suite:
    pub const TRAJECTORY_RECORDED: &str = "trajectory_recorded";
    pub const TRAJECTORY_OUTCOME: &str = "trajectory_outcome";
    pub const TRAJECTORY_STEP_DURATION_MS: &str = "trajectory_step_duration_ms";
    pub const FAILURE_RECORDED: &str = "failure_recorded";
    pub const FAILURE_RESOLUTION_HIT: &str = "failure_resolution_hit";
    pub const FAILURE_PREVENTED_COUNT: &str = "failure_prevented_count";
    pub const PROPOSAL_VALIDATED: &str = "proposal_validated";
    pub const PROPOSAL_ADOPTED: &str = "proposal_adopted";
    pub const COMPRESSION_PROFILE_TUNED: &str = "compression_profile_tuned";
    pub const COMPRESSION_CACHE_HIT: &str = "compression_cache_hit";
    pub const PERSONA_AXIS_DELTA: &str = "persona_axis_delta";
    pub const VITALS_GATE_AT_TURN: &str = "vitals_gate_at_turn";
}
```

`openfang-types::vitals` becomes a one-line re-export of `ainl_contracts::vitals::*` so nothing in `openfang-runtime` or the dashboard breaks. `vitals_classifier` (which produces these types) stays in `openfang-runtime` because it is **provider-coupled** (logprob shape from OpenAI/OpenRouter) — only the type vocabulary moves.

### 15.2 Per-crate dependency map (what each new/extended crate consumes)

| New / extended crate | Reads from | Writes to | Telemetry sink trait | Why this shape |
|---|---|---|---|---|
| **`ainl-trajectory`** | `ainl-contracts::{vitals, ContextFreshness, TrajectoryStep, FailureKind}`<br>`ainl-semantic-tagger` (canonicalise adapter + tool names at write time)<br>`ainl-context-freshness` (record `freshness_at_step` on the step) | `ainl-memory` (`Trajectory` node + sibling table)<br>Optional sink (caller-provided event bus) | `pub trait TrajectoryTelemetrySink: Send + Sync { fn emit(&self, ev: TrajectoryEvent); }` | Trajectory is the substrate; every other learner reads from it. Tagging at write time means downstream consumers don't re-parse strings. |
| **`ainl-failure-learning`** | `ainl-contracts::{FailureKind, ContextFreshness}`<br>`ainl-trajectory` (the failing step it derived from)<br>`ainl-semantic-tagger` (correction / tone tags)<br>`ainl-context-freshness` (don't suggest stale resolutions)<br>`ainl-persona` (downstream consumer reads failure → bumps `Persistence` axis) | `ainl-memory` (`Failure` node + FTS5 row + `RESOLVES`/`FIXED_BY` edges) | `pub trait FailureTelemetrySink: Send + Sync { fn emit(&self, ev: FailureEvent); }` | The `freshness_at_failure` field is what prevents the system from re-suggesting a fix that worked against an out-of-date index. |
| **`ainl-improvement-proposals`** | `ainl-contracts::{ProposalEnvelope, ImpactDecision, ContextFreshness}`<br>`ainl-graph-extractor::recurrence` (recurrence drives proposal trigger)<br>`ainl-impact-policy` (`recommend_next_tools` → only allow proposals consistent with current `AuthoringPhase`)<br>`ainl-context-freshness` (block adoption when `Stale`)<br>`ainl-runtime` (strict validate via the existing parser) | `ainl-memory` (proposal ledger as `Procedural` nodes with `kind = "proposal"` flag, or own table) | `pub trait ProposalTelemetrySink: Send + Sync { fn emit(&self, ev: ProposalEvent); }` | Closed loop = "Recurrence detected → propose → impact-policy gate → freshness gate → strict validate → adopt". Each gate is a separate crate already in tree. |
| **`ainl-context-compiler`** (new) | **All** stores: `ainl-memory`, `ainl-trajectory`, `ainl-failure-learning`, `ainl-persona`, `ainl-improvement-proposals`<br>`ainl-context-freshness` (rank-down stale items)<br>`ainl-semantic-tagger` (filter blocks by `topic` relevance)<br>`ainl-compression` (apply per-block compression within budget) | Returns `Vec<ContextBlock>` — the host (openfang-runtime / ainl-runtime / ainl-inference-server) injects them into the prompt | `pub trait ContextEmissionSink: Send + Sync { fn emit(&self, ev: BlockEmittedEvent); }` | The compiler is **read-only** w.r.t. graph state — it composes, it doesn't mutate. Means it's safe to call from any host, including read-only inspection tools. |
| **`ainl-compression` extensions** (`profiles`, `adaptive`, `cache`) | `ainl-contracts::vitals::CognitiveVitals` (low-trust turns → never go aggressive)<br>`ainl-semantic-tagger` (topic-based content classification in `adaptive.rs`)<br>`ainl-context-freshness` (stale → never go aggressive) | `ainl_compression::profiles::ProjectProfile` JSON file (or table in `ainl_memory.db` when the host wants atomicity) | Existing `CompressionTelemetrySink` (already in tree) — extend `CompressionMetrics` with `mode_recommended_by` enum: `Manual`/`AdaptiveContent`/`AdaptiveProject`/`CacheHysteresis` | `CompressionTelemetrySink` is the canonical sink pattern in the workspace; new sinks should match its signature exactly. |
| **`ainl-memory` extensions** | `ainl-contracts::FailureKind` (typed failure payload column)<br>`ainl-semantic-tagger` (auto-populate `embedding_text` for FTS5 from canonical tags + summary) | n/a (it is the store) | n/a (the writer crates emit telemetry, not the store) | Putting tagger here means **every** writer benefits from FTS5 quality without each having to re-tag. |

### 15.3 Vitals integration — concrete signal map

`vitals_classifier::classify(tokens) -> Option<CognitiveVitals { gate, phase, trust, mean_logprob, entropy, sample_tokens }>` runs once per LLM completion in `openfang-runtime`. The vitals snapshot should fan out to:

| Consumer | What it does with vitals |
|---|---|
| `ainl-trajectory` | Stamps each `TrajectoryStep` with the vitals snapshot active at that step. Enables post-hoc analysis of "tools that ran during low-trust turns failed N% more often". |
| `ainl-failure-learning` | A failure recorded during `gate = Block` is **not** auto-promoted into a "suggested resolution" — the model itself was uncertain, the resolution may be wrong. Stored, but flagged `auto_suggest = false`. |
| `ainl-improvement-proposals` | `policy.rs` rule: `accept_if vitals.trust > 0.7 AND vitals.gate != Block AND validation_passed`. High-trust + valid + recurring → adopt; everything else → keep in ledger for human review. |
| `ainl-persona` | Already in tree as a signal source via `ainl-graph-extractor::persona_signals`. Add a `vitals → Persistence` axis edge: `gate = Retry` raises Persistence reward weight; `gate = Block` lowers Curiosity. |
| `ainl-compression::adaptive` | `vitals.trust < 0.5` → cap at `Balanced` regardless of content classification (don't compound model uncertainty with information loss). |
| `ainl-context-compiler::sources::active_persona` | Annotate emitted persona block with vitals-at-evolution so the dashboard can show "this trait moved during a high-trust turn — confidence high". |

This is the single biggest win from the Phase 0 contracts uplift: **today vitals are openfang-runtime-internal**, after Phase 0 they become a first-class learner input across the whole workspace.

### 15.4 Telemetry sink pattern (canonical)

Mirror `ainl_compression::PromptCompressor::with_telemetry_callback` exactly. Every new crate ships a `…TelemetrySink` trait + a builder method that takes an `Option<Arc<dyn …Sink>>`:

```rust
// In ainl-trajectory/src/lib.rs

pub trait TrajectoryTelemetrySink: Send + Sync {
    fn emit(&self, event: TrajectoryEvent);
}

#[derive(Debug, Clone)]
pub enum TrajectoryEvent {
    Recorded { trajectory_id: String, episode_id: String, outcome: TrajectoryOutcome, step_count: usize, duration_ms: u64 },
    StepRecorded { trajectory_id: String, step: TrajectoryStep },
    Pruned { trajectory_id: String, reason: &'static str },
}

pub struct TrajectoryRecorder {
    store: TrajectoryStore,
    sink: Option<Arc<dyn TrajectoryTelemetrySink>>,
}

impl TrajectoryRecorder {
    pub fn with_sink(store: TrajectoryStore, sink: Arc<dyn TrajectoryTelemetrySink>) -> Self {
        Self { store, sink: Some(sink) }
    }
    // … recording methods call `self.sink.as_ref().map(|s| s.emit(ev))` after every store write
}
```

In `openfang-runtime`, a single sink impl bridges all four trait families to the existing `SystemEvent` SSE stream + audit log + budget tracker. Outside armaraos (e.g. the Python `ainativelang` MCP server using PyO3 bindings, or `ainl-inference-server`), implementors provide their own sink — or pass `None` to opt out entirely.

Telemetry field names **must** come from `ainl_contracts::telemetry::*` so dashboards, Prometheus exporters, and CI gates can reference them consistently across hosts.

### 15.5 Cargo features matrix (keeping crates standalone)

Every new crate gets the same feature shape so embedders can pick the level of integration they want:

```toml
# Example: crates/ainl-trajectory/Cargo.toml
[features]
default = ["sqlite"]
sqlite       = ["dep:rusqlite"]                # in-tree default
in-memory    = []                              # for tests / inference-server
freshness    = ["dep:ainl-context-freshness"]  # opt-in freshness stamping
tagger       = ["dep:ainl-semantic-tagger"]    # opt-in tag normalisation
vitals       = []                              # always available via ainl-contracts
serde        = []                              # always (used internally)
graph-export = ["dep:serde_json"]              # JSONL replay export
```

Slim builds for non-armaraos hosts:
```bash
# ainl-inference-server consumer (no SQLite, in-memory only)
cargo build -p ainl-trajectory --no-default-features --features in-memory,vitals,graph-export

# ArmaraOS embedded host (everything)
cargo build -p ainl-trajectory   # all defaults
```

This matches the pattern already used by `ainl-compression` (`graph-telemetry` feature) and `openfang-runtime` (`ainl-extractor`, `ainl-tagger`, `ainl-persona-evolution` feature gates).

### 15.6 Updated dependency graph

```
                        ainl-contracts
                  (vitals, freshness, impact, telemetry consts,
                   TrajectoryStep, FailureKind, ProposalEnvelope)
                              ▲ ▲ ▲ ▲ ▲ ▲
        ┌─────────────┬───────┘ │ │ │ │ └────────┬─────────────┐
        │             │         │ │ │ │          │             │
ainl-context-     ainl-impact-  │ │ │ │     ainl-semantic-   ainl-repo-
freshness         policy        │ │ │ │     tagger           intel
        ▲             ▲         │ │ │ │          ▲
        │             │         │ │ │ │          │
        └──────┬──────┘         │ │ │ │          │
               │                │ │ │ │          │
               ▼                │ │ │ │          │
       ┌───────────────┐        │ │ │ │   ┌──────┴──────┐
       │ ainl-memory   │◄───────┘ │ │ │   │             │
       │ + Failure     │          │ │ │   │             │
       │ + Trajectory  │          │ │ │   │             │
       │ + project_id  │          │ │ │   │             │
       └───────▲───────┘          │ │ │   │             │
               │                  │ │ │   │             │
        ┌──────┴──────┐ ┌─────────┴─┴─┴───┴─┐ ┌─────────┴──────┐
        │             │ │                   │ │                │
ainl-trajectory   ainl-failure-      ainl-improvement-   ainl-graph-
    (NEW)          learning (NEW)    proposals (NEW)     extractor
        ▲             ▲                   ▲                   ▲
        │             │                   │                   │
        └─────────┬───┴────────────┬──────┴───────────────────┘
                  │                │
                  ▼                ▼
            ainl-context-compiler (NEW) ──reads──► ainl-persona
                  │                                        ▲
                  ▼                                        │
            ainl-compression                               │
            (+ profiles, adaptive, cache)                  │
                  │                                        │
                  └────── consumes vitals via ainl-contracts ┘

                    ─── ALL crates above are openfang-free ───
                                       │
                                       ▼
                         ┌─────────────────────────────┐
                         │       openfang-runtime      │
                         │  (provides telemetry sinks, │
                         │   vitals_classifier impl,   │
                         │   GraphMemoryWriter wiring) │
                         └─────────────────────────────┘
                                       │
                                       ▼
                         ┌─────────────────────────────┐
                         │   ainl-inference-server     │
                         │   ainativelang Python MCP   │
                         │   (other hosts)             │
                         │  — provide their own sinks  │
                         └─────────────────────────────┘
```

The bottom row is what makes the new crates **portable**: any host that can implement four telemetry sink traits and supply a `GraphMemory` (or no-op store) can adopt the entire learner stack.

### 15.7 Improvement proposals (shipped; cross-host contract)

**On-disk:** per-agent ledger at `~/.armaraos/agents/<agent_id>/.graph-memory/improvement_proposals.db` (crate `ainl-improvement-proposals`). Hash integrity: `submit` only accepts `proposed_ainl_text` that matches `ProposalEnvelope.proposed_hash`.

**Environment (host `openfang-runtime::improvement_proposals_host`)**

| Variable | Role |
|---|---|
| `AINL_IMPROVEMENT_PROPOSALS_ENABLED` | Master gate: `1` / `true` / `yes` / `on` enables submit, validate, adopt, and the HTTP list route. Default **off**. |
| `AINL_IMPROVEMENT_PROPOSALS_DEFAULT_VALIDATE_MODE` | When a client omits `mode`: `structural` (default) \| `strict` \| `external`. Maps to `ValidateMode` in `improvement_proposals_validators`. |
| `AINL_IMPROVEMENT_PROPOSALS_EXTERNAL_VALIDATE` | Optional `sh -c` template for **external** mode; must contain `%p` (replaced with a temp UTF-8 `.ainl` path). If unset under `external`, strict line checks still run; external step is a no-op. If set without `%p`, validation errors. |

**HTTP (`openfang-api` `graph_memory` routes)** — all require a sanitized `agent_id` and a truthy `AINL_IMPROVEMENT_PROPOSALS_ENABLED` on the daemon (503 when disabled).

| Method | Path | Body / query | Notes |
|---|---|---|---|
| `GET` | `/api/graph-memory/improvement-proposals` | `?agent_id=…&limit=…` | Returns `{ "ok", "proposals": ImprovementProposalListItem[] }` (no large text columns). |
| `POST` | `…/submit` | `agent_id`, `ProposalEnvelope` JSON, `proposed_ainl_text` | Append-only ledger row. |
| `POST` | `…/validate` | `agent_id`, `proposal_id`, optional `mode` | `mode`: `structural` \| `strict` \| `external` (overrides default for this call). |
| `POST` | `…/adopt` | `agent_id`, `proposal_id` | Materializes into `ainl_memory.db`; response includes `idempotent` when the adopt was a no-op or **ledger repair** (graph node existed, ledger not yet marked). |

**Adoption shape (host)**

- Envelope `kind` matching `pattern_promote` / `pattern-promote` / `pattern promote` → **procedural** `AinlMemoryNode::new_pattern` (opaque AINL bytes, `trace_id` = proposal UUID, `prompt_eligible: false`).
- All other kinds → **semantic** fact + JSON `topic_cluster` receipt (`improvement_proposal_adopted` tags including `proposal:<uuid>`).

**Idempotency and repair:** If the ledger already has `adopted_at` + `adopted_graph_node_id` and the node still exists, adopt returns success with `idempotent: true`. If a tagged graph node exists but the ledger was not updated (e.g. `mark_adopted` failed after `write_node`), adopt **repairs** the ledger; metrics increment `adopt_ledger_repair` and `adopt_idempotent` as in `improvement_proposals_host::metrics_snapshot()`.

**Cross-host telemetry parity:** `metrics_snapshot` exposes both raw counters (`validate_accepted`, `adopt_to_graph_ok`, `adopt_idempotent`, `adopt_ledger_repair`, …) and the shared keys `ainl_contracts::telemetry::PROPOSAL_VALIDATED` / `PROPOSAL_ADOPTED` (totals for validated and adopted) so other hosts (Python MCP, `ainl-inference-server`, scheduled `ainl run`) can emit the same field names if they call the `ainl-improvement-proposals` ledger API through a thin wrapper.

**Operator UX:** Dashboard **Graph memory** page includes an **Improvement proposals (ledger)** table (kind, state, id, **Focus** → `selectNodeById` when `adopted_graph_node_id` is set).

---

## 16. Updated phase gates (cross-crate verification)

In addition to the per-phase tests in §8, each phase must verify the **boundary contract** with the rest of the `ainl-*` ecosystem:

| Phase | Boundary verification (must pass before merge) |
|---|---|
| **0** | `cargo build -p ainl-contracts --no-default-features` succeeds; `cargo test -p openfang-types` shows the re-export shim is byte-equivalent JSON to the old type; downstream `cargo build -p openfang-runtime` is green without code changes. |
| **1** | `ainl-trajectory` builds with `--no-default-features --features in-memory` (proves it works without SQLite for `ainl-inference-server`). Tagger integration test: a step with `adapter = "shell"` is canonicalised to `"bash"` via `ainl-semantic-tagger` before write. |
| **2** | `ainl-failure-learning` rejects auto-suggestion when `freshness_at_failure = Stale` — explicit unit test. FTS5 lookup integration test against an `ainl-memory` store seeded with 3 failure rows. |
| **3** | Promotion / pattern persist skips when `vitals.gate = fail` (maps doc “Block” → [`VitalsGate::Fail`]). Tests in `ainl-memory` `pattern_promotion`. (Optional: per-episode linkage still future.) |
| **4** | `ainl-improvement-proposals` round-trip test: `ProposalEnvelope` JSON survives serialise → deserialise → `ainl-runtime::validate_strict` → adopt. Uses `ainl-impact-policy::recommend_next_tools` to refuse proposals out of phase. |
| **5** | `ainl-compression::adaptive` returns `Balanced` (not `Aggressive`) when the input describes a code task **or** vitals trust is below 0.5 — both branches tested. `cache.rs` hysteresis test: 4 mode flips inside a 5-min window must collapse to ≤ 1 actual change. |
| **6** | `ainl-context-compiler` compiles with each source feature individually disabled (`--no-default-features --features sqlite,active_persona` etc.) to prove sources are independently optional. |
| **7** | CLI subcommands shell out to crate APIs only — `cargo build -p openfang-cli --no-default-features` (CLI-minus-host) still produces a usable binary that operates on a standalone `ainl_memory.db`. |
| **8** | Dashboard SSE event types use `ainl_contracts::telemetry::*` constants verbatim — grep test in `openfang-api` against the contracts module. |

This way, every phase pays double:

1. The new capability lands in armaraos (the integration win).
2. The new capability ships as a standalone `ainl-*` crate that any other AINL host can consume (the ecosystem win).

---

## 17. Documentation deltas (companion to §13)

In addition to the existing doc updates in §13, Phase 0 + §15 require:

- **`crates/ainl-contracts/README.md`** — document the new `vitals`, `TrajectoryStep`, `FailureKind`, `ProposalEnvelope` types and their schema versions
- **`crates/openfang-types/src/vitals.rs`** — single-line re-export comment pointing at `ainl_contracts::vitals`; deprecation note
- **`docs/architecture.md`** — add the dependency graph from §15.6
- **`docs/learning-frame-v1.md`** — extend the vitals-signal table with the per-learner consumption rules from §15.3
- **`docs/ainl-crates-publish.md`** — add the new crates (`ainl-trajectory`, `ainl-failure-learning`, `ainl-improvement-proposals`, `ainl-context-compiler`) to the publish matrix; declare them as `no-openfang-deps` in the README badges
- **External: `ainl-inference-server/AGENTS.md`** — note that the same crates are available there with the `--no-default-features --features in-memory` slim build
