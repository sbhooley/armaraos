# Self-Learning Integration Map (AINL Graph Memory ⇄ ArmaraOS)

> **Source plugin:** `~/.claude/plugins/ainl-graph-memory` (Python MCP, ≈ v0.2)
> **Target:** Rust crates under `crates/ainl-*` and the embedding host in `openfang-runtime`.
> **Goal:** Bring Hermes-style self-learning (trajectory capture, failure learning, adaptive compression, persona evolution, closed-loop validation) into ArmaraOS as first-class crates so every embedded host (`openfang-runtime`, scheduled `ainl run`, future hosts) gets it for free.

The plugin explicitly attributes its design to ArmaraOS (`ainl-memory`, `ainl-persona`, `ainl-runtime`, `ainl-compression`, `ainl-graph-extractor`, `ainl-semantic-tagger`) — so this document is a **back-port and parity ledger**: it tracks what the Python reference already proved, what landed in the Rust stack, and what is still **optional “depth”** (richer heuristics, host-only fields, extra automation) or **operational** process (full workspace checks, *live* daemon curls, lockstep doc updates), not a missing “first mile” for core self-learning.

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
| **Typed graph memory** (`ainl_graph_nodes`, `ainl_graph_edges`, FTS5) | `ainl-memory::GraphMemory` (SQLite, WAL) + `Failure` kind + `ainl_failures_fts` (FTS5) + `ainl_nodes_fts` (all-node body search, migrated + backfilled) + typed edges; 6+ kinds including `Failure` and `Trajectory`. | **Shipped** per agent DB; optional cross-project scoping when `AINL_MEMORY_PROJECT_SCOPE` is on. | `ainl-memory` |
| **Project isolation** (`project_id` column) | `ainl_graph_nodes.project_id` + `ainl_nodes_fts` columns; `AINL_MEMORY_PROJECT_SCOPE=1` opt-in in `openfang-runtime` | **Shipped (opt-in):** per-agent default remains global; multi-project agents can enable project-scoped search/writes. | `ainl-memory` + `openfang-runtime` |
| **Context-aware retrieval** (`AINLContextCompiler`, 500-tok budget, priority 1-3) | `crates/ainl-context-compiler` (Tier 0) + per-turn `compose_telemetry` in `openfang-runtime` | **Core shipped:** `ContextCompiler::compose` + kernel side-channel. **Deeper product work (optional):** Tier-1/2 `summarizer` / `embedder` env gates, finer tool/segment budget scoring beyond Phase 6 defaults. | `ainl-context-compiler` + hosts |
| **Graceful degradation** (hooks never crash CC) | `GraphMemoryWriter::open` is non-fatal; persona pass returns `ExtractionReport`, not `Result<(), String>` | ✅ parity. Apply same pattern to all new modules. | n/a |
| **CLI inspection** (`memory_cli.py`, `compression_cli.py`, `trajectory_cli.py`) | Dashboard `#graph-memory`, `GET /api/graph-memory*`, `openfang orchestration …` | **Near parity:** `openfang trajectory …`, `openfang memory …`, **`openfang compression`** (`test|score|detect`, `profiles|adaptive|cache`, **`project-profiles show|set`** for on-disk EMA JSON + `GET /api/compression/project-profiles`). Deeper parity vs the Python plugin (extra introspection) can still grow. | `openfang-cli` |
| **Zero-LLM persona evolution** (5 axes, EMA α=0.3) | `ainl-persona` (5 axes: `Instrumentality`, `Verbosity`, `Persistence`, `Systematicity`, `Curiosity`, `EMA_ALPHA = 0.2`) + `PersonaEvolutionHook` | ✅ feature parity. Note α delta: armaraos = **0.2**, plugin = **0.3**. Keep armaraos value (more conservative; less overfitting on short sessions). | n/a (alignment only) |
| **Trajectory capture** (`ExecutionTrajectory` w/ `ainl_source_hash`, frame vars, adapters, per-step latency, outcome, fitness delta) | `ainl-memory` persists `Trajectory` nodes + `ainl_trajectories` rows (`persist_trajectory_for_episode`). `AINL_SOURCE_HASH` / `session_id` / `project_id` flow from env + trace JSON. **`ainl-runtime::run_turn` / `run_turn_async`** record **per procedural patch** (adapter name, `execute_patch` duration, success, output preview) **then** normalized **tool** steps. **`openfang-runtime`** owns per-host tool-call traces in `tool_runner` / EndTurn; **dashboard + SSE** for trajectories/failures/proposals (Phase 8) **shipped**. **Detail-table retention (product v1):** `openfang trajectory prune` + `--dry-run` deletes (or counts) `ainl_trajectories` rows older than a cutoff; **graph** `Trajectory` nodes are *not* removed. | **Partial gap (optional depth):** even richer per-step **frame** / **fitness** on stored step bodies, more host-only tool fields, **graph**-level / automatic TTL + dashboard control (see §9) — not blockers for core use. | `ainl-trajectory` (replay/JSONL) + `ainl-memory` (store) + hosts |
| **Pattern promotion** (min observations + EMA fitness ≥ 0.7; optional env `AINL_PATTERN_PROMOTION_*`) | `ainl_memory::pattern_promotion`, `GraphMemory::find_procedural_by_tool_sequence`, `GraphMemoryWriter::record_pattern` (merge + EMA), `ProceduralNode::{pattern_observation_count, prompt_eligible}`. | **Shipped (host):** prompt-time `SuggestedProcedure` in `graph_memory_context` only lists `prompt_eligible` procedurals; candidates stay in the graph until the gate trips. | `ainl-memory` + `openfang-runtime` |
| **Failure learning** (`failure_resolutions` table, FTS5 lookup, prevention prompt) | **`ainl-memory`:** `Failure` nodes, **`ainl_failures_fts`**, `search_failures_for_agent` — same failure surface as the plugin’s FTS idea, scoped per agent DB. **Host:** `LearningRecorder`, **`## FailureRecall`**, **`/api/graph-memory/failures/*`**, **`FailureLearned`** + graph writes. | Ecosystem: optional portable **`ainl-failure-learning`**; optional **`failure_resolutions`** edge table. | `ainl-memory` + `openfang-runtime` (shipped) |
| **Smart suggestions** (context-aware recommendations from history) | Graph-memory prompt blocks + optional **`## SuggestedNext`** (`AINL_MEMORY_INCLUDE_SUGGESTED_NEXT=1`) heuristics from top procedure / pattern / fact. | Heuristic channel shipped; **LLM-ranked** “next best action” remains optional. | `openfang-runtime` + `ainl-context-compiler` |
| **Closed-loop validation** (proposal → strict validate → adopt; tracks accept rate) | `ainl-improvement-proposals` (SQLite ledger) + `openfang-runtime::{improvement_proposals_host, improvement_proposals_validators}`; HTTP + dashboard on Graph Memory. | **Shipped** (default **on**; set `AINL_IMPROVEMENT_PROPOSALS_ENABLED=0` / `false` / `no` / `off` to disable). | `ainl-improvement-proposals` + `openfang-runtime` + `openfang-api` |
| **Adaptive compression** (per-project mode learning, semantic scoring, cache TTL coordination, adaptive eco by content type) | `ainl-compression`: `EfficientMode`, `compress()`, `CompressionMetrics`, `estimate_semantic_preservation_score`, plus `profiles` / `adaptive` / `cache` (built-ins + heuristics). **Host:** kernel `[adaptive_eco]` + `resolve_adaptive_eco_turn` + **`AINL_ADAPTIVE_COMPRESSION`** (merges content recommendation, profile hint, cache stretch into `adaptive_eco` metadata). **On-disk EMA + API:** `compression_project_ema` / `AINL_COMPRESSION_PROJECT_EMA`, **`GET /api/compression/project-profiles`**, CLI **`openfang compression project-profiles`**. **Dashboard / config:** `efficient_mode = "adaptive"` (chat **eco ada** pill, Settings → Budget) forces the adaptive path per request even if `[adaptive_eco].enabled` is off. | **Shipped (core + EMA file).** Richer default **cache policy** matrix / DB-backed profiles remain optional. | `ainl-compression` + `openfang-runtime` + `openfang-kernel` |

