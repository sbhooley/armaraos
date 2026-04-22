# Ultra Cost-Efficient Mode (prompt compression)

ArmaraOS compresses **user input** before each LLM call using the standalone `ainl-compression` crate (`crates/ainl-compression`). Compression is **transparent**: users type normally; only the text sent to the model is shortened. Vector memory and session history can still use the original wording where the pipeline preserves it.

**Latency:** Target **under 30 ms** end-to-end on typical hardware (often much less for short prompts).  
**Token estimate:** `chars / 4 + 1` per segment (same heuristic as elsewhere in telemetry).

For a shorter product overview and benchmarks, see the root [README.md](../README.md#ultra-cost-efficient-mode). This document is the **operator and integrator reference**.

---

## Modes (`efficient_mode`)

| Value | Retention (approx.) | Typical input savings\* | Role |
|-------|----------------------|---------------------------|------|
| `off` | 100 % | 0 % | No compression. |
| `balanced` | ~55 % of tokens | ~40–56 % | Default; strong safety on technical content. |
| `aggressive` | ~35 % of tokens | ~55–74 % on conversational text | Higher savings; smaller gap vs Balanced on **dense technical** prompts (many opcodes/URLs) because more lines are “hard-locked”. |

\*Actual savings depend on length, structure, and how many sentences match **hard** vs **soft** preserve rules (see below).

**Passthrough:** Prompts under **80 estimated tokens** skip compression (no benefit after overhead).

---

## How compression works (summary)

1. **Code fences** (` ``` ` … ` ``` `) are extracted and re-inserted verbatim.
2. Prose is split into sentences (`. ` and newlines); very short blocks (≤2 sentences) are left as-is.
3. Each sentence is scored; high-scoring sentences are packed into a **budget** = `original_tokens × retain(mode)`, with a **floor** of `original_tokens / 4` so short prompts do not collapse two different modes to the same budget.
4. **Hard preserve:** sentences containing any **hard** substring are always kept (see list below).
5. **Soft preserve (Balanced only):** sentences containing **soft** substrings are also forced kept. In **Aggressive**, soft matches only **boost** the score — so changelog-style text with many product names/units can compress much more than in Balanced.
6. **Aggressive extras:** sentences starting with `This `, `These `, `It `, or `Which ` get a score penalty (meta / trailing explanations).
7. **Filler stripping** removes common hedging phrases (`I think `, `Basically, `, `To be honest, `, mid-sentence ` basically `, etc.).
8. First character may be re-capitalized after stripping.
9. If the result is not shorter than the original, the **original** is returned (no-op).

Debug logging (optional):  
`RUST_LOG=ainl_compression=debug` logs full before/after text per call.

---

## Preserve lists (conceptual)

**Hard (both modes):** user-intent and diagnostics — `exact`, `steps`, `already tried` / `restarted` / `checked`, `restart`, `daemon`, `error`, URLs (`http://`, `https://`), `R http`, `R web`, AINL-like tokens (`L_`, `->`, `::`, `.ainl`, `opcode`, `R queue`, `R llm`, `R core`, `R solana`, `R postgres`, `R redis`), and fenced code markers (`` ``` ``).

**Soft (Balanced only; score boost in Aggressive):** `##`, measurement suffixes (` ms`, ` kb`, ` mb`, ` gb`, ` %`), identifiers `openfang`, `armaraos`, `manifest`.

The exact lists live in source; tune there for your deployment vocabulary.

---

## Configuration

### Global (`config.toml`)

```toml
# balanced (default) | aggressive | off
efficient_mode = "balanced"   # default is "off"; set here to enable

# Optional — adaptive eco (shadow by default; uses model catalog + durable semantic scores)
[adaptive_eco]
enabled = false
enforce = false                      # when true, hysteresis + recommendations can change efficient_mode
enforce_min_consecutive_turns = 2    # streak before applying a new mode under enforce
allow_aggressive_on_structured = false
semantic_floor = 0.82
circuit_breaker_enabled = true
circuit_breaker_window = 12
circuit_breaker_min_below_floor = 3
# Optional rate limits (0 = disabled)
min_secs_between_enforced_changes = 0
# After a circuit-breaker step-down, block raising compression tier above the trip floor (seconds; 0 = off)
post_circuit_cooldown_secs = 0
# Prompt-cache TTL awareness (reduces oscillation with Anthropic/OpenAI-style caching)
provider_prompt_cache_ttl_secs = 300
cache_ttl_dampens_raises = true
circuit_breaker_extra_window_when_prompt_cache = 6
```

**Rollout:** enable `adaptive_eco.enabled` first with `enforce = false` to populate `adaptive_eco` metadata and `GET /api/usage/adaptive-eco`. **Staging:** follow [operations/ADAPTIVE_ECO_STAGING_AND_ENFORCEMENT.md](operations/ADAPTIVE_ECO_STAGING_AND_ENFORCEMENT.md) and run **`scripts/verify-adaptive-eco-usage.sh`** against your daemon. When satisfied, set `enforce = true` and tune `enforce_min_consecutive_turns`, **`min_secs_between_enforced_changes`**, circuit breaker fields, and (if needed) **`provider_prompt_cache_ttl_secs`** / **`cache_ttl_dampens_raises`**. Run **`cargo test -p openfang-runtime --test adaptive_eco_eval_harness`** before broad enforcement; see [operations/ADAPTIVE_ECO_EVAL_HARNESS.md](operations/ADAPTIVE_ECO_EVAL_HARNESS.md). API reference: [api-reference.md](api-reference.md#get-apiusageadaptive-eco).

Hot-reload: use **`POST /api/config/set`** with `path: "efficient_mode"` (full contract: [api-reference.md](api-reference.md#post-apiconfigset)) or edit the file and **`POST /api/config/reload`** where applicable. For **`[adaptive_eco]`** keys, **`POST /api/config/set`** with a two-segment path (e.g. **`adaptive_eco.enabled`**) writes **`config.toml`** and, after a successful in-process **`reload_config()`**, updates the **live** adaptive policy used by the kernel and reflected in **`GET /api/status`** → **`adaptive_eco`** (no separate “stale `config` struct” for this section).

### Per-agent override

If the agent manifest includes **`metadata.efficient_mode`**, it **wins** over the global value. The kernel injects the global default only when the manifest does not already set `efficient_mode` (`or_insert_with`).

### Dashboard

- **Settings → Budget** — card **Ultra Cost-Efficient Mode** with a dropdown, compression telemetry (window **7d / 30d / all**), and an **Adaptive eco policy** block that loads **`GET /api/usage/adaptive-eco`** and **`/replay`** for the same window.
- **Chat (agent open)** — header **⚡ eco** pill cycles **Off → Balanced → Aggressive → Off** (`cycleEcoMode` in `static/js/pages/chat.js`). The **authoritative per-agent map** is stored in **`~/.armaraos/ui-prefs.json`** under **`agent_eco_modes`** (merged into `localStorage` **`armaraos-eco-modes-v1`** on load) so each agent remembers its own mode across navigation and **desktop reinstalls** that wipe WebView storage. The UI still calls **`POST /api/config/set`** with **`path: "efficient_mode"`** so the running kernel applies the mode for the **currently open** agent’s next message. Global default remains **`efficient_mode`** in **`config.toml`** / **`GET /api/config`** for new installs and for agents without an entry in **`agent_eco_modes`**.

### AINL CLI (host signal only)

The AINL repo’s `ainl run --efficient-mode …` sets **`AINL_EFFICIENT_MODE`** in the environment for hosts that read it. **No compression runs in Python** — ArmaraOS performs compression in Rust when the daemon runs the workflow.

---

## API and telemetry

### Dashboard “savings” vs measured usage

**Ultra Cost-Efficient Mode** and **`GET /api/usage/compression`** report **aggregated estimates** (original vs compressed token heuristics, cache-read economics, semantic rollups). Those are **not** a duplicate meter from the cloud API proving “exact tokens you would have paid without compression.”

**Measured** token and dollar usage for **completed** calls live in the persistent **`usage_events`** data surfaced by **`GET /api/usage/summary`**.

**Quota- and budget-blocked turns** (turn never started) are tracked separately — see **`quota_enforcement`** on **`GET /api/usage/summary`**. **`compression_savings`** on the same endpoint rolls up all-time (unbounded) compression and cache “saved” metrics, with **model catalog** input $/1M **persisted on each** `eco_compression_events` row so cumulative USD tracks pricing at the time of each turn. As of **schema v15**, every compression event also persists **`provider`**, **`billed_input_tokens`** (provider-reported input after compression), and **`billed_input_cost_usd`** (catalog-priced billable USD). The summary therefore exposes:

- `original_input_tokens_total` — sum of pre-compression input tokens across persisted compression turns (audit baseline).
- `billed_input_tokens_total` / `billed_input_cost_usd_total` — sum of provider-reported input tokens and catalog-priced cost actually billed for those rows.
- `by_provider_model[]` — per-(provider, model) rollup with `original_input_tokens`, `compressed_input_tokens`, `billed_input_tokens`, `input_tokens_saved`, weighted `input_price_per_million_usd`, and `est_input_cost_saved_usd` / `billed_input_cost_usd`.

**Fallback-model attribution (v15+):** `provider` and `model` always reflect the model that **actually** serviced the turn — not the manifest's requested model. When a primary model returns `RateLimited` / `Overloaded` / `ModelNotFound`, `openfang-runtime` retries on a manifest fallback or an OpenRouter free-tier model and exposes the actually-used identity via `AgentLoopResult.actual_provider` / `actual_model`. The kernel then snapshots catalog pricing against that model when persisting both `usage_events` and `eco_compression_events`, so cost rollups and `by_provider_model[]` show the correct entity (e.g. `openrouter / stepfun/step-3.5-flash:free`) instead of misattributing the spend to the requested provider.

This lets dashboards show **true** pre-compression vs billed input per provider/model — not only the pre-LLM heuristic — and survives daemon restarts. **`GET /api/status` → `eco_compression`** remains a 7d slice for other diagnostics. See [api-reference.md](api-reference.md#get-apiusagesummary). Blocked-turn lines are still **heuristic** counterfactuals, not provider-invoiced.

**Historical-data backfill (v15+ kernels, runs once at boot):** Compression rows persisted before pricing was snapshotted on each row landed in `eco_compression_events` with `provider = ''`, `input_price_per_million_usd = 0`, `est_input_cost_saved_usd = 0`, and `billed_input_tokens = 0`. The dashboard's "USD NOT SPENT (EST.)" therefore underreported by orders of magnitude on installs that pre-date v15. On every kernel boot, immediately after the model catalog loads, the runtime calls two idempotent repair passes against `usage_events` + `eco_compression_events`:

- **`UsageStore::backfill_compression_pricing`** — for every row missing pricing, looks up the row's `model` in the current catalog and writes back `input_price_per_million_usd`, `est_input_cost_saved_usd = (input_tokens_saved / 1e6) * price`, `billed_input_cost_usd = (billed_input_tokens / 1e6) * price`, and the catalog's `provider`. Rows that already carry a non-zero price snapshot are **never re-priced** (catalog drift over time is preserved).
- **`UsageStore::backfill_compression_billed_tokens`** — for every row whose `billed_input_tokens` is still zero, joins to the closest `usage_events` row for the same `agent_id` within ±60 s of the compression event (the kernel writes both within the same turn handler, so the join is exact) and copies `input_tokens` into `billed_input_tokens`, then recomputes `billed_input_cost_usd`.

Both passes are **idempotent** — their `WHERE` clauses are the marker — so subsequent boots are no-ops once the corpus is repaired. Boot logs report the repair count: `Backfilled compression pricing on N historical eco_compression_events row(s)`. The next `GET /api/usage/summary` will reflect the recovered totals on `compression_savings.{estimated_compression_cost_saved_usd, billed_input_tokens_total, billed_input_cost_usd_total, by_provider_model[]}`.

For product copy on the Get started page layout and measured vs estimated columns, see [dashboard-overview-ui.md](dashboard-overview-ui.md#measured-vs-estimated-savings).

### REST

**`POST /api/agents/{id}/message`** response may include:

- **`compression_savings_pct`** (`u8`, 0–100) — omitted when zero.
- **`compressed_input`** (`string`, optional) — text actually sent to the LLM when savings &gt; 0; powers the **Eco Diff** UI.
- **`adaptive_confidence`** (`f32`, optional) — policy confidence when **`[adaptive_eco]`** produced metadata for the turn.
- **`eco_counterfactual`** (object, optional) — counterfactual token estimates (applied vs baselines / recommendation).
- **`adaptive_eco_effective_mode`**, **`adaptive_eco_recommended_mode`** (string, optional) — modes after kernel policy vs resolver recommendation (omitted when unset).
- **`adaptive_eco_reason_codes`** (string array, optional) — machine-readable policy reasons for the turn (omitted when unset).
- **`tools`** — optional array of tool executions for that blocking turn (same field name as elsewhere; unrelated to compression). See [api-reference.md](api-reference.md) (**POST /api/agents/{id}/message**).

### WebSocket (`/api/agents/{id}/ws`)

Final **`{"type":"response",...}`** may include **`compression_savings_pct`** and **`compressed_input`** when compression ran.

Streaming emits a **`CompressionStats`** event before LLM tokens; the dashboard uses it for the **⚡ eco ↓X%** badge and diff payload. When **`[adaptive_eco]`** is enabled, the same event (and the final **`response`**) may also include **`adaptive_confidence`** (0.0–1.0), **`eco_counterfactual`**, and optional **`adaptive_eco_effective_mode` / `adaptive_eco_recommended_mode` / `adaptive_eco_reason_codes`**. Chat appends a short **`conf N%`** / **`Δrec … tok`** suffix to the token line and exposes a **tooltip** with JSON for debugging.

**Aggregates (dashboards / audits):**

- **`GET /api/usage/compression`** — durable compression rollups; may embed **`adaptive_eco: { summary, replay }`** for the same **`?window=`** so adaptive outcomes are available without extra requests (see [api-reference.md](api-reference.md#get-apiusagecompression)). If the compression rollup query fails, the response is still JSON-shaped with zeros/empties, may set **`compression_summary_error: true`**, and can still fill **`adaptive_eco`** from the dedicated adaptive queries when **`adaptive_eco_filled_from_fallback`** is true.
- **`GET /api/usage/adaptive-eco`** — counts shadow mismatches, circuit-breaker trips, hysteresis blocks (optional **`?window=7d`** or **`all`**).
- **`GET /api/usage/adaptive-eco/replay`** — same window parameter; **shadow mismatch rate**, **eco compression turn count**, **semantic p50 / p95 / mean** on durable `eco_compression_events`, plus **effective mode flip** rate and **adaptive confidence** p50/p95/mean and bucket counts.

Formal Milestone 1 recovery checklist (pre–Milestone 2): [MILESTONE1_PREFLIGHT_RECOVERY_CHECKLIST.md](MILESTONE1_PREFLIGHT_RECOVERY_CHECKLIST.md).

### Multi-provider prompt caching (context)

See **[prompt-caching-multi-provider.md](prompt-caching-multi-provider.md)** — how provider prompt-cache billing relates to ArmaraOS **input** compression and **`cache_capability`**.

### Logs

Structured **`prompt:compressed`** (and streaming variant) at **INFO**: original/compressed token estimates, savings percentage, optional estimated USD at list input pricing (model-specific billing still applies).

---

## Eco Diff modal

When savings are non-zero, chat can show **⚡ eco ↓X% — diff**. Opening it compares **original user text** vs **compressed prompt** side-by-side. Copy in the modal explains that compression reduces API cost while preserving critical details.

---

## AINL companion (`efficient_styles.ainl`)

In the **AI_Native Lang** repo, **`modules/efficient_styles.ainl`** offers optional **output** styling nodes (`human_dense_response`, `terse_structured`) to keep **responses** dense. That is separate from **input** compression in ArmaraOS; use both for end-to-end cost reduction.

See the AINL repo: **`docs/operations/EFFICIENT_MODE_ARMARAOS_BRIDGE.md`** (output-style module + CLI env bridge).

---

## Tests

```bash
cargo test -p openfang-runtime -- prompt_compressor
```

Includes gap tests between Balanced and Aggressive on mixed prose and regression tests for dashboards, HTTP/AINL prompts, and preserve markers.

**Adaptive eco policy (resolver + kernel):**

```bash
cargo test -p openfang-runtime --test adaptive_eco_eval_harness
cargo test -p openfang-kernel --lib post_circuit
cargo test -p openfang-kernel --lib test_apply_adaptive_eco_skipped_when_disabled
```

**HTTP / API integration (usage routes, `GET /api/status` adaptive block, `POST /api/config/set` for `adaptive_eco.enabled`, Budget HTML markers):**

```bash
cargo test -p openfang-api --test api_integration_test test_usage_
cargo test -p openfang-api --test api_integration_test adaptive_eco_reflects
cargo test -p openfang-api --test api_integration_test config_set_persists_adaptive
cargo test -p openfang-api --test api_integration_test dashboard_html_includes
```

**SQLite telemetry (counterfactual JSON + replay summaries):**

```bash
cargo test -p openfang-memory --lib test_adaptive_eco
```

**HTTP `MessageResponse` JSON (adaptive explainability fields):**

```bash
cargo test -p openfang-api --lib message_response_serializes
```

---

## See also

- [configuration.md](configuration.md) — `efficient_mode` top-level field  
- [api-reference.md](api-reference.md) — message response and WebSocket shapes  
- [dashboard-settings-runtime-ui.md](dashboard-settings-runtime-ui.md) — Budget card + chat controls  
- [dashboard-testing.md](dashboard-testing.md) — manual QA checklist  
