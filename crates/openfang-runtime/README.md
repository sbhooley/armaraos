# openfang-runtime

Agent loop, tool execution, graph memory (`GraphMemoryWriter`), and related runtime services for ArmaraOS / OpenFang.

**Default Cargo features (ArmaraOS daemon):** **`ainl-persona-evolution`**, **`ainl-extractor`**, **`ainl-tagger`**. Slim builds can use **`--no-default-features`** and opt back in per feature (see below).

## Semantic tagging (`ainl-tagger`)

Feature **`ainl-tagger`** links **`ainl-semantic-tagger`** and wires [`ainl_semantic_tagger_bridge::SemanticTaggerBridge`] into graph-memory writes (episode + fact nodes). It is **on by default** in this workspace; distributors may ship a binary without it.

At **runtime**, tagger-derived strings are merged **only** when the variable is set to the literal **`1`** (after trim):

```bash
export AINL_TAGGER_ENABLED=1
```

If unset, or set to any other value (including `true` / `yes` / `on`), tag lists from this bridge stay **empty**—even when the feature is compiled in—so operators opt in explicitly.

Re-enable the dependency after **`--no-default-features`**:

```bash
cargo build -p openfang-runtime --features ainl-persona-evolution,ainl-extractor,ainl-tagger
```

Tests for the bridge (with the env var set inside the tests) run when this feature is enabled:

```bash
cargo test -p openfang-runtime --lib --features ainl-tagger test_tag_fact_returns_strings
cargo test -p openfang-runtime --lib --features ainl-tagger test_tag_episode_from_tool_sequence
```

## AINL Graph Extractor Integration (`ainl-extractor`)

Link the published **`ainl-graph-extractor`** crate (persona evolution pass + turn-scoped semantic tags) with Cargo feature **`ainl-extractor`**. It is enabled in **default** features alongside **`ainl-persona-evolution`**.

For **post-turn fact / procedural pattern extraction** in the agent loop, the structured crate path is **on by default** when the feature is compiled in. To opt out at runtime without recompiling, set:

```bash
export AINL_EXTRACTOR_ENABLED=0  # also: false, no, off
```

When the variable is absent (the normal case) or set to any other value, the crate path runs. This is an **opt-out** semantics — the reverse of the previous behaviour. If you have legacy shell scripts that set `AINL_EXTRACTOR_ENABLED=1` to enable the extractor, they continue to work (non-falsy = enabled).

**Persona evolution pass return type:** when **`ainl-extractor`** is enabled, **`GraphMemoryWriter::run_persona_evolution_pass`** returns **`ainl_graph_extractor::ExtractionReport`** (type-alias **`PersonaEvolutionExtractionReport`**). Without **`ainl-extractor`**, the same method returns a small stub report (see **`graph_memory_writer.rs`**). Inspect **`has_errors()`** and the **`extract_error` / `pattern_error` / `persona_error`** fields when present; the implementation logs one **`warn!`** per populated slot (signal merge vs pattern persistence vs persona write) so operators see partial extractor failures without failing the spawned task. **`AinlRuntime::run_turn`** maps the same three fields to distinct **`TurnPhase`** warnings — see **`docs/ainl-runtime.md`** (*Persona evolution pass*).

Slim build without the crates.io dependency:

```bash
cargo build -p openfang-runtime --no-default-features --features ainl-persona-evolution
```

## Graph-memory learning stack (trajectories + failures)

**Master switch — `AINL_LEARNING`:** when set to a falsy token (`0`, `false`, `no`, `off`), the agent loop disables **both** (a) per-turn trajectory slot capture **and** (b) typed `Failure` graph writes, regardless of the subsystem envs below. When **`AINL_LEARNING` is unset**, you can opt an agent out with **`manifest.metadata["ainl_learning"]`** using the same tokens.

When the master switch is **not** off, existing knobs apply: **`AINL_TRAJECTORY_ENABLED`**, **`AINL_FAILURE_LEARNING_ENABLED`** (see `graph_memory_writer`).

