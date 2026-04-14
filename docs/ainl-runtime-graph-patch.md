# ainl-runtime GraphPatch (Rust) — integration notes

This document is the **host-facing bridge** between ArmaraOS’s SQLite graph memory (`ainl_memory.db`, written primarily by **openfang-runtime** today) and the standalone **`ainl-runtime`** crate’s procedural patch dispatch.

## Current architecture (honest)

- **Dashboard / daemon execution** still runs in **`openfang-runtime`**. It does **not** call `AinlRuntime::run_turn` yet.
- **`ainl-runtime`** is a **separate** orchestration crate: `run_turn` loads `MemoryContext`, dispatches **active procedural** rows from `GraphQuery::active_patches`, records episodes, optional extraction, etc.
- **Full Python GraphPatch** (IR promotion, `memory.patch`, compile-time checks in AINL) is **not reimplemented** in Rust. The Rust path is **metadata + small JSON summaries from patch adapters** so a host can decide what to execute.

### Optional: `run_turn_async` (crate feature `async`)

For Tokio embedders, **`ainl-runtime`** can offload SQLite-heavy work with **`AinlRuntime::run_turn_async`** (`features = ["async"]`). Graph memory is guarded by **`std::sync::Mutex`** inside **`Arc`**, not **`tokio::sync::Mutex`**, so **`AinlRuntime::new`** and short borrows like **`sqlite_store()`** remain safe on any thread (including Tokio workers used in **`#[tokio::test]`**); see **`crates/ainl-runtime/README.md`**.

## Where patches come from

`MemoryContext.active_patches` is `Vec<AinlMemoryNode>` where each node is `AinlNodeType::Procedural` with a [`ProceduralNode`](https://github.com/sbhooley/armaraos/blob/main/crates/ainl-memory/src/node.rs) payload: `label` / `pattern_name`, `patch_version`, `declared_reads`, `fitness`, `retired`, `compiled_graph` (`Vec<u8>`), `procedure_type`, etc. The same JSON shape is what Python `ainl_graph_memory` uses for procedural / patch-style rows at a higher layer.

## What to register

1. **`AinlRuntime::register_default_patch_adapters()`** — installs [`GraphPatchAdapter`](https://github.com/sbhooley/armaraos/blob/main/crates/ainl-runtime/src/adapters/graph_patch.rs) under the name `graph_patch`. It is used as a **fallback** when no label-specific [`PatchAdapter`](https://github.com/sbhooley/armaraos/blob/main/crates/ainl-runtime/src/adapters/mod.rs) matches the procedural `label`.
2. **Optional:** `GraphPatchAdapter::with_host(Arc<dyn GraphPatchHostDispatch>)` — your process receives the same JSON **summary** the adapter returns: `{ "label", "patch_version", "frame_keys" }` (after declared-read checks). Use it to forward to Python `ainl run`, another worker, or a no-op logger.

## Memory context / semantic ranking (migration)

`MemoryContext` is built inside `AinlRuntime::run_turn` / `run_turn_async` via `compile_memory_context_for`.

**`compile_memory_context_for(None)` no longer inherits previous episode text for semantic ranking; pass `Some(user_message)` if you want topic-aware ranking.**

- `compile_memory_context()` still calls `compile_memory_context_for(None)` — that path behaves like an **empty** user message: `relevant_semantic` uses the **high-recurrence** fallback, not the latest episode body.
- `run_turn` / `run_turn_async` always pass the **current** turn’s `user_message` into memory compilation, so the default turn pipeline stays topic-aware without extra calls.

See **`crates/ainl-runtime/README.md`** and the **`ainl-runtime`** crate rustdoc (`MemoryContext`) for the same note.

## Future: openfang-runtime

When **openfang-runtime** embeds `AinlRuntime` for a turn, the intended wiring is:

1. Open the same `SqliteGraphStore` / agent id as the dashboard writer (or a read replica — **not** concurrent writers on the same evolution row; see `AinlRuntime` rustdoc on `evolution_writes_enabled`).
2. Call `register_default_patch_adapters()` (and any label-specific `PatchAdapter`s).
3. Consume `TurnOutput.patch_dispatch_results` and/or the host hook envelope to drive tool execution outside the minimal Rust runtime.

Until that wiring lands, treat this path as **library + tests + docs**, not daemon behavior.