**Optional depth (product, not a single PR):** the §1 “Gap” column for several rows is **done for core use**; follow-ups are LLM-ranked “next best” suggestions, richer per-step `TrajectoryStep` / `frame_vars` on stored detail bodies, optional `failure_resolutions` / `RESOLVES` edge materialization, deeper context-compiler **summarizer / embedder** defaults and **segment** scoring, and DB- or policy-matrix–backed adaptive **cache** profiles. Trajectory **retention** in SQLite is addressed by `openfang trajectory prune` (detail rows) plus exports; *graph* `Trajectory` nodes and automatic TTL are still a separate, optional pass.

**Hermes-parity procedure learning:** `ainl-contracts` now defines portable `ExperienceBundle`, `ProcedureArtifact`, `ProcedurePatch`, lifecycle, render-target, and reuse-outcome schemas. `ainl-procedure-learning` is a host-neutral crate for deterministic artifact distillation, reuse scoring, rendering (`SKILL.md`, `skill.toml`, AINL compact skeleton, Hand metadata), and failure-aware patch application. `ainl-trajectory` can cluster repeated trajectories into bundles, `ainl-failure-learning` can generate patch candidates from recurring failures, `ainl-improvement-proposals` recognizes procedure proposal kinds, and `ainl-memory` can persist validated artifacts as procedural graph nodes. In ArmaraOS, the same recurrence edge that auto-submits `pattern_promote` also stages a companion `procedure_mint` proposal (env `AINL_AUTO_SUBMIT_PROCEDURE_PROPOSALS`, default on). Validated `procedure_mint` rows adopt into prompt-eligible procedural graph nodes and appear as rich `SuggestedProcedure` prompt hints; `procedure_patch` rows validate as JSON and use the same Graph Memory proposal review/list/validate/adopt API.

**Summary:** the Phase 0 `ainl-contracts` uplift and the “net-new” learner crates in §3–7 are now largely **in-tree and wired**; remaining effort clusters into optional depth (above) and the **ongoing** quality bars in §7–8, **live** checks (§11), and **permanent** doc coherency (§13).

---

## 2. Node-kind extension to `ainl-memory`

> **Archive / context:** `Failure`, `Trajectory`, `project_id`, and related migrations are **already in** `crates/ainl-memory` as of this plan’s implementation. The Rust snippet and SQL below are **retained** as design history and migration reference — not an open to-do.

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

