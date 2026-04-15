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
