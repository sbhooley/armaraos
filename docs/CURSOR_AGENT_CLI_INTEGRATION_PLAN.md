# Cursor Agent CLI integration — implementation plan (ArmaraOS)

This document proposes how to add **Cursor Agent CLI** as a first-class LLM backend in ArmaraOS, analogous to the existing **`claude-code`** (`ClaudeCodeDriver`) path. It consolidates **Cursor’s official CLI documentation**, the **Paperclip** reference adapter, and **current** ArmaraOS wiring points (paths and types verified against the tree at the time of writing).

---

## Codebase alignment (verified)

Use this as a checklist so the implementation matches the repo **today**.

| Area | Location | What to mirror / extend |
| --- | --- | --- |
| Runtime crate root | `crates/openfang-runtime/src/lib.rs` | `pub mod drivers;`, `pub mod model_catalog;`, `pub mod llm_driver;` |
| Driver trait + config | `crates/openfang-runtime/src/llm_driver.rs` | `LlmDriver`, `CompletionRequest` / `CompletionResponse`, **`DriverConfig`** (`provider`, `api_key`, `base_url`, `skip_permissions`, `http_client`, `model_hint`). Today `skip_permissions` is documented only for Claude Code; Cursor will either **reuse** it as a generic “non-interactive trust/force” flag or add a dedicated field later. |
| Driver factory | `crates/openfang-runtime/src/drivers/mod.rs` | **`create_driver`**, `provider_defaults`, **`effective_base_url_for_cache`**, **`known_providers()`** (static list used in tests — **`test_known_providers_list` currently expects exactly 38 providers**; adding `cursor` must bump that assertion to **39**). |
| Claude Code CLI | `crates/openfang-runtime/src/drivers/claude_code.rs` | Subprocess template: **positional prompt**, **`stdin(Stdio::null())`**, concurrent stdout/stderr drain, **PID map**, **`message_timeout_secs`** (default 300s — **not** `LlmConfig.client_timeout_ms`, which is for the HTTP client inside `LlmDriverFactory`). |
| Qwen Code CLI | `crates/openfang-runtime/src/drivers/qwen_code.rs` | Second subprocess driver; **`skip_permissions` → `--yolo`** on `qwen`; **`base_url`** = optional CLI path override (same overload pattern as `claude-code`). |
| Model catalog | `crates/openfang-runtime/src/model_catalog.rs` | **`builtin_providers()`**, **`detect_auth()`** (special cases for `claude-code` / `qwen-code` today), **`builtin_aliases()`**, model entries, tests at bottom of file. |
| Kernel | `crates/openfang-kernel/src/kernel.rs` | Builds `ModelCatalog`, **`detect_auth`**, **`load_custom_models`**, etc. **`infer_provider_from_model`** (~L9607): prefix match arm must include **`"cursor"`** once `cursor/...` model IDs exist (today lists `claude-code`, `codex`, `copilot`, …). |
| HTTP API / dashboard | `crates/openfang-api/src/routes.rs` (~L10602) | Providers with **`!key_required`** and no probe get **`is_local: true`** (“Local provider with empty base_url (e.g. claude-code) — skip probing”). **`cursor`** should follow the same **`ProviderInfo`** shape as other CLI locals. |
| CLI onboarding | `crates/openfang-cli/src/tui/screens/wizard.rs`, `init_wizard.rs` | Provider picker rows + **`claude_code::claude_code_available()`**-style detection hooks for a future **`cursor_agent_available()`**. |
| Global LLM HTTP config | `crates/openfang-types/src/config.rs` | **`LlmConfig`**: `client_timeout_ms`, `connect_timeout_ms`, `driver_isolation`, `max_cached_drivers` — applies to **HTTP** drivers via `LlmDriverFactory`; **CLI subprocess timeouts stay inside the driver** (see `ClaudeCodeDriver`). |
| Kernel / agent defaults | `crates/openfang-types/src/config.rs` | **`KernelConfig`**, **`DefaultModelConfig`**, **`fallback_providers`**, **`provider_urls`** — any new top-level knobs should follow existing serde + `config.toml` patterns here, not ad-hoc globals. |

