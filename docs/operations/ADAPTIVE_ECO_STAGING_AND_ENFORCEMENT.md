# Adaptive eco: staging validation and controlled enforcement

This runbook is for **operators** running ArmaraOS with a real daemon and agents. It does not replace unit tests; it validates **shadow telemetry** (`enforce = false`) on **real traffic**, then promotes **`enforce = true`** with tuned limits.

## Prerequisites

- Daemon reachable (default `http://127.0.0.1:4200`).
- At least one agent with normal chat traffic after enabling adaptive eco (SQLite rows accumulate over time).
- Optional: `OPENFANG_API_KEY` exported if your API requires Bearer auth (same as dashboard).

## Phase A — Shadow-only (`enforce = false`)

### 1. Config (`~/.armaraos/config.toml` or your config path)

Enable adaptive eco but **do not** enforce yet:

```toml
[adaptive_eco]
enabled = true
enforce = false
enforce_min_consecutive_turns = 2
allow_aggressive_on_structured = false
semantic_floor = 0.82
circuit_breaker_enabled = true
circuit_breaker_window = 12
circuit_breaker_min_below_floor = 3
min_secs_between_enforced_changes = 0
post_circuit_cooldown_secs = 0
provider_prompt_cache_ttl_secs = 300
cache_ttl_dampens_raises = true
circuit_breaker_extra_window_when_prompt_cache = 6
```

Apply: **`POST /api/config/reload`** after editing, or restart the daemon.

### 2. Generate traffic

Use dashboard chat (or any channel) so agents complete turns with compression + adaptive metadata. Aim for **dozens of turns** across **7d** before judging aggregates (empty DB shows zeros — that is normal).

### 3. Review usage endpoints

Run the helper (see repo `scripts/verify-adaptive-eco-usage.sh`) or curl manually:

```bash
./scripts/verify-adaptive-eco-usage.sh http://127.0.0.1:4200
```

**Interpretation (same window, e.g. `7d`):**

| Endpoint | What to check |
|----------|----------------|
| **`GET /api/usage/adaptive-eco`** | `events` > 0 after traffic. `shadow_mismatch_turns` = recommendation differed from applied (shadow). `circuit_breaker_trips` / `hysteresis_blocks` stay **0** while `enforce = false` (hysteresis enforcement is off; breaker can still trip in-kernel for circuit-adjusted base). |
| **`GET /api/usage/adaptive-eco/replay`** | `shadow_mismatch_rate` not alarming for your tolerance. `effective_mode_flip_rate` and confidence buckets stable — not required to be zero. Semantic percentiles on compression rows should stay in a band you accept. |
| **`GET /api/usage/compression`** | `adaptive_eco.summary` / `adaptive_eco.replay` mirror the dedicated endpoints for that `window`. Confirms one-shot dashboards can use compression alone. |

**Promotion criteria (subjective, but concrete):**

- No unexplained explosion in `shadow_mismatch_rate` vs your pilot expectations.
- Semantic p50/p95 in replay align with quality bar (see chat Eco Diff / org policy).
- Operators comfortable with **`reason_codes`** in `adaptive_eco` metadata on messages (debug).

If anything looks wrong, keep **`enforce = false`**, adjust **`semantic_floor`** / **`circuit_breaker_*`**, or disable **`adaptive_eco.enabled`** until root-caused.

## Phase B — Controlled enforcement (`enforce = true`)

Only after Phase A looks good over a meaningful window (often **several days** of mixed traffic, not one afternoon).

### 1. Flip enforcement

```toml
[adaptive_eco]
enabled = true
enforce = true
```

Reload config. Start with **conservative** rate limits:

```toml
enforce_min_consecutive_turns = 3
min_secs_between_enforced_changes = 120
provider_prompt_cache_ttl_secs = 300
cache_ttl_dampens_raises = true
```

### 2. Tune by provider class

| Knob | Anthropic / OpenAI (cached) | Local / Groq (no prompt cache) |
|------|-----------------------------|----------------------------------|
| **`provider_prompt_cache_ttl_secs`** | 300–600 | Often **60–120** or set **`cache_ttl_dampens_raises = false`** if oscillation is not observed |
| **`circuit_breaker_extra_window_when_prompt_cache`** | 6–12 | **0** (rely on `circuit_breaker_window` only) — set extra to **0** by using a small value only when using routed providers |
| **`enforce_min_consecutive_turns`** | 3–4 | 2–3 if latency-sensitive |
| **`min_secs_between_enforced_changes`** | 120–300 s | 60–120 s |

**`post_circuit_cooldown_secs`:** set **non-zero** (e.g. **300**) if you see aggressive re-escalation right after a breaker trip.

### 3. Watch live

- Dashboard **Settings → Budget** adaptive eco block (same window as compression).
- Chat meta **⚡ eco** / tooltips for policy confidence and counterfactuals.
- **`GET /api/usage/adaptive-eco/replay`** daily: `effective_mode_flip_rate`, breaker trips, hysteresis blocks.

### 4. Roll back

If enforcement misbehaves:

```toml
enforce = false
```

or

```toml
enabled = false
```

Reload — kernel stops changing modes; compression still follows **`efficient_mode`** from manifest/global.

## Automation

- **Harness (no daemon):** `cargo test -p openfang-runtime --test adaptive_eco_eval_harness` — see [ADAPTIVE_ECO_EVAL_HARNESS.md](ADAPTIVE_ECO_EVAL_HARNESS.md).
- **Live JSON smoke:** `scripts/verify-adaptive-eco-usage.sh` — see script header.

## See also

- [prompt-compression-efficient-mode.md](../prompt-compression-efficient-mode.md)
- [api-reference.md](../api-reference.md) — usage routes
- [MILESTONE1_PREFLIGHT_RECOVERY_CHECKLIST.md](../MILESTONE1_PREFLIGHT_RECOVERY_CHECKLIST.md)