> **Shipped / how to read this section:** the crate is **in-tree** at `crates/ainl-trajectory/`. The directory sketch below is **design + API history**; the canonical persisted payload for turns lives in `ainl-memory`’s `ainl_trajectories` + graph `Trajectory` nodes (see `graph-memory.md`). New work here is *mostly* format polish and host wiring, not “create the crate.”

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

> **Shipped / how to read this section:** the crate **exists** (`crates/ainl-failure-learning/`) and powers helpers such as `should_emit_failure_suggestion` alongside the host. The module list below is the original shape; the **source of truth** for on-disk `Failure` rows and FTS is `ainl-memory`. Optional packaging / extra edges remain backlog.

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

> **Shipped / how to read this section:** crate is **in-tree and wired** (ledger + `openfang-runtime` + HTTP; see **§15.7**). The layout below is **structural history**; feature details moved to the shipped rows in §8 / §15.7.

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

## 7. Crate: `ainl-context-compiler` (hoisted; in-tree)

> **Shipped / how to read this section:** the crate **exists and is on the default host path** (see Phase 6 in §8). The "optional" in older drafts meant *adopt across every host*; the **backlog** is Tier-1/2 summarizer/embedder defaults, finer segment/budget heuristics, and extra `MemoryBlock` sources — not a missing crate.

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

Each phase is **independently shippable** and gated by a Cargo feature so partial builds keep working. All phases must respect the standard verification trio from `CLAUDE.md` (`cargo build` / `cargo test` / `cargo clippy -D warnings` on the working set). The **clippy** line is a **standing quality bar** for the workspace — not a “phase deliverable” by itself.

