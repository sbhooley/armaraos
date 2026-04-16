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
```

Hot-reload: use **`POST /api/config/set`** with `path: "efficient_mode"` (full contract: [api-reference.md](api-reference.md#post-apiconfigset)) or edit the file and **`POST /api/config/reload`** where applicable.

### Per-agent override

If the agent manifest includes **`metadata.efficient_mode`**, it **wins** over the global value. The kernel injects the global default only when the manifest does not already set `efficient_mode` (`or_insert_with`).

### Dashboard

- **Settings → Budget** — card **Ultra Cost-Efficient Mode** with a dropdown and short guidance on typical savings ranges and dense-technical prompts.
- **Chat (agent open)** — header **⚡ eco** pill cycles **Off → Balanced → Aggressive → Off** (`cycleEcoMode` in `static/js/pages/chat.js`). The **authoritative per-agent map** is stored in **`~/.armaraos/ui-prefs.json`** under **`agent_eco_modes`** (merged into `localStorage` **`armaraos-eco-modes-v1`** on load) so each agent remembers its own mode across navigation and **desktop reinstalls** that wipe WebView storage. The UI still calls **`POST /api/config/set`** with **`path: "efficient_mode"`** so the running kernel applies the mode for the **currently open** agent’s next message. Global default remains **`efficient_mode`** in **`config.toml`** / **`GET /api/config`** for new installs and for agents without an entry in **`agent_eco_modes`**.

### AINL CLI (host signal only)

The AINL repo’s `ainl run --efficient-mode …` sets **`AINL_EFFICIENT_MODE`** in the environment for hosts that read it. **No compression runs in Python** — ArmaraOS performs compression in Rust when the daemon runs the workflow.

---

## API and telemetry

### REST

**`POST /api/agents/{id}/message`** response may include:

- **`compression_savings_pct`** (`u8`, 0–100) — omitted when zero.
- **`compressed_input`** (`string`, optional) — text actually sent to the LLM when savings &gt; 0; powers the **Eco Diff** UI.
- **`tools`** — optional array of tool executions for that blocking turn (same field name as elsewhere; unrelated to compression). See [api-reference.md](api-reference.md) (**POST /api/agents/{id}/message**).

### WebSocket (`/api/agents/{id}/ws`)

Final **`{"type":"response",...}`** may include **`compression_savings_pct`** and **`compressed_input`** when compression ran.

Streaming emits a **`CompressionStats`** event before LLM tokens; the dashboard uses it for the **⚡ eco ↓X%** badge and diff payload.

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

---

## See also

- [configuration.md](configuration.md) — `efficient_mode` top-level field  
- [api-reference.md](api-reference.md) — message response and WebSocket shapes  
- [dashboard-settings-runtime-ui.md](dashboard-settings-runtime-ui.md) — Budget card + chat controls  
- [dashboard-testing.md](dashboard-testing.md) — manual QA checklist  
