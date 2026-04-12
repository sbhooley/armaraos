# OpenRouter: defaults, resilience, and chat errors

This page summarizes how ArmaraOS integrates **OpenRouter** for new installs, what happens when the **primary** model is **rate limited** or **overloaded**, how **errors** appear in the dashboard, and where to look in code when updating behavior.

For general provider setup (keys, catalog, routing), see **[providers.md](providers.md)**. For operational fixes, see **[troubleshooting.md](troubleshooting.md)**.

---

## Default model (product)

Fresh installs and templates that choose OpenRouter align on a single default **model id** (without the `openrouter/` prefix; drivers normalize it):

| Constant | Location | Purpose |
|----------|----------|---------|
| `DEFAULT_OPENROUTER_MODEL_ID` | `crates/openfang-types/src/config.rs` | Kernel `[default_model]` migration hints, desktop bundled defaults, spawn templates, setup wizard (`wizard.js`), bundled `agents/*/agent.toml` examples |

Current value: **`nvidia/nemotron-3-super-120b-a12b:free`** (OpenRouter `:free` tier). This is what new spawn templates, the setup wizard, bundled example `agents/*/agent.toml`, and the desktop “bundled LLM defaults” path align on (see `DEFAULT_OPENROUTER_MODEL_ID` in `openfang-types`).

Older installs may still have **`stepfun/step-3.5-flash:free`** in `~/.armaraos/agents/.../agent.toml`; OpenRouter may return **404** (“no endpoints”) for deprecated free routes. Update the agent’s **`[model]`** block or use the dashboard model picker.

---

## Built-in OpenRouter free fallbacks (agent loop)

Separate from **`[fallback_providers]`** in `config.toml` and separate from per-manifest **`fallback_models`**, the **agent loop** (`crates/openfang-runtime/src/agent_loop.rs`) can call OpenRouter again after the **primary** driver exhausts **rate limit / overload** retries (`MAX_RETRIES` with backoff).

| Constant | Location | Behavior |
|----------|----------|----------|
| `OPENROUTER_FREE_FALLBACK_MODELS` | `crates/openfang-types/src/config.rs` | Ordered list of OpenRouter **`:free`** model ids. Tried after `call_with_retry` / `stream_with_retry` hit the retry cap on **429-style** throttling or overload. |

Rules:

- Each id is tried **in order** via a short-lived OpenRouter driver (`OPENROUTER_API_KEY` from the environment).
- Any entry **equal to the primary request’s model id is skipped** so we do not immediately re-call the same model when the primary is already Nemotron.
- If all attempts fail, the returned error includes a **short per-model summary** (e.g. rate limited, HTTP body snippet) so audit logs and API responses are easier to read than a generic “rate limited after N retries” alone.

**Shipped list (verify in source before relying in prose):** as of recent releases, `OPENROUTER_FREE_FALLBACK_MODELS` is typically **`nvidia/nemotron-3-super-120b-a12b:free`**, then **`meta-llama/llama-3.1-8b-instruct:free`** (second alternate when the primary is Nemotron). The exact slice lives in `crates/openfang-types/src/config.rs`.

**Example compounded error** (after retries + fallbacks; counts reflect models actually tried, after skipping the primary id):

`Rate limited after 3 retries. OpenRouter free-model fallbacks failed (1): meta-llama/llama-3.1-8b-instruct:free → rate limited (retry after 5000ms)`

The same fallback path runs for **overload** (with “Model overloaded after N retries” as the prefix). Helpers: `summarize_openrouter_fallback_err`, `truncate_llm_detail` in `agent_loop.rs`.

To change the fallback list for a release, edit **`OPENROUTER_FREE_FALLBACK_MODELS`** and keep entries compatible with **tool use** if agents rely on tools. OpenRouter’s free catalog rotates; verify ids on [openrouter.ai/models](https://openrouter.ai/models?q=free) or their free-models collection.

---

## Dashboard chat: friendly vs raw errors

Stream/send failures surface a dismissible **LLM error banner** in chat. The visible sentence is produced by **`humanizeChatError`** in `crates/openfang-api/static/js/pages/chat.js`; the **raw** daemon message is stored for **hover** / debugging (`lastStreamErrorTechnical`).

**Evaluation order (important):** the function checks **rate limits → stream decode → timeouts → billing/credits → “no endpoints” / model access → 401 / invalid key phrasing → 403 / forbidden → …** so that a response containing both `403` and “insufficient credits” is classified as **billing**, not “check API key.”

Rough mapping (simplified):

| Signal in raw error | User-facing intent |
|---------------------|-------------------|
| **429**, rate limit language | Traffic limiting — wait and retry |
| **402**, insufficient credits / billing wording | Billing or credits on the **provider account** — not “wrong API key” by default |
| **No endpoints** / model access phrasing | Model or account access — pick another model or confirm access on OpenRouter |
| **401**, invalid API key phrasing | Check **Settings** and **`OPENROUTER_API_KEY`** |
| **403**, forbidden | Often **not** a bad key (model restriction, account rule); hover for the exact body |

So **HTTP 403 with credits on the account** can still happen (e.g. model not enabled for the key); the banner copy distinguishes **401** (key) from **403** (access) and billing-like strings from both.

**Usage page heuristics:** `crates/openfang-api/static/js/pages/usage.js` labels spend rows as “OpenRouter” when the model string mentions `openrouter`, `nemotron`, or `nvidia/` (so Nemotron usage is grouped correctly).

---

## Related code (for contributors)

| Area | Path |
|------|------|
| Default + fallback constants | `crates/openfang-types/src/config.rs` |
| Retry + OpenRouter fallback calls | `crates/openfang-runtime/src/agent_loop.rs` (`call_with_retry`, `stream_with_retry`, `try_openrouter_free_fallbacks`, `try_openrouter_free_fallbacks_stream`, `summarize_openrouter_fallback_err`, `truncate_llm_detail`) |
| Error classification (Rust) | `crates/openfang-runtime/src/llm_errors.rs` |
| Chat banner copy | `crates/openfang-api/static/js/pages/chat.js` (`humanizeChatError`) |
| Setup wizard defaults | `crates/openfang-api/static/js/pages/wizard.js` (`defaultModelForProvider`) |
| Spawn / Armara template | `crates/openfang-api/static/js/pages/agents.js` |

---

## See also

- **[providers.md](providers.md)** — OpenRouter row, model catalog, manifest `fallback_models`
- **[troubleshooting.md](troubleshooting.md)** — OpenRouter free tier, 401/403 vs credits, legacy StepFun 404
- **[configuration.md](configuration.md)** — `[default_model]`, env vars
- **[dashboard-testing.md](dashboard-testing.md)** — Chat UX manual checks, **LLM error banner** (`humanizeChatError`)
- **[dashboard-setup-wizard.md](dashboard-setup-wizard.md)** — Wizard default models and `wizard.js`