```bash
cargo build --workspace --lib
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

| Phase | Deliverable | Crates touched | Feature flag | Gate |
|---|---|---|---|---|
| **0 — Contracts uplift** *(prerequisite, see §15)* | ✅ **Done:** `ainl-contracts` hosts `vitals`, `TrajectoryStep` / `TrajectoryOutcome` / `FailureKind` / `ProposalEnvelope`, `telemetry::*` learner keys, `CONTRACT_SCHEMA_VERSION` / `LEARNER_SCHEMA_VERSION`; `openfang-types::vitals` re-exports canonical types. **Gate:** JSON round-trip tests in `ainl-contracts`; `TypeId` + JSON wire tests in `openfang-types` proving re-export parity. | `ainl-contracts`, `openfang-types` | n/a (always on) | `cargo test -p ainl-contracts --lib` + `cargo test -p openfang-types vitals_` |
| **1 — Trajectory foundation** | ✅ **Done:** same as prior row **plus** `ainl-trajectory` **`replay` JSONL**; **`openfang trajectory list|search|analyze|export|prune`** (offline DB; `prune` is `ainl_trajectories` **detail** rows only + `--dry-run`). **`ainl-runtime`** mirrors graph-memory persistence: after each episode it writes **patch-dispatch steps then tool steps** via `persist_trajectory_for_episode` (not coarse-tools-only). **Dashboard + SSE:** see **Phase 8** (`#trajectories`). | `ainl-memory`, `ainl-trajectory`, `openfang-runtime`, `ainl-runtime`, `openfang-api`, `openfang-cli` | n/a | §11 curl + `cargo test -p ainl-trajectory --lib` + `cargo test -p ainl-runtime --all-features` |
| **2 — Failure learning** | **Shipped (host):** same persistence + API as prior row. **Host spine:** `openfang-runtime::graph_memory_learning::LearningRecorder` centralizes failure ingest (loop-guard, tool errors, hook/param precheck, **`ainl-runtime` graph validation** when `run_turn` fails with `graph validation` in the error text) with **message sanitization** + **metrics** (`graph_memory_learning_metrics` / `GET /api/status`). **In-prompt recall:** `graph_memory_context::build_prompt_memory_context` injects a **`## FailureRecall`** block from FTS (`search_failures_for_agent`), gated by the same learning policy (`learning.policy().failures`) + `MemoryContextPolicy::include_failure_recall` + `AINL_MEMORY_INCLUDE_FAILURE_RECALL` / `controls.json` **`include_failure_recall`**, **plus** `ainl_policy::workspace_policy_view` → when MCP-inferred `context_freshness` is **Stale**, `ainl-failure-learning::should_emit_failure_suggestion` **suppresses** that block (API lists / SSE for operators are unchanged). **Operator API:** `GET /api/graph-memory/failures/recent` lists recent `failure` nodes; **`GET /api/graph-memory/failures/search`** remains FTS. **SSE:** `FailureLearned` + `GraphMemoryWrite` (`kind=failure`) already publish from the kernel on graph writes. **Ecosystem (optional, in-tree):** `crates/ainl-failure-learning` (FTS + `should_emit_failure_suggestion` + prevention string helpers). | `ainl-memory`, `openfang-runtime`, `openfang-api`, `openfang-kernel`, `ainl-failure-learning` | `AINL_LEARNING` + subsystem envs | `cargo test -p openfang-runtime graph_memory_learning` + `cargo test -p ainl-memory` + `failure_recall` tests in `graph_memory_context` |
| **3 — Pattern promotion gate** | ✅ **Done:** `ainl_memory::pattern_promotion` (min observations + EMA + env tunables), `ProceduralNode::{pattern_observation_count, prompt_eligible}`, `GraphMemoryWriter::record_pattern` merge, vitals `gate=fail` skips persist, prompt blocks **`## SuggestedProcedure`** (promoted) + **`## SuggestedPatternCandidates`** (pre-promotion) + `controls` / `AINL_MEMORY_INCLUDE_SUGGESTED_PATTERN_CANDIDATES`. | `ainl-memory`, `openfang-runtime`, `openfang-api` (controls UI) | n/a | `cargo test` graph_memory_context + `ainl-memory` |
| **4 — Closed-loop validation** | ✅ **Shipped (end-to-end):** crate `ainl-improvement-proposals` (no `openfang` deps) + **`openfang-runtime`**: `improvement_proposals_host` + `improvement_proposals_validators` (`structural` / `strict` / `external` + `AINL_IMPROVEMENT_PROPOSALS_EXTERNAL_VALIDATE` `%p`). **HTTP** (`openfang-api` `graph_memory`): `GET/POST .../improvement-proposals` (list, submit, validate, adopt). **Dashboard** Graph Memory table. **Idempotency + ledger↔graph repair** + metrics. **Loop-driven auto-submission** of `pattern_promote` envelopes when `record_pattern_with_outcome` reports `just_promoted` (recurrence trigger; default on; **§15.7.1**). Details: **§15.7** + **§15.7.1** and env `AINL_IMPROVEMENT_PROPOSALS_ENABLED`, `…DEFAULT_VALIDATE_MODE`, `AINL_AUTO_SUBMIT_PATTERN_PROPOSALS`, `AINL_AUTO_VALIDATE_PATTERN_PROPOSALS`. | `ainl-improvement-proposals`, `openfang-runtime`, `openfang-api` | on by default; **opt out** with `…ENABLED=0|false|no|off` (master) and `AINL_AUTO_SUBMIT_PATTERN_PROPOSALS=0|false|no|off` or per-agent manifest (auto-submit only); see §15.7 / §15.7.1 | `cargo test -p ainl-improvement-proposals` + `cargo test -p openfang-runtime --lib improvement_proposals_host:: graph_memory_learning:: graph_memory_writer::` |
| **5 — Adaptive compression** | ✅ **Crates:** `ainl-compression` ships `profiles`, `adaptive` (`recommend_mode_for_content`), `cache` (`effective_ttl_with_hysteresis`). ✅ **Host:** `[adaptive_eco]` in kernel + `openfang_runtime::eco_mode_resolver::resolve_adaptive_eco_turn` — when `AINL_ADAPTIVE_COMPRESSION=1` (and `[adaptive_eco].enabled`), merges `ainl_compression` content recommendations into `recommended_mode`, adds profile id hint from `metadata.project_id` / `suggest_profile_id_for_project`, and **cache stretched TTL** metadata for prompt-cache lines. `agent_loop` still applies `compress_with_metrics` with manifest `efficient_mode` (kernel-injected). Per-agent JSON profile persistence (full EMA) remains future. | `ainl-compression`, `openfang-runtime`, `openfang-kernel` | `AINL_ADAPTIVE_COMPRESSION` + `config.toml` `[adaptive_eco]` | `cargo test -p openfang-runtime eco_mode_resolver` + adaptive eco harness; cache hysteresis tests in `ainl-compression` |
| **6 — Context compiler hoist** *(pulled forward; runs ahead of Phases 3–5 — see Cursor plan `ainl_context_engine_d20f60d3` for sequencing rationale)* | `ainl-context-compiler` in-tree. Whole-prompt orchestration (System/User/RecentTurn/OlderTurn/ToolResult) + optional graph-memory and FTS failure material as **separate** `MemoryBlock` segments. **History expansion:** `compose_telemetry` maps `MessageContent::Text` to turn segments and **walks** `MessageContent::Blocks` into coalesced text, [`Segment::tool_result`](crates/ainl-context-compiler/src/segment.rs) (and thinking/image placeholders) so tool-heavy sessions are scored fairly. **M1 (measurement):** `process_compose_telemetry_for_turn` (via `context_compiler_for_telemetry()`) runs `ContextCompiler::compose` each turn; `record_compose_turn` / `take_compose_turn` to the kernel. **M1.5 (compiler-root measurement):** `AINL_COMPOSE_GRAPH_MEMORY_AS_SEGMENTS=1` + `PromptMemoryContext::to_memory_block_segments` (system = pre–graph string; model still gets legacy `system_prompt` until M2 apply). **Optional tiers (env, default off):** **`AINL_CONTEXT_COMPOSE_SUMMARIZER=1`** → in-process `HeuristicAnchorSummarizer` (Tier 1, no extra LLM). **`AINL_CONTEXT_COMPOSE_EMBED=1`** → [`PlaceholderEmbedder`](crates/ainl-context-compiler/src/embedder.rs) + cosine **rerank** of non-pinned segments vs user query. **M2 (apply):** `AINL_CONTEXT_COMPOSE_APPLY=1` replaces `system_prompt` + `messages` when *all* messages are plain `MessageContent::Text`. **Trajectory recap (opt-in in host):** `AINL_MEMORY_INCLUDE_TRAJECTORY_RECAP=1` + optional `AINL_MEMORY_TRAJECTORY_RECAP_MAX*`, `format_trajectory_recap_lines` in [`trajectory_recap`](crates/ainl-context-compiler/src/trajectory_recap.rs) → `## TrajectoryRecap` block. **Feature:** `sources-trajectory-recap` (default in `ainl-context-compiler`; can be omitted in slim builds). **Operator runbook:** [`crates/openfang-runtime/README.md`](../crates/openfang-runtime/README.md) (*Phase 6* + *M2 safe rollout*). | `ainl-context-compiler`, `openfang-runtime`, `openfang-kernel` | envs: `AINL_COMPOSE_*`, `AINL_CONTEXT_COMPOSE_APPLY`, `AINL_CONTEXT_COMPOSE_SUMMARIZER`, `AINL_CONTEXT_COMPOSE_EMBED` | `cargo test -p openfang-runtime compose_telemetry`; `cargo test -p ainl-context-compiler`; **`./scripts/verify-ainl-context-compiler-feature-matrix.sh`** (§16) |
| **7 — CLI surfaces** | **Done (surfaces):** same as prior rows; **`openfang compression`** includes **`project-profiles show|set`**, and **host** `compression_project_ema` + **`GET /api/compression/project-profiles`** ( **`AINL_COMPRESSION_PROJECT_EMA=1`** ). **Process bar (not a mappable “phase”):** CI does **not** replace **CLAUDE.md** *Live Integration Testing* for daemon/HTTP/SSE; run it when you change those layers. The **workspace** `build` / `test` / `clippy` trio (§8) is still the merge bar for **code** you touched. | `openfang-cli` | n/a | *Live* checks per `CLAUDE.md` |
| **8 — Dashboard panels** | ✅ **Shipped:** **`#trajectories`**, dedicated **`#graph-failures`** and **`#graph-proposals`** nav, **`GET /api/trajectories`**, **`GET /api/graph-memory/failures/recent`**, and SSE (`armaraos-kernel-event` / dashboard): **`TrajectoryRecorded`**, **`FailureLearned`**, **`GraphMemoryWrite`** (`trajectory` / `failure` / `improvement_proposal`); **`ImprovementProposalAdopted`** (alias for operator UX; telemetry keys remain `ainl_contracts::telemetry` — see `PROPOSAL_ADOPTED` for field-rate parity). **CI guard:** `openfang_api::dashboard_learning_panels_js_guard` tests. | `openfang-api` (`static/js/pages/{trajectories,graph-failures,graph-proposals}.js` + `index_body.html`) | n/a | `cargo test -p openfang-api dashboard_learning` + `scripts/verify-dashboard-smoke.sh` |

