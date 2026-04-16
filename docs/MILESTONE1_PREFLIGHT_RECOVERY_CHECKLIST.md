# Milestone 1 pre-flight recovery checklist (formal)

This document is the **formal deliverable** for the “strict recovery pass” before Milestone 2: it records whether Milestone 1 durable compression telemetry and surfaces are **present**, **missing**, or **inconsistent** in the current `armaraos` tree.

**Verification date:** 2026-04-16 (repo state: `main` with Milestone 2 adaptive-eco work merged).  
**Method:** code inspection of canonical paths + existing automated tests (`openfang-memory` usage tests, `openfang-api` integration tests).

| Area | Expected (Milestone 1) | Status | Notes |
|------|-------------------------|--------|--------|
| **SQLite: `eco_compression_events`** | Table + indexes; rows per compression turn with mode, tokens, savings %, optional semantic score | **Present** | Created in migration; `UsageStore::record_compression` inserts rows. |
| **SQLite rollups / queries** | `query_compression_summary` aggregates by mode and agent; p50/p95 savings; semantic p50/p95/mean | **Present** | `crates/openfang-memory/src/usage.rs` |
| **Kernel metering → durable writes** | Post-turn persistence of compression rows | **Present** | Kernel metering path records via usage store (see metering + kernel integration). |
| **API: `GET /api/usage/compression`** | JSON with `window`, `modes`, `agents`, estimated token/cost savings, cache-read rollup | **Present** | `routes::usage_compression` |
| **API: compression + adaptive (Milestone 2 extension)** | Same endpoint may embed `adaptive_eco: { summary, replay }` for the same `window` | **Present** | Bundled in `CompressionSummary.adaptive_eco` when queries succeed; see `CompressionAdaptiveEcoBundle`. |
| **Dashboard: Budget → Ultra Cost-Efficient** | Mode dropdown, compression table, window selector, semantic/savings trends | **Present** | `static/index_body.html` Budget tab |
| **Semantic p50 / p95 in API** | Exposed under each mode in `modes` map | **Present** | `CompressionModeSummary` |
| **Receipts / diff payload (chat)** | Savings %, optional compressed text for Eco Diff | **Present** | Chat + WS/API fields (see `prompt-compression-efficient-mode.md`) |

## Drift-prone files (audit)

| File | Role | Status |
|------|------|--------|
| `openfang-runtime/src/llm_driver.rs` | Compression / driver path | **Reviewed** — in tree; no regression noted in checklist pass |
| `openfang-api/src/ws.rs` | Stream events | **Reviewed** |
| `openfang-api/src/types.rs` | Message shapes | **Reviewed** |
| `openfang-api/src/routes.rs` | HTTP compression + adaptive usage routes | **Reviewed** |
| `openfang-runtime/src/agent_loop.rs` | Apply `efficient_mode` before LLM | **Reviewed** |

## Inconsistencies / caveats

- **Single source of truth:** Adaptive aggregates appear on **`GET /api/usage/adaptive-eco`**, **`GET /api/usage/adaptive-eco/replay`**, and (when enabled) inside **`GET /api/usage/compression`** under `adaptive_eco`. Clients may use either dedicated endpoints or the bundled object; numbers should match for the same `window` parameter.
- **Pre-flight vs live DB:** Empty databases return zeros and empty mode maps; this is **consistent**, not missing.

## Sign-off criteria

- [x] Durable schema + query path verified in code.
- [x] API response shape for compression documented (`docs/api-reference.md`) and includes optional `adaptive_eco` bundle.
- [x] Automated tests cover usage store + adaptive replay extensions (`openfang-memory` tests; `api_integration_test` for usage routes).
- [x] Milestone 2 gaps closed: **prompt-cache TTL dampening** + extra circuit window (`kernel.rs` + `[adaptive_eco]` fields) and **`cargo test -p openfang-runtime --test adaptive_eco_eval_harness`** (see `docs/operations/ADAPTIVE_ECO_EVAL_HARNESS.md`).

For rollout, treat this checklist as **complete** for Milestone 1 integrity at the time above; re-run the same table after any refactor of `usage.rs`, `routes.rs` usage handlers, or compression migrations.
