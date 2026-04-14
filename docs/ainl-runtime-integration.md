# ainl-runtime integration (OpenFang / ArmaraOS)

This document describes the **optional** embed of [**ainl-runtime**](https://crates.io/crates/ainl-runtime) (`0.3.5-alpha`) inside **`openfang-runtime`**. The ArmaraOS workspace depends on the **path** crate (`crates/ainl-runtime`) with the same version so `ainl-memory`, `ainl-persona`, and `ainl-graph-extractor` resolve to a single dependency graph; using the registry tarball alone would duplicate those crates and fail to compile. The built-in OpenFang agent loop remains the **default**; no behavior changes unless the feature and a switch are enabled.

## Feature flag

- **Cargo feature:** `ainl-runtime-engine` on `openfang-runtime`
- **Dependency:** `ainl-runtime` with the crate’s `async` feature (for `run_turn_async` in the bridge)

Build example:

```bash
cargo build -p openfang-runtime --features ainl-runtime-engine
```

## Activation

Either:

1. **Per-agent manifest** — set the top-level boolean on `AgentManifest`, e.g. in TOML:
   ```toml
   ainl_runtime_engine = true
   ```
   **or** the same field in persisted manifest JSON, **or**
2. **Environment** — `AINL_RUNTIME_ENGINE=1` for the daemon process.

Both require a working AINL graph memory database (`ainl_memory.db`) for the agent; if the graph writer cannot be opened, the loop logs a warning and **falls back** to the normal OpenFang LLM path.

## Runtime behavior

When the switch is on and graph memory is available, `run_agent_loop` / `run_agent_loop_streaming` constructs an [`AinlRuntimeBridge`](crates/openfang-runtime/src/ainl_runtime_bridge.rs), runs a single **ainl-runtime** `run_turn` with the (possibly compressed) user text, maps the result to an assistant reply, saves the session, fires `AgentLoopEnd`, and returns **without** calling the LLM driver.

- **Delegation depth:** `AinlRuntimeBridge::with_delegation_cap` uses `[runtime_limits].max_agent_call_depth` so nested **ainl-runtime** turns align with OpenFang’s agent-call budget.
- **Persona evolution writes:** the bridge builds `AinlRuntime` with `with_evolution_writes_enabled(false)` so OpenFang’s existing post-turn persona / extractor pipeline is not competing on the same evolution row (see **ainl-runtime** crate docs).

## Known limitations

- **No multi-tenancy:** one SQLite store per agent id; the shim does not add tenant isolation beyond the existing graph layout.
- **No approval queue:** tool approvals and kernel policy gates are part of the OpenFang LLM/tool loop; the ainl-runtime path does not surface pending approvals.
- **No LLM:** **ainl-runtime** orchestrates graph memory + extraction hooks only; the assistant text is synthesized from persona / status strings, not from a model completion.
- **Streaming:** the early-return path does not emit token deltas; clients should treat the turn as a single completed reply after session save.
- **Double connection:** the bridge opens a second SQLite handle to the same `ainl_memory.db` as `GraphMemoryWriter` (WAL-safe); avoid holding long-lived incompatible locks across the two APIs.

## EndTurn-shaped telemetry

The bridge maps **ainl-runtime** `TurnOutcome` into a small host struct (`output`, `tool_calls`, `delegation_to`, `cost_estimate`) and logs an **info** line for observability. Fields of `TurnResult` / `MemoryContext` that do not have OpenFang dashboard equivalents yet produce **warnings** (see `map_ainl_turn_outcome` in `ainl_runtime_bridge.rs`).

## Convergence roadmap

1. **Unify evolution writes** — single writer for `EVOLUTION_TRAIT_NAME` with explicit hand-off between OpenFang and **ainl-runtime**, or make **ainl-runtime** read-only on SQLite when embedded.
2. **Tool + approval parity** — optional second phase that runs OpenFang tool dispatch from **ainl-runtime** emit / patch hooks instead of short-circuiting the LLM loop.
3. **Orchestration traces** — push mapped EndTurn fields into the same pipeline as LLM `StopReason::EndTurn` for the dashboard.
4. **Streaming** — incremental assistant text if a future **ainl-runtime** API streams memory summaries.

Until then, treat this path as an **experimental graph-first** turn for agents that do not need live LLM replies.