Estimated effort: 2 engineer-weeks per phase; phases 1+2 are the critical path because every other learner reads trajectories and writes failures.

---

## 9. Sequencing risks & mitigations

| Risk | Mitigation |
|---|---|
| **Schema migration on existing user DBs** (`~/.armaraos/agents/<id>/ainl_memory.db`) | All `ALTER TABLE` ops are nullable-column adds; FTS5 virtual table is `IF NOT EXISTS`. Bump `user_version` and add `migrate::up_to_v…`. Keep `import_graph(allow_dangling_edges=true)` semantics. |
| **WAL coexistence** with `ainl-runtime` `RuntimeStateNode` writer | Already-tested pattern (see graph-memory.md *Optional `ainl-runtime`*). Trajectory writer follows same `Arc<Mutex<_>>` discipline. |
| **Project isolation regression** for single-project agents | `project_id` defaults to `None`; resolver in `openfang-runtime` lazily computes from `ARMARAOS_HOME` + cwd hash and is **opt-in** via `AINL_MEMORY_PROJECT_SCOPE=1`. |
| **Persona α drift** (plugin uses 0.3, we use 0.2) | Keep ArmaraOS at `EMA_ALPHA = 0.2` — empirically less reactive on 5–10 turn sessions. Document in `persona-evolution.md`. |
| **Trajectory bloat** | **Mitigation (shipped for detail rows):** `openfang trajectory prune` + `--dry-run` removes old **`ainl_trajectories`** rows by cutoff; **graph** `Trajectory` nodes are unchanged (optional future: coordinated graph delete + automatic TTL + dashboard). Design-level caps in writers still help; **exports** + external archival remain the long-horizon story for compliance. |
| **Hook crash safety** | Mirror plugin's golden rule (`hooks never break the host`): every new writer wraps in `let _ = … ;` at the top-level call site; errors logged at `warn!` only, like `ExtractionReport`. |
| **Provider cache thrash** during phase 5 | `CacheCoordinator` ships with a kill switch (`AINL_COMPRESSION_CACHE_AWARE=0`). |

---

## 10. Direct file map (plugin → armaraos)

