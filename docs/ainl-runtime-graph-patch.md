# ainl-runtime GraphPatch (Rust) — integration notes

This document is the **host-facing bridge** between ArmaraOS’s SQLite graph memory (`ainl_memory.db`, written primarily by **openfang-runtime** today) and the standalone **`ainl-runtime`** crate’s procedural patch dispatch.

**Hub doc** (sync vs async API, verification, mutex design): **[ainl-runtime.md](ainl-runtime.md)**.

## Current architecture (honest)

- **Dashboard / daemon execution** still runs in **`openfang-runtime`**. The **default** chat path is the OpenFang LLM loop; an **optional** embed can call `AinlRuntime::run_turn` when Cargo feature **`ainl-runtime-engine`** is enabled **and** the agent sets `ainl_runtime_engine = true` or the process sets `AINL_RUNTIME_ENGINE=1` — see **`docs/ainl-runtime-integration.md`**.
- **`ainl-runtime`** is a **separate** orchestration crate: `run_turn` loads `MemoryContext`, dispatches **active procedural** rows from `GraphQuery::active_patches`, records episodes, optional extraction, etc.
- **Full Python GraphPatch** (IR promotion, `memory.patch`, compile-time checks in AINL) is **not reimplemented** in Rust. The Rust path is **metadata + small JSON summaries from patch adapters** so a host can decide what to execute.

### Session persistence (`RuntimeStateNode`)

**`AinlRuntime`** reads and writes a single **`runtime_state`** graph row per agent (`ainl-memory` **`RuntimeStateNode`**) in the same SQLite file: **`turn_count`**, **`last_extraction_at_turn`**, optional **`persona_snapshot_json`**, **`updated_at`**. Restore happens in **`AinlRuntime::new`**; persist runs at the end of **`run_turn`** / **`run_turn_async`**. SQLite errors are non-fatal and surface as **`TurnOutcome::PartialSuccess`** with **`TurnPhase::RuntimeStatePersist`**. See **`crates/ainl-runtime/README.md`** (*Session persistence*), **[ainl-runtime.md](ainl-runtime.md)**, and **[graph-memory.md](graph-memory.md)**.

### Optional: `run_turn_async` (crate feature `async`)

For Tokio embedders, **`ainl-runtime`** can offload SQLite-heavy work with **`AinlRuntime::run_turn_async`** (`features = ["async"]`). Graph memory is guarded by **`std::sync::Mutex`** inside **`Arc`**, not **`tokio::sync::Mutex`**, so **`AinlRuntime::new`** and short borrows like **`sqlite_store()`** remain safe on any thread (including Tokio workers used in **`#[tokio::test]`**); see **`crates/ainl-runtime/README.md`**.

### Delegation depth and hard errors

Nested **`run_turn`** / **`run_turn_async`** calls share an **internal** depth counter (not **`TurnInput::depth`**, which is metadata only). Past **`RuntimeConfig::max_delegation_depth`** (default **8**), the runtime returns **`Err(AinlRuntimeError::DelegationDepthExceeded { depth, max })`**. String-shaped failures use **`AinlRuntimeError::Message`**; use **`message_str()`**, **`is_delegation_depth_exceeded()`**, and **`delegation_depth_exceeded()`** for partial matching. **`TurnStatus`** no longer carries a depth-limit variant — depth is a **hard** error, not a completed turn with a status flag.

Hub overview and verification: **[ainl-runtime.md](ainl-runtime.md)**; crate README: **`crates/ainl-runtime/README.md`**; tests: **`cargo test -p ainl-runtime --test test_delegation_depth`**.

## Where patches come from

