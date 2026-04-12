# Dashboard: Setup Wizard (`#wizard`)

The **Setup Wizard** is the first-run flow in the embedded dashboard. It walks through LLM provider setup, creating a first agent, a short “try it” chat, optional messaging channels, and a completion summary. Hash route: **`#wizard`**.

## Source files

| Piece | Location |
|-------|----------|
| Markup (steps 1–6, progress bar) | `crates/openfang-api/static/index_body.html` — search `page === 'wizard'` |
| Logic (navigation, providers, agent TOML, channels, finish) | `crates/openfang-api/static/js/pages/wizard.js` — `wizardPage()` |

**Static embedding:** Dashboard JavaScript (including `wizard.js`) is compiled into the API crate via `include_str!` in `crates/openfang-api/src/webchat.rs`. After editing `static/js/pages/wizard.js` or related HTML, **rebuild** the daemon (`cargo build --release -p openfang-cli` or your usual workflow) so the browser loads the updated bundle.

## Step overview

| Step | Label | Purpose |
|------|--------|---------|
| 1 | Welcome | Intro, optional desktop product-analytics consent (Tauri) |
| 2 | Provider | Pick provider, save API key if needed, run connection test |
| 3 | Agent | Choose a template, set name, **Create Agent** |
| 4 | Try It | Mini chat against the created agent |
| 5 | Channel | Optional Telegram / Discord / Slack token |
| 6 | Done | Summary; **Start Chatting** / **Go to Dashboard**; sets onboarding flag |

On finish, the wizard sets `localStorage` **`openfang-onboarded`** to **`true`** and may create a sample scheduled job when an agent was created. Interaction with **Get started** (`#overview`) and **Run setup again** is documented in [dashboard-overview-ui.md](dashboard-overview-ui.md).

## Step 2: Provider readiness

The **Next** button on the provider step is enabled when the selected provider is considered **ready**:

- **Claude Code:** CLI detected or already configured.
- **Providers without an API-key env:** configured in the kernel.
- **Providers with an API key:** after a connection test completes, if the provider is already **configured** (key present), the user may proceed even when the test reports a failure (e.g. free-tier model quota, transient API errors). The UI still shows the test result; a failed test uses a warning style with short guidance.

New keys must pass **Save & verify** (or a successful **Test connection**) before advancing.

## Step 3: Agent creation and manifest TOML

**Create Agent** sends `POST /api/agents` with a `manifest_toml` string.

The TOML must match the **`AgentManifest`** shape: **`name`**, **`description`**, **`profile`**, and **`[model]`** at the **root** of the document — the same layout as `~/.armaraos/agents/<id>/agent.toml`. Do **not** wrap these fields under an `[agent]` table; that nests them where the deserializer does not populate `AgentManifest.name`, which leads to incorrect or default agent naming.

**`profile`** must be a valid **`ToolProfile`** variant: `minimal`, `coding`, `research`, `messaging`, `automation`, `full`, or `custom`. Wizard templates map each archetype to one of these (for example general assistant → `automation`, coding templates → `coding`).

Default models per provider in the wizard (e.g. OpenRouter’s bundled free default, Anthropic’s default id) are defined in `wizard.js` (`defaultModelForProvider`). The OpenRouter default id and rate-limit fallback list are also documented in **[openrouter.md](openrouter.md)** (`DEFAULT_OPENROUTER_MODEL_ID`, `OPENROUTER_FREE_FALLBACK_MODELS` in `openfang-types`). Template cards show the **effective** provider and default model when a provider is already configured.

## Step 6: Summary

If the user already had a provider configured before opening the wizard, the **Done** step pre-fills the provider line in the summary from the loaded provider list so it does not only show a generic “pre-configured” label when a display name is available.

## Related docs

- [Dashboard Get started UI](dashboard-overview-ui.md) — checklist, `openfang-onboarded`, **Run setup again**, `navigateOverview()`
- [Dashboard testing](dashboard-testing.md) — manual QA for the wizard flow
- [API reference — POST /api/agents](api-reference.md) — search the doc for `POST /api/agents`; manifest JSON/TOML contract

## Runtime note (streaming tool batching)

The streaming agent loop merges parallel tool results in LLM order; the implementation uses an explicit iterator type where needed so the Rust compiler can infer `ToolCall` references (`openfang-runtime` `agent_loop.rs`). No dashboard change; relevant when hacking the streaming path.
