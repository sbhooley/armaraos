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

For **post-turn fact / procedural pattern extraction** in the agent loop, the structured bridge is used only when this env var is set:

```bash
export AINL_EXTRACTOR_ENABLED=1
```

Accepted truthy values: `1`, `true`, `yes`, `on` (case-insensitive). When unset or falsey, the runtime keeps the legacy [`graph_extractor`](src/graph_extractor.rs) heuristics for facts and patterns while persona evolution still runs if the feature remains enabled.

**Persona evolution pass return type:** when **`ainl-extractor`** is enabled, **`GraphMemoryWriter::run_persona_evolution_pass`** returns **`ainl_graph_extractor::ExtractionReport`** (type-alias **`PersonaEvolutionExtractionReport`**). Without **`ainl-extractor`**, the same method returns a small stub report (see **`graph_memory_writer.rs`**). Inspect **`has_errors()`** and the **`extract_error` / `pattern_error` / `persona_error`** fields when present; the implementation logs one **`warn!`** per populated slot (signal merge vs pattern persistence vs persona write) so operators see partial extractor failures without failing the spawned task. **`AinlRuntime::run_turn`** maps the same three fields to distinct **`TurnPhase`** warnings — see **`docs/ainl-runtime.md`** (*Persona evolution pass*).

Slim build without the crates.io dependency:

```bash
cargo build -p openfang-runtime --no-default-features --features ainl-persona-evolution
```

## See also (ArmaraOS operator docs)

- **[`docs/graph-memory.md`](../../docs/graph-memory.md)** — SQLite paths, inbox drain, end-of-turn writes, orchestration vs graph stores.
- **[`docs/persona-evolution.md`](../../docs/persona-evolution.md)** — **`AINL_PERSONA_EVOLUTION`** axis hook vs **`run_persona_evolution_pass`**.
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
cargo test -p openfang-runtime --features ainl-runtime-engine ainl_runtime test_agent_loop_uses_openfang_by_default
```

**Daemon note:** `openfang-kernel` does not enable this feature by default; shipping the shim in production binaries requires forwarding **`features = ["ainl-runtime-engine"]`** on the `openfang-runtime` dependency (or an equivalent workspace feature) in your packaging graph.
