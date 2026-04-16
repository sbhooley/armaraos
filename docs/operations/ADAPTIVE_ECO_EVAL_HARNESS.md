# Adaptive eco evaluation harness

Formal **replay-style** checks for Milestone 2 adaptive eco (shadow resolver + semantic circuit breaker) without starting the daemon or calling LLM APIs.

## What it validates

- Representative **user traces** (conversational, structured JSON/code-heavy, technical opcode-like) go through **`resolve_adaptive_eco_turn`** with a fixed manifest and **`ModelCatalog`** pricing.
- **Structured-payload guard**: with **`allow_aggressive_on_structured = false`**, aggressive manifests still get **`balanced`** recommendations when the message matches the structured heuristic.
- **Circuit breaker**: synthetic semantic score vectors exercise **`circuit_breaker_adjust_base`** (trip vs hold) for regression detection when tuning **`semantic_floor`** / window sizes.

## How to run

From the `armaraos` repo root:

```bash
cargo test -p openfang-runtime adaptive_eco_eval -- --nocapture
```

Implementation: `crates/openfang-runtime/tests/adaptive_eco_eval_harness.rs`.

## Relation to production

| Harness (tests) | Production |
|-----------------|------------|
| Resolver + breaker only | Full kernel also applies **hysteresis**, **`min_secs_between_enforced_changes`**, and **prompt-cache TTL dampening** (see `prompt-compression-efficient-mode.md` and `KernelConfig.adaptive_eco`). |
| No SQLite | Durable rows in **`adaptive_eco_events`** / **`eco_compression_events`** and **`GET /api/usage/...`** aggregates |

Before enabling **`adaptive_eco.enforce`**, use:

1. This harness (green).
2. Shadow telemetry (`enforce = false`) + **`GET /api/usage/adaptive-eco/replay`** in staging.
3. Criteria in **`docs/prompt-compression-efficient-mode.md`** (rollout section).

## See also

- [prompt-compression-efficient-mode.md](../prompt-compression-efficient-mode.md)
- [MILESTONE1_PREFLIGHT_RECOVERY_CHECKLIST.md](../MILESTONE1_PREFLIGHT_RECOVERY_CHECKLIST.md)
- [api-reference.md](../api-reference.md) — usage endpoints
