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