| Plugin file (Python) | ArmaraOS target (Rust) | Notes |
|---|---|---|
| `mcp_server/node_types.py` | `crates/ainl-memory/src/node.rs` | **Implemented in-tree** (`Failure` / `Trajectory` in `AinlNodeKind`). The §2 `enum` snippet is **pre-migration / design reference**, not a new task. |
| `mcp_server/graph_store.py` | `crates/ainl-memory/src/store.rs` | **Implemented:** `insert_trajectory_detail`, `list_trajectories_for_agent`, `delete_trajectory_details_before` / prune helpers, `search_failures_fts_for_agent`, full-graph FTS, etc. Early “add record_*” wording is **historical**. |
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
| `mcp_server/output_compression.py` | `openfang_runtime::assistant_output_compress` + `ainl-compression` **`compress` / `compress_with_metrics`** | **Shipped:** assistant final text (non-streaming + streaming agent loops) can be passed through heuristics when **`AINL_OUTPUT_COMPRESSION=1`** (optional **`AINL_OUTPUT_COMPRESSION_MODE`**). |
| `cli/memory_cli.py` | `crates/openfang-cli/src/main.rs` (`MemoryCommands` in `openfang memory …`) | KV: **`list` / `get` / `set` / `delete`** (daemon). Graph: **`graph-export`**, **`graph-search`**, **`graph-persona`**, **`graph-validate`** (offline DB), **`graph-audit`**, **`graph-inspect`**, **`graph-remember`**, **`graph-forget`** (daemon `/api/graph-memory/*`). |
| `cli/trajectory_cli.py` | `crates/openfang-cli/src/main.rs` (`TrajectoryCommands`) | **`openfang trajectory list`** (table / `--json`), **`search`**, **`analyze`**, **`export`** (JSONL), **`prune`** (`--before-recorded-at` **or** `--older-than-days`, optional `--dry-run` — detail table only) |
| `cli/compression_advanced_cli.py` | `crates/openfang-cli/src/main.rs` (`CompressionCommands`) | **`compression test|score|detect`**; **`compression profiles`** `list|show|map-project`; **`compression project-profiles`** `show|set`; **`compression adaptive suggest`**; **`compression cache ttl|policy`** — backed by `ainl-compression` and `openfang_runtime::compression_project_ema` where applicable. |
| `hooks/post_tool_use.py` | `crates/openfang-runtime/src/tool_runner.rs` + `agent_loop.rs` | Trajectory step append (`trajectory_turn`); tool **error** → `GraphMemoryWriter::record_tool_execution_failure` |
| `hooks/stop.py` | `crates/openfang-runtime/src/agent_loop.rs` EndTurn (extend) | Trajectory finalize; persona evolution already runs here |

---

## 11. Acceptance criteria (per CLAUDE.md *Live Integration Testing*)

These `curl` examples (and the spirit of **Phase 0–2** in `CLAUDE.md` *Live Integration Testing*) are the **operating contract** for behavior changes on the HTTP/SSE/daemon side: if an endpoint, event name, or response shape regresses, fix it *before* merge, even when unit and integration tests in CI are green.

After each relevant change, in addition to the **workspace** trio in §8, run the **live** verification loop from `CLAUDE.md` (start daemon → curl new endpoints → verify side effects → cleanup) where the phase touched network-visible surfaces. **Automation note:** not every `curl` here is (or will be) CI-robot-friendly; the map’s **phased commands (0–8)** are the *repeatable* merge checklist; §11 is the *interactive* one for I/O. Phase-specific **starting points** (adjust hosts/agents/IDs):

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

**Maintenance, not a one-time checkbox:** when you change graph-memory, trajectory, failure, compression, or compose behavior that operators see in `docs/*`, HTTP, or the dashboard, update the same doc set the feature touches — the same way you would update API contracts. The list below is the *usual* set; the exact file set follows the product surface you modified.

Keep these aligned when behavior or defaults move:

- **`docs/graph-memory.md`** — extend the *What gets written* table with `Failure` and `Trajectory` rows; document `project_id` column in *On-disk layout*
- **`docs/persona-evolution.md`** — add note about α=0.2 vs plugin's 0.3
- **`docs/architecture.md`** — add the three new crates to the crate graph
- **`ARCHITECTURE.md`** — add a *Self-learning loop* subsection mirroring the plugin README's diagram
- **`crates/openfang-runtime/README.md`** — list env toggles: **`AINL_LEARNING`** (master off for trajectory + failure stack), `AINL_TRAJECTORY_ENABLED`, `AINL_FAILURE_LEARNING_ENABLED`, **`AINL_IMPROVEMENT_PROPOSALS_ENABLED` (default on; set `0` / `false` / `no` / `off` to disable)**, **`AINL_AUTO_SUBMIT_PATTERN_PROPOSALS` (default on; opt-out gates loop-driven `pattern_promote` submissions — see §15.7.1)**, **`AINL_AUTO_VALIDATE_PATTERN_PROPOSALS` (default off; structurally validates auto-submitted proposals)**, `AINL_ADAPTIVE_COMPRESSION`, `AINL_COMPRESSION_CACHE_AWARE`, `AINL_MEMORY_PROJECT_SCOPE`; **Phase 6 compose:** `AINL_COMPOSE_GRAPH_MEMORY_AS_SEGMENTS` (graph blocks as `MemoryBlock` for compiler-root **telemetry**), `AINL_COMPOSE_FAILURE_RECALL` (FTS failure in compose), `AINL_CONTEXT_COMPOSE_APPLY` (M2 prompt swap, text-only transcripts), **`AINL_CONTEXT_COMPOSE_SUMMARIZER`**, **`AINL_CONTEXT_COMPOSE_EMBED`**; `GET /api/status` field **`graph_memory_learning_metrics`**
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
| `AINL_IMPROVEMENT_PROPOSALS_ENABLED` | **Default: on** when unset. **Opt out** with `0` / `false` / `no` / `off` (disables submit, validate, adopt, and the HTTP list route; returns 503 for those). |
| `AINL_IMPROVEMENT_PROPOSALS_DEFAULT_VALIDATE_MODE` | When a client omits `mode`: `structural` (default) \| `strict` \| `external`. Maps to `ValidateMode` in `improvement_proposals_validators`. |
| `AINL_IMPROVEMENT_PROPOSALS_EXTERNAL_VALIDATE` | Optional `sh -c` template for **external** mode; must contain `%p` (replaced with a temp UTF-8 `.ainl` path). If unset under `external`, strict line checks still run; external step is a no-op. If set without `%p`, validation errors. |