**Introspection:** `openfang_runtime::graph_memory_learning_metrics()` returns best-effort counters (recorded vs skipped vs write-none) for operators / status surfaces.

**Other self-learning / kernel toggles (see [SELF_LEARNING_INTEGRATION_MAP.md](../../docs/SELF_LEARNING_INTEGRATION_MAP.md) §13, §8):** `AINL_IMPROVEMENT_PROPOSALS_ENABLED` (improvement proposals — **on by default**; set to `0` / `false` / `no` / `off` to disable), `AINL_ADAPTIVE_COMPRESSION` + kernel `[adaptive_eco]`, `AINL_COMPRESSION_CACHE_AWARE` (see `ainl-compression` cache), `AINL_MEMORY_PROJECT_SCOPE=1` (per-project `project_id` for graph + FTS in `ainl-memory`).

| Env | Behavior |
|-----|----------|
| **`AINL_MEMORY_INCLUDE_TRAJECTORY_RECAP`** | When **truthy** (`1` / `true` / `yes` / `on`), `graph_memory_context::build_prompt_memory_context` pulls recent `ainl_trajectories` rows and appends a **`## TrajectoryRecap`** block (and `graph_trajectory_recap` `MemoryBlock` in `to_memory_block_segments`) using `ainl_context_compiler::format_trajectory_recap_lines`. **Default: off** (extra prompt bytes). |
| **`AINL_MEMORY_TRAJECTORY_RECAP_MAX`** | Optional `usize` cap (default `5`, max `20`) for how many **rows** to format. |
| **`AINL_MEMORY_TRAJECTORY_RECAP_MAX_OPS`** | Optional `usize` cap (default `4`, max `12`) of tool/adapter **operation** names to print per row. |
| **`AINL_COMPRESSION_PROJECT_EMA`** | **Truthify** to enable per-project compression EMA persistence / merge (`compression_project_ema`); also powers **`GET /api/compression/project-profiles`** in the operator UI. **Default: off** until explicitly enabled. |

## Phase 6: context compiler (`ainl_context_compiler`) and compose telemetry

The agent loop feeds the assembled system prompt, history, and current user message through **`ainl_context_compiler::ContextCompiler::compose`** for whole-prompt token estimates. Results are published via the **`compose_telemetry` side channel** (same hand-off pattern as `eco_telemetry`): the kernel’s compression recorder can consume `take_compose_turn(agent_id)` after each turn. Authoritative behavior is in [`src/compose_telemetry.rs`](src/compose_telemetry.rs).

| Env | When truthy (`1` / `true` / `yes` / `on`) | Notes |
|-----|-----------------------------------------|--------|
| **`AINL_COMPOSE_GRAPH_MEMORY_AS_SEGMENTS`** | Whole-prompt **telemetry** may use a “compiler-root” segment layout: `system` = the manifest/kernel string **before** the graph-memory prompt append, then optional failure-recall block, then one `MemoryBlock` per non-empty graph section (from `PromptMemoryContext::to_memory_block_segments`), then history and user. | The model still receives the **legacy** single `system_prompt` (graph block appended) unless M2 apply below replaces it. |
| **`AINL_COMPOSE_FAILURE_RECALL`** | Adds a FTS failure `MemoryBlock` after the system segment in the **compose** path. | **Default off** — avoids duplicating failure text that may already be in the graph-memory prompt block. |
| **`AINL_CONTEXT_COMPOSE_APPLY`** (M2) | After telemetry, the composed window **may** replace the in-memory `system_prompt` and `messages` passed to the driver. | **Strict:** apply runs only if **every** message in the turn uses `MessageContent::Text` (no `Blocks` / multimodal / tool-block transcripts). If any message is `Blocks`, apply is skipped (telemetry may still run). `map_composed_to_system_and_messages` also requires a non-empty message list; otherwise the host prompt is kept. **Default: off** (measurement only). |
| **`AINL_CONTEXT_COMPOSE_SUMMARIZER`** | Uses Tier 1 anchored **summarization** when the compiler must drop `OlderTurn` segments over budget (in-process summarizer; no extra HTTP by default in the stub). | **Default: off** (extra CPU; can change tier strings in `record_compose_turn`). |
| **`AINL_CONTEXT_COMPOSE_EMBED`** | Uses Tier 2 **embedding** rerank of score-ordered non-pinned segments when a host `Embedder` is available (`PlaceholderEmbedder` for tests; production wiring optional). | **Default: off** |