`MemoryContext.active_patches` is `Vec<AinlMemoryNode>` where each node is `AinlNodeType::Procedural` with a [`ProceduralNode`](https://github.com/sbhooley/armaraos/blob/main/crates/ainl-memory/src/node.rs) payload: `label` / `pattern_name`, `patch_version`, `declared_reads`, `fitness`, `retired`, `compiled_graph` (`Vec<u8>`), `procedure_type`, etc. The same JSON shape is what Python `ainl_graph_memory` uses for procedural / patch-style rows at a higher layer.

## PatchAdapter registry (label dispatch)

- Implement [`PatchAdapter`](https://github.com/sbhooley/armaraos/blob/main/crates/ainl-runtime/src/adapters/mod.rs) (`name`, `execute_patch`) and register with **`AinlRuntime::register_adapter`**.
- Each active procedural row is dispatched by **procedural `label`**: lookup **`adapter_registry.get(label)`**, else fallback to the built-in **`graph_patch`** adapter when **`register_default_patch_adapters()`** has been called.
- **`PatchDispatchResult`** records optional **`adapter_output`** (`serde_json::Value`) when **`execute_patch`** returns **`Ok`**; **`adapter_name`** is set when an adapter was selected (including on **`Err`**, after which output stays **`None`**). **`Err`** from **`execute_patch`** is logged and the turn continues as metadata-only dispatch (fitness update still applies when the node persists).

## crates.io dependency alignment

When consuming **`ainl-runtime`** from the registry (not path deps), Cargo resolves the **exact** versions declared in that release’s `Cargo.toml`. For **`ainl-runtime` 0.3.5-alpha**, use compatible releases:

| Crate | Minimum / matching version on crates.io |
|-------|----------------------------------------|
| **ainl-memory** | **0.1.8-alpha** |
| **ainl-persona** | **0.1.4** (0.1.3 caps `ainl-memory` at ^0.1.7-alpha and conflicts with graph-extractor + memory 0.1.8) |
| **ainl-graph-extractor** | **0.1.5** |
| **ainl-semantic-tagger** | **0.1.5** (match the release’s `Cargo.toml`; workspace may be ahead of older registry pins) |

Workspace **path** dependencies sidestep this; **`cargo publish -p ainl-runtime`** validates the tarball against the index.

### Pre-release versions and `cargo publish`

When **`ainl-memory`** moves to a **newer pre-release** (for example **0.1.5-alpha** after **0.1.3-alpha**), a dependency written as **`ainl-memory = "^0.1.3-alpha"`** in a crate **already on crates.io** can force the resolver to pick **only** **0.1.3-alpha** for that subtree. That **does not unify** with another edge that requires **`^0.1.5-alpha`** (or **0.1.8-alpha**), so **`cargo publish -p ainl-runtime`** fails with “failed to select a version for `ainl-memory`”.

**Fix:** publish **new semver versions** of intermediate crates (**`ainl-persona`**, **`ainl-graph-extractor`**) whose published `Cargo.toml` declares the **same** **`ainl-memory`** floor the stack needs, **then** publish **`ainl-runtime`**. Republishing the same persona/extractor version number is not possible on crates.io; bump the crate version.

Operational checklist: **`scripts/publish-prep-ainl-crates.sh`** (ordered dry-runs) and the table above after each release.

## What to register

1. **`AinlRuntime::register_default_patch_adapters()`** — installs [`GraphPatchAdapter`](https://github.com/sbhooley/armaraos/blob/main/crates/ainl-runtime/src/adapters/graph_patch.rs) under the name `graph_patch`. It is used as a **fallback** when no label-specific [`PatchAdapter`](https://github.com/sbhooley/armaraos/blob/main/crates/ainl-runtime/src/adapters/mod.rs) matches the procedural `label`.
2. **Optional:** `GraphPatchAdapter::with_host(Arc<dyn GraphPatchHostDispatch>)` — your process receives the same JSON **summary** the adapter returns: `{ "label", "patch_version", "frame_keys" }` (after declared-read checks). Use it to forward to Python `ainl run`, another worker, or a no-op logger.

## Memory context / semantic ranking (migration)

`MemoryContext` is built inside `AinlRuntime::run_turn` / `run_turn_async` via `compile_memory_context_for`.

**`compile_memory_context_for(None)` no longer inherits previous episode text for semantic ranking; pass `Some(user_message)` if you want topic-aware ranking.**

- `compile_memory_context()` still calls `compile_memory_context_for(None)` — that path behaves like an **empty** user message: `relevant_semantic` uses the **high-recurrence** fallback, not the latest episode body.
- `run_turn` / `run_turn_async` always pass the **current** turn’s `user_message` into memory compilation, so the default turn pipeline stays topic-aware without extra calls.

See **`crates/ainl-runtime/README.md`** and the **`ainl-runtime`** crate rustdoc (`MemoryContext`) for the same note.

## openfang-runtime embed (shipped)

**`AinlRuntimeBridge`** (feature **`ainl-runtime-engine`**) implements the first three bullets in a **thin** form:

1. Opens a **second** `SqliteGraphStore` connection to the same `ainl_memory.db` path as `GraphMemoryWriter` (see **`docs/ainl-runtime-integration.md`**).
2. Registers **`GraphPatchAdapter::with_host`** for logging (label / patch_version / frame_keys summaries).
3. Maps **`TurnOutcome`** to host logging and an assistant reply string; **full** tool execution from patch results is still **not** wired.

Further work (tool runner from patch dispatch, streaming, dashboard parity) is tracked in **`docs/ainl-runtime-integration.md`** under **Convergence roadmap**.