---

## Goals

1. Allow agents and the kernel to select a **Cursor-backed model** (e.g. `cursor/auto`, `cursor/gpt-5.3-codex`) that executes via the **`agent`** binary in non-interactive (**print**) mode.
2. Parse **structured output** (`stream-json` or `json`) for `session_id`, final text, usage, and errors — aligned with Cursor’s documented event shapes.
3. Support **optional session resume** across turns (store `session_id` when the runtime supports it, matching Cursor’s `--resume` semantics).
4. Remain safe for daemon use: **non-interactive trust / force** behavior documented and configurable (see [Headless CLI](https://cursor.com/docs/cli/headless)).

**Non-goals (initial slice):**

- Re-implementing Cursor’s tool loop inside Rust (the CLI owns tools).
- Cloud Agent handoff (`&` in interactive sessions per [CLI overview](https://cursor.com/docs/cli/overview)) — out of scope unless we add a separate orchestration feature later.

---

## Official Cursor CLI documentation (applicable notes)

Primary sources (current Cursor docs site):

| Topic | URL | Notes for integration |
| --- | --- | --- |
| CLI overview, install, interactive vs print | [cursor.com/docs/cli/overview](https://cursor.com/docs/cli/overview) | Install via `curl https://cursor.com/install -fsS \| bash` (macOS/Linux/WSL). Entry binary for coding agent is **`agent`**. Modes: Agent (default), **Plan** (`--mode plan`), **Ask** (`--mode ask`). |
| Global parameters | [cursor.com/docs/cli/reference/parameters](https://cursor.com/docs/cli/reference/parameters) | **`-p` / `--print`**: non-interactive; “has access to all tools, including write and shell.” **`--output-format`**: only with `--print` — `text`, `json`, `stream-json` (default `text`). **`--resume [chatId]`**, **`--continue`**. **`--model`**, **`--workspace`**. Auth: **`--api-key`** or **`CURSOR_API_KEY`**. **`--force`** / **`--yolo`**: “Force allow commands unless explicitly denied”; **`--yolo`** is an alias for **`--force`**. **`--trust`**: “Trust the workspace without prompting (headless mode only).” **`--sandbox enabled|disabled`**. Model listing: subcommand **`agent models`** (not `--list-models`). |
| Output formats | [cursor.com/docs/cli/reference/output-format](https://cursor.com/docs/cli/reference/output-format) | **`json`**: single JSON object on success; on failure, stderr, non-zero exit, may omit JSON. **`stream-json`**: NDJSON; terminal **`result`** event on success; events include **`system`**, **`user`**, **`assistant`**, **`tool_call`**, **`result`**. Optional **`--stream-partial-output`** with `stream-json` for delta streaming; docs describe duplicate/skippable `assistant` events when partial streaming is on. **Thinking events are suppressed in print mode.** |
| Headless / automation | [cursor.com/docs/cli/headless](https://cursor.com/docs/cli/headless) | **Without `--force`**, “changes are only proposed, not applied.” For real file edits in scripts: `agent -p --force "..."`. Examples use positional prompt after `-p`. |

**Requirements implied by docs:**

- For ArmaraOS “do work in a workspace” runs, we almost certainly need **`--force`** or **`--trust`** (and/or **`--yolo`**) in combination with **`-p`**, matching Paperclip’s auto-`--yolo` unless the user overrides — but we must **expose this explicitly** in config because it weakens interactive confirmations (Cursor documents this under headless behavior).
- **`--output-format`** is only valid with **`--print`** (or when print mode is inferred: non-TTY stdout or piped stdin, per output-format doc). Our integration should **`-p`** explicitly to stay deterministic in served environments.
- **Authentication**: support **`CURSOR_API_KEY`** / **`--api-key`** so CI and headless hosts can run without interactive `agent login` when the product allows it.

---

## Reference implementation: Paperclip `cursor-local`

[paperclipai/paperclip](https://github.com/paperclipai/paperclip) implements adapter type **`cursor`** in `packages/adapters/cursor-local/`. Useful patterns for our plan:

- **Command**: configurable, default **`agent`** (not `cursor` — the Agent CLI binary name).
- **Arguments**: `agent -p --output-format stream-json --workspace <cwd> [--resume <id>] [--model …] [--mode plan|ask] [--yolo] …`
- **Prompt delivery**: Paperclip passes the prompt via **stdin**. **ArmaraOS today** passes the Claude Code prompt as a **positional argv** and **`stdin(Stdio::null())`** (`claude_code.rs`). For Cursor, **either** approach is valid; choose one after a short spike (positional matches Cursor docs examples; stdin matches Paperclip and avoids argv size limits).
- **Parsing**: custom NDJSON parser (see Paperclip `server/parse.ts`) — extract **`session_id`**, assistant **`result`**, **`usage`**, costs, errors; retry without **`--resume`** if session is unknown.
- **Skills**: inject symlinks under **`~/.cursor/skills`** for Paperclip skills — ArmaraOS may or may not replicate; if we have first-party skills, mirror the same hook.

---

## Current ArmaraOS baseline (accurate)

- **`claude-code`**: `ClaudeCodeDriver` — subprocess **`claude`**, **`-p`**, **`--output-format json`** (non-streaming path in `complete`), **`--dangerously-skip-permissions`** when `skip_permissions`, env stripping, **PID tracking**, **tokio timeout + kill**, **concurrent stdout/stderr drain** (`claude_code.rs`).
- **`qwen-code`**: `QwenCodeDriver` — subprocess **`qwen`**, **`-p`**, **`json` / `stream-json`**, **`--yolo`** when `skip_permissions`, **`base_url`** = optional binary override (`qwen_code.rs`).
- **Driver registration**: `create_driver` in `drivers/mod.rs` branches on `provider == "claude-code"` and `provider == "qwen-code"`.
- **Catalog / auth UI**: `ProviderInfo` + **`detect_auth`** in `model_catalog.rs` special-case **`claude-code`** and **`qwen-code`** (CLI probe, no API key).
- **Cache key**: `effective_base_url_for_cache` uses **`cli:`** prefix for `claude-code` and **`qwen-cli:`** for `qwen-code` (add **`cursor-cli:`** or similar for `cursor`).
- **Instrumented factory**: `LlmDriverFactory::get_driver` wraps drivers with `InstrumentingLlmDriver`; CLI drivers still go through **`create_driver`** the same way.

The Cursor integration should follow the same structural seams as **`claude-code`** (timeouts + drain + PID) rather than the lighter **`qwen`** `output().await` path, unless we explicitly accept blocking risk on large streams.

---

## Proposed design

### 1. Provider ID and model IDs

- **Provider id**: `cursor` (aligns with Paperclip and is short for config).
- **Model id scheme**: `cursor/<cursor-model-id>` where `<cursor-model-id>` matches **`agent models`** / Cursor docs (e.g. `auto`, composer / codex ids). Seed a static catalog; operators can also use **`load_custom_models`** paths already supported by the kernel.

### 2. New driver module

Add **`crates/openfang-runtime/src/drivers/cursor_agent.rs`** (name TBD) implementing `LlmDriver`:

- **Binary resolution**: default **`agent`** on `PATH`; optional override via **`DriverConfig.base_url`** (same overload as `claude-code` / `qwen-code`: empty `None` → default binary name).
- **Invocation (MVP)**:
  - `agent -p --output-format stream-json --workspace <abs cwd> --model <id>`
  - Append `--resume <session_id>` when resuming (see session persistence below).
  - Append **`--mode ask`** or **`--mode plan`** only when the request explicitly requests those modes (future: map from agent profile).
  - **Force / trust**: configurable strategy; map carefully to Cursor flags (**`--force`/`--yolo`** vs **`--trust`** per [Parameters](https://cursor.com/docs/cli/reference/parameters)). Do **not** blindly overload `skip_permissions` without documenting semantics for Cursor vs Claude vs Qwen.
- **Prompt**: positional vs stdin — **spike** (see Paperclip vs `claude_code.rs` above).
- **Stdout/stderr**: **Claude-style** concurrent drain + **process wait timeout** + optional PID map.
- **Timeouts**: **driver-internal** seconds (like `ClaudeCodeDriver`); do not assume `LlmConfig.client_timeout_ms` applies to subprocess lifetime.

**`drivers/mod.rs`:** `pub mod cursor_agent;`, new **`create_driver`** branch, **`provider_defaults`** / cache URL branch, append **`"cursor"`** to **`known_providers()`** and **update `test_known_providers_list` expected length**.

### 3. Parsing strategy

Two phases:

**Phase A (MVP)** — parse **`stream-json`** NDJSON (Paperclip-compatible):

- Scan lines; handle **`type`**: `assistant`, `result`, `error`, `system`, `tool_call` per [Output format](https://cursor.com/docs/cli/reference/output-format).
- Final text: **`assistant`** segments and/or terminal **`result.result`**.
- **`session_id`** from **`system`** / **`result`** / other events as documented.
- **Usage / cost**: lenient parsing (field names may vary by CLI version).

**Phase B (optional)** — **`--output-format json`** for single-blob completion.

**Partial streaming**: optional **`--stream-partial-output`** only if we implement Cursor’s duplicate-filter rules in the doc.

### 4. Session persistence

- Cursor **`--resume [chatId]`** / **`--continue`** per [Parameters](https://cursor.com/docs/cli/reference/parameters).
- Store **`session_id` + cwd** in the same persistence layer used for other multi-turn runtimes (investigate where `claude-code` / agent runtime stores session today — may be agent-scoped state outside the LLM driver; document during M5).
- On unknown session, **retry once without `--resume`** (Paperclip pattern).

### 5. `create_driver` wiring

Already covered: **`drivers/mod.rs`** + **`effective_base_url_for_cache`** stable key (e.g. `cursor-cli:<resolved binary>`).

### 6. Model catalog & detection

In **`model_catalog.rs`**:

- **`ProviderInfo`**: `cursor`, display name **Cursor Agent CLI**, `key_required` / **`detect_auth`** logic: e.g. **`agent --version`** or **`agent status`** when implemented; treat **`CURSOR_API_KEY`** as optional “API billing” path (Paperclip-style split).
- **Aliases**: e.g. `cursor` → `cursor/auto` if product agrees.
- **Tests**: follow **`test_claude_code_*`** / **`test_qwen_code_*`** blocks in the same file.

### 7. Kernel, API, CLI

- **`infer_provider_from_model`** in **`kernel.rs`**: add **`"cursor"`** to the delimited-prefix match list alongside **`claude-code`**.
- **`routes.rs`**: no change needed if **`key_required: false`** mirrors other CLI locals; update the inline comment to mention **`cursor`** when added.
- **`wizard.rs` / `init_wizard.rs`**: add provider row + availability probe for **`agent`**.

### 8. Security & environment

- **Env stripping**: extend the same allowlist/denylist pattern as `claude_code.rs` / `qwen_code.rs` (add **`CURSOR_API_KEY`** handling policy — likely **keep** for headless, strip unrelated provider keys).
- **Document** high-privilege flags (**`--force`/`--yolo`/`--trust`**).
- **`HOME`**: set like `claude_code.rs` so `~/.cursor/` auth state resolves when the daemon has no login shell.

### 9. Configuration surface (proposal)

- Prefer **minimal** first step: **`DriverConfig`** + catalog only.
- Optional later: typed fields on **`KernelConfig`** / agent defaults in **`openfang-types/src/config.rs`** (`cursor_cli_path`, force/trust mode enum, output format, subprocess timeout seconds) — follow serde patterns used by existing LLM-related sections.

### 10. Testing

- **Unit tests**: NDJSON fixtures from Cursor’s **Example sequence** in [output-format](https://cursor.com/docs/cli/reference/output-format).
- **Integration** (optional): `agent -p --output-format json "ping"` when `agent` exists on CI runner.

### 11. Documentation updates

- Extend **`docs/providers.md`** with **Cursor Agent CLI** (install, env, force/trust warning, link here).
- Mention **`CURSOR_API_KEY`** per [headless doc](https://cursor.com/docs/cli/headless).

---

## Milestones

| Milestone | Deliverable |
| --- | --- |
| M1 — Spike | `tokio::process::Command`: **`agent -p`**, **`--workspace`**, **`--output-format stream-json`**, prompt via chosen stdin/argv; verify exit codes / stderr on failure. |
| M2 — Parser | NDJSON parser + unit tests from doc samples. |
| M3 — Driver | `CursorAgentDriver` + `complete` (+ optional `stream` later). |
| M4 — Registry | `create_driver`, **`known_providers` + test len**, catalog, **`detect_auth`**. |
| M5 — Session resume | Persist ids + cwd; resume + unknown-session fallback (wire into existing session store if any). |
| M6 — UX & docs | Wizards, **`infer_provider_from_model`**, `providers.md`, runbook. |

---

## Risks and open questions

1. **CLI stability**: tolerate unknown JSON fields.
2. **Binary name**: **`agent`** on PATH after official install; confirm in spike.
3. **Prompt path**: positional vs stdin — argv limits vs Paperclip parity ([Parameters](https://cursor.com/docs/cli/reference/parameters)).
4. **Billing / subscription**: align usage with existing **`InstrumentingLlmDriver`** / metrics.
5. **Tool call volume**: `stream-json` logs can be large; consider summarizing for storage.

---

## Related files (existing)

- `crates/openfang-runtime/src/drivers/claude_code.rs` — subprocess + timeout + drain + PID pattern (**gold reference**).
- `crates/openfang-runtime/src/drivers/qwen_code.rs` — second CLI driver, **`--yolo`**, argv builder tests.
- `crates/openfang-runtime/src/drivers/mod.rs` — **`create_driver`**, **`known_providers`**, cache keys.
- `crates/openfang-runtime/src/llm_driver.rs` — **`DriverConfig`**, **`LlmDriver`**.
- `crates/openfang-runtime/src/model_catalog.rs` — providers, aliases, **`detect_auth`**, tests.
- `crates/openfang-kernel/src/kernel.rs` — catalog init, **`infer_provider_from_model`**.
- `crates/openfang-api/src/routes.rs` — provider list / local probe behavior.
- `crates/openfang-cli/src/tui/screens/wizard.rs`, `init_wizard.rs` — onboarding provider list.
- `crates/openfang-types/src/config.rs` — **`LlmConfig`**, **`KernelConfig`**, defaults.
- Paperclip: `packages/adapters/cursor-local/src/server/execute.ts`, `server/parse.ts`.

---

## References

- Cursor: [CLI overview](https://cursor.com/docs/cli/overview), [Parameters](https://cursor.com/docs/cli/reference/parameters), [Output format](https://cursor.com/docs/cli/reference/output-format), [Headless CLI](https://cursor.com/docs/cli/headless)
- Paperclip adapter: [github.com/paperclipai/paperclip](https://github.com/paperclipai/paperclip) — `packages/adapters/cursor-local`
- ArmaraOS: paths listed in **Codebase alignment** above.