**Reference:** [docs/SELF_LEARNING_INTEGRATION_MAP.md](../../docs/SELF_LEARNING_INTEGRATION_MAP.md) (Phase 6), [`compose_telemetry.rs`](src/compose_telemetry.rs).

### M2 safe rollout and observability

- **Start with `AINL_CONTEXT_COMPOSE_APPLY` unset** — whole-prompt compression telemetry is recorded, but the LLM request body is not rewritten.
- **Canaries:** enable apply on a single process or a narrow slice of agents before a fleet default.
- **Preconditions for apply:** sessions where stored history is plain **text** messages. Tool-heavy or multimodal histories that use `MessageContent::Blocks` will log `compose M2: skipping prompt swap (non-text MessageContent in history)` and keep the host-assembled prompt.
- **What to watch:** `record_compose_turn` / `take_compose_turn` output (tier, `original_tokens` / `compressed_tokens`); daemon logs for M2 skip lines if apply is on but most turns are skipped. Dashboard “whole-prompt” figures reflect the **compiler-scored** window, which can diverge from the literal driver payload when `Blocks` are present and apply is off.

**Follow-up (product):** optional per-agent `manifest.metadata["ainl_context_compose_apply"]` to OR with the env (not required for basic rollout).

## Trajectory rows (`Trajectory` nodes)

After each successful **`record_turn`**, the loop may persist a **`Trajectory`** graph node (one coarse step per tool name on the turn) and an edge **`trajectory_of`** from that node to the episode row. This mirrors **extractor opt-out** semantics:

```bash
export AINL_TRAJECTORY_ENABLED=0  # also: false, no, off
```

When the variable is **unset**, trajectory recording is **on** (unless **`AINL_LEARNING`** turned the whole stack off). Falsy values disable it so you can turn off SQLite writes without rebuilding.

Optional **`AINL_MEMORY_PROJECT_ID`** (if set) is stored on the trajectory payload for future multi-project agents; when unset, the field is omitted.

## AINL graph memory JSON export (Python `ainl_graph_memory` bridge)

After persona evolution, **`run_agent_loop`** refreshes a JSON **`AgentGraphSnapshot`** so **ainativelang** **`GraphStore`** can load the same subgraph as dashboard SQLite without a manual **`openfang memory graph-export`**.

| Env | Rust behavior |
|-----|-----------------|
| **`AINL_GRAPH_MEMORY_ARMARAOS_EXPORT`** set (non-empty) | Treated as a **directory**. Writes **`{dir}/{agent_id}_graph_export.json`** (one file per agent — avoids multi-agent overwrites). |
| **Unset** | Writes **`{openfang_home_dir()}/agents/{agent_id}/ainl_graph_memory_export.json`** (next to **`ainl_memory.db`**, same home rules as **`GraphMemoryWriter::open`**). |