**HTTP (`openfang-api` `graph_memory` routes)** — all require a sanitized `agent_id` and a **not opted-out** `AINL_IMPROVEMENT_PROPOSALS_ENABLED` (default on; 503 when set to `0` / `false` / `no` / `off`).

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

### 15.7.1 Auto-submission of `pattern_promote` proposals (loop-driven; default on)

The closed loop in §15.7 is no longer purely operator-initiated. When the agent loop records a tool sequence whose `ProceduralNode` crosses the [`ainl_memory::pattern_promotion::should_promote`](../crates/ainl-memory/src/pattern_promotion.rs) threshold (`MIN_OBSERVATIONS=3` and EMA `fitness ≥ 0.7`), the runtime auto-submits a `pattern_promote` `ProposalEnvelope` into the same ledger described above — **no LLM call**.

**Trigger (single source of truth).** [`graph_memory_writer::record_pattern_with_outcome`](../crates/openfang-runtime/src/graph_memory_writer.rs) is the only writer that flips a pattern's `prompt_eligible` from `false → true` and now returns a [`PatternUpsertOutcome`](../crates/openfang-runtime/src/graph_memory_writer.rs) carrying `just_promoted: bool` (true *exactly once* per node, on the call that crossed the threshold). `record_pattern` remains as a backwards-compatible wrapper. The agent loop ([`agent_loop`](../crates/openfang-runtime/src/agent_loop.rs)) consumes that outcome and forwards `just_promoted` patterns to `LearningRecorder::maybe_auto_submit_pattern_promotion`.

**Pipeline.** [`graph_memory_learning::LearningRecorder`](../crates/openfang-runtime/src/graph_memory_learning.rs) (`maybe_auto_submit_pattern_promotion`) → `tokio::task::spawn_blocking` → [`improvement_proposals_host::auto_submit_pattern_proposal`](../crates/openfang-runtime/src/improvement_proposals_host.rs):

1. **Build** a deterministic `ProposalEnvelope` + minimal AINL text via `build_pattern_promote_envelope` (kind `pattern_promote`, hash = `sha256_hex_lower(text)`, structural-validation safe).
2. **Dedup** against the ledger by `proposed_hash` + matching `kind` (no DB-level UNIQUE; host-level guard).
3. **Submit** through the existing `submit` path (same hash-integrity check the HTTP route enforces).
4. **Optional auto-validate**: when enabled, run `ValidateMode::Structural` immediately and record accept/reject via the same telemetry counters operators already see.

The agent loop is **never blocked** by ledger I/O — submission runs on a blocking executor task and only emits `tracing::debug!` / `warn!` on result.

**Environment (additive to §15.7)**

| Variable | Default | Role |
|---|---|---|
| `AINL_AUTO_SUBMIT_PATTERN_PROPOSALS` | **on** when unset | Master gate for loop-driven `pattern_promote` submissions. Off semantics: `0` / `false` / `no` / `off`. Implies `AINL_IMPROVEMENT_PROPOSALS_ENABLED` is also on (master proposals gate still wins). |
| `AINL_AUTO_VALIDATE_PATTERN_PROPOSALS` | **off** when unset | When truthy, also runs `ValidateMode::Structural` immediately after submit. Useful for fast operator review queues. |

**Per-agent opt-out (manifest).** `LearningStackPolicy` reads agent-manifest metadata via `manifest_auto_submit_opt_out`. Setting `ainl_auto_submit_pattern_proposals` (alias `auto_submit_pattern_proposals`) to `0` / `false` / `no` / `off` on an agent disables auto-submission for that agent only — useful for read-only or sandbox agents where ledger pollution would obscure operator-driven candidates.

**Telemetry (additive in `improvement_proposals_host::metrics_snapshot`).** New counters surface alongside the existing submit/validate/adopt totals so operators can graph the auto-loop without redeploying:

- `auto_submit_ok`, `auto_submit_dedup`, `auto_submit_disabled`, `auto_submit_error`
- `auto_validate_accepted`, `auto_validate_rejected`, `auto_validate_error`
- Boolean policy state: `auto_submit_env_enabled`, `auto_validate_env_enabled`

Standard `submit` / `validate_accepted` / shared `ainl_contracts::telemetry::PROPOSAL_VALIDATED` keys also tick when auto-submission fires, so existing dashboards continue to reflect the *full* ledger flow rather than splitting human and machine sources.

**Why this is closed-loop-friendly.** Recurrence detection ([`ainl-graph-extractor`](../crates/ainl-graph-extractor) recurrence + `ainl-memory::pattern_promotion`) already determines *when* a pattern is worth elevating. Before this hook, the operator had to notice the new `prompt_eligible` row, hand-craft an envelope, and POST `/api/graph-memory/improvement-proposals/submit`. Now the same recurrence signal directly seeds the ledger; humans (or the optional structural-validate auto-step) only handle the **decision** (validate / adopt / reject) — not authoring boilerplate.

