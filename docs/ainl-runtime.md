# ainl-runtime (Rust orchestration crate)

This page is the **ArmaraOS documentation hub** for the workspace crate **`crates/ainl-runtime`**: a standalone orchestration layer over **`ainl-memory`** (SQLite **`GraphMemory`**). It is **not** the Python AINL `RuntimeEngine`, **not** the MCP server, and **not** wired into the live dashboard agent loop today.

---

## What it does

| API | When to use |
|-----|----------------|
| **`AinlRuntime::run_turn`** | Synchronous single-turn pipeline: validate subgraph, compile **`MemoryContext`**, dispatch procedural patches via **`PatchAdapter`**, record episodes, optional **`GraphExtractorTask::run_pass`**, sync **`TurnHooks`**. |
| **`AinlRuntime::run_turn_async`** | Same semantics off the Tokio async executor: enable crate feature **`async`**, run SQLite-heavy work on **`tokio::task::spawn_blocking`**, optional **`TurnHooksAsync`**. |

Both return **`Result<TurnOutcome, AinlRuntimeError>`** ( **`Complete`** vs **`PartialSuccess`** with **`TurnWarning`** + **`TurnPhase`** ). **`RuntimeConfig::max_delegation_depth`** applies to nested **`run_turn`** / **`run_turn_async`** the same way.

**Semantic ranking:** **`compile_memory_context_for(None)`** does not inherit the latest episode body for **`MemoryContext::relevant_semantic`**; pass **`Some(user_message)`** for topic-aware ranking. **`run_turn`** / **`run_turn_async`** pass the current turn text. Details: **[ainl-runtime-graph-patch.md](ainl-runtime-graph-patch.md)** (section *Memory context / semantic ranking*).

---

## Cargo feature `async`

```toml
[dependencies]
ainl-runtime = { version = "…", features = ["async"] }
```

Pulls in **`async-trait`** and **`tokio`** (`rt`, `macros`, `fs`, `sync`). Default features are **empty** so sync-only dependents pay no Tokio cost.

---

## Why graph memory uses `std::sync::Mutex`, not `tokio::sync::Mutex`

With the **`async`** feature, graph state is **`Arc<std::sync::Mutex<ainl_memory::GraphMemory>>`** (see **`crates/ainl-runtime/src/graph_cell.rs`**).

- **`AinlRuntime::new`** and **`sqlite_store()`** may run on **any** OS thread, including a Tokio **worker** running **`#[tokio::test]`**. Holding a **`tokio::sync::Mutex`** across those entry points pushes embedders toward **`blocking_lock`** or “cannot block inside async context” failure modes when the runtime detects blocking work on the executor.
- **Heavy** SQLite reads/writes for **`run_turn_async`** still run inside **`spawn_blocking`** closures, matching how **`openfang-runtime`** isolates blocking **`GraphMemory`** work from async tasks.

Full rationale and host guidance: **`crates/ainl-runtime/README.md`** (section *Optional Tokio API*).

---

## Hooks: sync vs async

| Trait | Used by | Notes |
|-------|---------|--------|
| **`TurnHooks`** | **`run_turn`** | **`Send + Sync`**, synchronous callbacks. |
| **`TurnHooksAsync`** | **`run_turn_async`** | **`#[async_trait]`**; install with **`AinlRuntime::with_hooks_async`**. Sync hooks remain available in parallel. |

---

## Related ArmaraOS docs

| Doc | Topic |
|-----|--------|
| **[ainl-runtime-graph-patch.md](ainl-runtime-graph-patch.md)** | **`PatchAdapter`** registry, **`GraphPatchAdapter`** fallback, **`PatchDispatchResult`** (**`adapter_name`**, **`adapter_output`**), host dispatch summaries; semantic ranking migration; **crates.io** version matrix (**0.3.5-alpha** and friends). |
| **[ainl-runtime-integration.md](ainl-runtime-integration.md)** | Optional **`openfang-runtime`** embed: feature **`ainl-runtime-engine`**, **`AinlRuntimeBridge`**, **`TurnOutcome`** mapping, activation and roadmap. |
| **[graph-memory.md](graph-memory.md)** | Live daemon path: **`GraphMemoryWriter`**, **`ainl_memory.db`**, Python inbox — vs this optional crate. |
| **[architecture.md](architecture.md)** | Workspace crate table; **`ainl-runtime`** row mentions **`async`**. |
| Repo root **[ARCHITECTURE.md](../ARCHITECTURE.md)** | Layer 3 graph substrate; execution engine vs **`ainl-runtime`** roadmap. |

Crate-level detail, **`RuntimeConfig`**, **`AinlRuntimeError`**, and evolution coordination with **`openfang-runtime`**: **`crates/ainl-runtime/README.md`**.

---

## Verification (developers)

From the repo root:

```bash
# Default features (no Tokio in ainl-runtime itself)
cargo test -p ainl-runtime
cargo clippy -p ainl-runtime --all-targets -- -D warnings

# Async API + integration tests (includes tests/test_async_runtime.rs)
cargo test -p ainl-runtime --features async
cargo clippy -p ainl-runtime --all-targets --features async -- -D warnings
```

The **`test_async_runtime`** target is declared in **`crates/ainl-runtime/Cargo.toml`** with **`required-features = ["async"]`** so **`cargo test --workspace`** skips it when the workspace does not enable **`ainl-runtime/async`**. CI that needs those tests should run the **`--features async`** command above for this crate.

Optional OpenFang bridge:

```bash
cargo test -p openfang-runtime --features ainl-runtime-engine ainl_runtime
```

---

## Future: openfang-runtime embedding

When **`openfang-runtime`** embeds **`AinlRuntime`** for a turn, hosts must still respect single-writer rules for persona evolution rows (see **`GraphMemoryWriter::run_persona_evolution_pass`** rustdoc and **`crates/ainl-runtime/README.md`** *Persona evolution and ArmaraOS*). The async path does not change SQLite file locking semantics; it only moves **where** blocking graph work runs relative to the Tokio executor.