Entry point: **`graph_memory_writer::armaraos_graph_memory_export_json_path`**. Python resolution (directory vs **`.json`** file, auto-fallback when env unset) lives in **ainativelang** **`armaraos/bridge/ainl_graph_memory.py`** — see **[`docs/adapters/AINL_GRAPH_MEMORY.md`](https://github.com/sbhooley/ainativelang/blob/main/docs/adapters/AINL_GRAPH_MEMORY.md)**.

**Tests:** **`cargo test -p openfang-runtime --test armaraos_graph_export_json_path`**.

## Cognitive vitals (`vitals_classifier`)

When the OpenAI driver (or any OpenRouter passthrough that surfaces logprobs) returns token logprob data, the runtime classifies it into a **`CognitiveVitals`** reading and attaches it to the `CompletionResponse`:

| Field | Description |
|-------|-------------|
| `gate` | `Pass` / `Warn` / `Fail` — coarse signal quality indicator |
| `phase` | Dominant phase label with trust score (e.g. `"reasoning:0.82"`) |
| `trust` | Scalar `[0, 1]` — high = low entropy + high logprob confidence |
| `mean_logprob` | Mean token log-probability over the sampled window |
| `entropy` | Mean positional entropy estimate |

Phases: `Reasoning`, `Retrieval`, `Refusal`, `Creative`, `Hallucination`, `Adversarial`. The classifier uses heuristic vocabulary matching (adversarial n-gram detection for prompt-injection patterns) and entropy thresholds — no external model or network call.

Vitals propagate downstream:

- **`EpisodicNode`** — `vitals_gate`, `vitals_phase`, `vitals_trust` stored alongside episode data.
- **`ainl-persona` signals** — confident reasoning nudges `Systematicity`; hallucination/creative phases nudge `Curiosity`.
- **`ainl-graph-extractor`** tags — `vitals:reasoning:pass` / `vitals:elevated` `SemanticTag`s.
- **AINL frame** — `_vitals_gate`, `_vitals_phase`, `_vitals_trust` keys injected so AINL programs can branch on cognitive state.
- **`TurnHooks::on_vitals_classified`** — host hook for real-time reaction.

Providers that don't return logprobs (Anthropic, Gemini, etc.) produce `vitals: None`; the system is fully fail-open.

## See also (ArmaraOS operator docs)

- **[`docs/graph-memory.md`](../../docs/graph-memory.md)** — SQLite paths, inbox drain, end-of-turn writes, orchestration vs graph stores.
- **[`docs/persona-evolution.md`](../../docs/persona-evolution.md)** — `AINL_PERSONA_EVOLUTION` axis hook vs `run_persona_evolution_pass`.
- **[`docs/graph-memory-sync.md`](../../docs/graph-memory-sync.md)** — short hub linking Python **`AinlMemorySyncWriter`** and this README.

## Optional ainl-runtime turn path (`ainl-runtime-engine`)

Cargo feature **`ainl-runtime-engine`** links workspace **`ainl-runtime`** (with crate feature **`async`**) and installs **`AinlRuntimeBridge`**. When **compiled in**, a chat turn can be handled entirely by **`AinlRuntime::run_turn`** (graph memory + extraction orchestration) **instead of** the default LLM + tool loop, if **either**:

- the agent manifest sets **`ainl_runtime_engine = true`**, or
- the process environment sets **`AINL_RUNTIME_ENGINE=1`**.

Graph memory (`ainl_memory.db`) must open successfully; otherwise the runtime logs a warning and keeps the normal loop. This path does **not** call the LLM, does **not** run approvals, and does **not** emit streaming token events.

**Operator / embedder doc:** **`docs/ainl-runtime-integration.md`** (build, activation, limitations, troubleshooting, roadmap).

**Build:**

```bash
cargo build -p openfang-runtime --features ainl-runtime-engine
```

**Tests:**

```bash
cargo test -p openfang-runtime --features ainl-runtime-engine test_agent_loop_uses_openfang_by_default
# Or: cargo test -p openfang-runtime --features ainl-runtime-engine ainl_runtime
```

**Daemon note:** `openfang-runtime` includes this feature in default builds. Activation still follows runtime switches (`manifest.ainl_runtime_engine` or `AINL_RUNTIME_ENGINE=1`) and falls back to the normal loop when graph memory is unavailable.