**Tests.** `cargo test -p openfang-runtime --lib improvement_proposals_host::` covers envelope construction, hash matching, structural validation, env opt-out, and ledger dedup. `cargo test -p openfang-runtime --lib graph_memory_writer::record_pattern_with_outcome_signals_just_promoted_once_at_threshold` covers the trigger contract. `graph_memory_learning::tests::auto_submit_policy_*` covers the master-gate / env / manifest interaction matrix.

---

## 16. Updated phase gates (cross-crate verification)

The table is **evidence of ecosystem boundaries** — a few tests prove standalone `ainl-*` usability; *no* single command is required to re-prove the **entire** matrix on every host (see the notes in the Phase **2** row, etc.).

In addition to the per-phase tests in §8, each phase must verify the **boundary contract** with the rest of the `ainl-*` ecosystem *where the phase touches that boundary*:

| Phase | Boundary verification (must pass before merge) |
|---|---|
| **0** | `cargo build -p ainl-contracts --no-default-features` succeeds; `cargo test -p openfang-types` shows the re-export shim is byte-equivalent JSON to the old type; downstream `cargo build -p openfang-runtime` is green without code changes. |
| **1** | `ainl-trajectory` builds with `--no-default-features --features in-memory` (proves it works without SQLite for `ainl-inference-server`). Tagger integration test: a step with `adapter = "shell"` is canonicalised to `"bash"` via `ainl-semantic-tagger` before write. |
| **2** | `ainl-failure-learning::should_emit_failure_suggestion` is **false** when `freshness_at_failure = Stale` (unit tests in `gate.rs`); `openfang-runtime` **`graph_memory_context`**: `failure_recall_skipped_when_context_freshness_stale` wires the gate into `## FailureRecall` assembly. **Note:** fully aspirational for “all hosts”; not every test re-asserts in one command. FTS5 integration coverage remains in `ainl-memory` / host tests. |
| **3** | Promotion / pattern persist skips when `vitals.gate = fail` (maps doc “Block” → [`VitalsGate::Fail`]). Tests in `ainl-memory` `pattern_promotion`. (Optional: per-episode linkage still future.) |
| **4** | `ainl-improvement-proposals` round-trip test: `ProposalEnvelope` JSON survives serialise → deserialise → `ainl-runtime::validate_strict` → adopt. Uses `ainl-impact-policy::recommend_next_tools` to refuse proposals out of phase. |
| **5** | `ainl-compression::adaptive` returns `Balanced` (not `Aggressive`) when the input describes a code task **or** vitals trust is below 0.5 — both branches tested. `cache.rs` hysteresis test: 4 mode flips inside a 5-min window must collapse to ≤ 1 actual change. |
| **6** | `ainl-context-compiler`: run **`./scripts/verify-ainl-context-compiler-feature-matrix.sh`** from the armaraos root — for each default feature in `crates/ainl-context-compiler/Cargo.toml`, `cargo test -p ainl-context-compiler --no-default-features` with the *other* defaults still enabled. Also spot-check that `openfang-runtime` `compose_telemetry` tests pass with `cargo test -p openfang-runtime compose_telemetry`. |
| **7** | CLI subcommands shell out to crate APIs only — `cargo build -p openfang-cli --no-default-features` (CLI-minus-host) still produces a usable binary that operates on a standalone `ainl_memory.db`. |
| **8** | Operator learning panels: `cargo test -p openfang-api dashboard_learning` (static JS must reference kernel `SystemEvent` names used in SSE) + `scripts/verify-dashboard-smoke.sh` when touching `static/`. |

This way, every phase pays double:

1. The new capability lands in armaraos (the integration win).
2. The new capability ships as a standalone `ainl-*` crate that any other AINL host can consume (the ecosystem win).

---

## 17. Documentation deltas (companion to §13)

In addition to the doc set in §13, Phase 0 + §15 expect:

- **`crates/ainl-contracts/README.md`** — schema/version story for **`CONTRACT_SCHEMA_VERSION`** (policy payloads), **`LEARNER_SCHEMA_VERSION`** and **learner** wire types in `ainl_contracts::learner` / re-exports (`TrajectoryStep` / `TrajectoryOutcome` / `FailureKind` / `ProposalEnvelope`), the **`vitals`** module, and **`telemetry::*` / `context_compiler::*` string keys (what changes when you bump a version; pointer to this map §15.1 and tests in the crate)
- **`crates/openfang-types/src/vitals.rs`** — re-export points at `ainl_contracts::vitals`; new code should import `ainl-contracts` directly when possible (**done** in-module comment)
- **`docs/architecture.md`** — [AINL learner dependency graph](#ainl-learner-dependency-graph) (from §15.6)
- **`docs/learning-frame-v1.md`** — cognitive / vitals table for learner hooks (see *Cognitive vitals and learners* there)
- **`docs/ainl-crates-publish.md`** — publish matrix for `ainl-trajectory`, `ainl-failure-learning`, `ainl-improvement-proposals`, `ainl-context-compiler`, `ainl-contracts` (no-`openfang` deps for learner crates)
- **External: `ainl-inference-server/AGENTS.md`** — cross-repo: slim `ainl-*` / patch notes (**update** the sibling when inference-server work lands)
- **`scripts/verify-ainl-context-compiler-feature-matrix.sh`** — Phase 6 §16 gate: **`DEFAULT_FEATS` must list exactly** the `default` feature set in `crates/ainl-context-compiler/Cargo.toml` (add/remove when defaults change, then re-run the script on the `armaraos` root)
