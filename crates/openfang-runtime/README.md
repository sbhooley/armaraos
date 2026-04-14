# openfang-runtime

Agent loop, tool execution, graph memory (`GraphMemoryWriter`), and related runtime services for ArmaraOS / OpenFang.

## Optional semantic tagging (`ainl-tagger`)

The crate can be built with Cargo feature **`ainl-tagger`**, which links **`ainl-semantic-tagger`** and wires [`ainl_semantic_tagger_bridge::SemanticTaggerBridge`] into graph-memory writes (episode + fact nodes).

At **runtime**, tagging is applied only when:

```bash
export AINL_TAGGER_ENABLED=1
```

Without this variable set to `1`, tag lists are left empty even if the binary was built with `ainl-tagger` (keeps default installs cheap and predictable).

Build example (adds `ainl-tagger` alongside the crate’s default features, e.g. `ainl-persona-evolution`):

```bash
cargo build -p openfang-runtime --features ainl-tagger
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

Slim build without the crates.io dependency:

```bash
cargo build -p openfang-runtime --no-default-features --features ainl-persona-evolution
```

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
