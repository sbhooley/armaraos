# Cursor Agent CLI integration — implementation plan (ArmaraOS)

This document proposes how to add **Cursor Agent CLI** as a first-class LLM backend in ArmaraOS, analogous to the existing **`claude-code`** (`ClaudeCodeDriver`) path. It consolidates **Cursor’s official CLI documentation**, the **Paperclip** reference adapter, and ArmaraOS wiring points.

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
| Global parameters | [cursor.com/docs/cli/reference/parameters](https://cursor.com/docs/cli/reference/parameters) | **`-p` / `--print`**: non-interactive; “has access to all tools, including write and shell.” **`--output-format`**: only with `--print` — `text`, `json`, `stream-json` (default `text`). **`--resume [chatId]`**, **`--continue`**. **`--model`**, **`--workspace`**. Auth: **`--api-key`** or **`CURSOR_API_KEY`**. **`--force`** / **`--yolo`**: “Force allow commands unless explicitly denied”; **`--yolo`** is an alias for **`--force`**. **`--trust`**: “Trust the workspace without prompting (headless mode only).” **`--sandbox enabled|disabled`**. |
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
- **Prompt delivery**: passed via **stdin** to the child process (avoids huge argv / shell escaping). ArmaraOS should validate whether we prefer **stdin** vs **positional prompt** ([Parameters](https://cursor.com/docs/cli/reference/parameters) documents a positional `prompt` argument). Recommendation: **default stdin** for large prompts; optional path to pass a single positional argument if we need to match Cursor examples exactly.
- **Parsing**: custom NDJSON parser (see Paperclip `server/parse.ts`) — extract **`session_id`**, assistant **`result`**, **`usage`**, costs, errors; retry without **`--resume`** if session is unknown.
- **Skills**: inject symlinks under **`~/.cursor/skills`** for Paperclip skills — ArmaraOS may or may not replicate; if we have first-party skills, mirror the same hook.

---

## Current ArmaraOS baseline

- **`claude-code`**: `ClaudeCodeDriver` — subprocess, **`claude -p`**, JSON/stream handling, `skip_permissions`, env hygiene, timeouts, PID tracking (`crates/openfang-runtime/src/drivers/claude_code.rs`).
- **Driver registration**: `create_driver` in `crates/openfang-runtime/src/drivers/mod.rs` branches on `provider == "claude-code"`.
- **Catalog / auth UI**: `ProviderInfo` and **`detect_auth`** special-case CLI providers (`crates/openfang-runtime/src/model_catalog.rs`).
- **Cache key**: `effective_base_url_for_cache` uses `cli:<path>` style for CLI backends.

The Cursor integration should follow the same structural seams.

---

## Proposed design

### 1. Provider ID and model IDs

- **Provider id**: `cursor` (aligns with Paperclip and is short for config).
- **Model id scheme**: `cursor/<cursor-model-id>` where `<cursor-model-id>` matches what **`agent --list-models`** / Cursor docs list (e.g. `auto`, `gpt-5.3-codex`, composer IDs). Exact catalogue can start as a **static list** plus `auto`, refreshed periodically from **`agent models`** / docs.

### 2. New driver crate module

Add **`crates/openfang-runtime/src/drivers/cursor_agent.rs`** (name TBD) implementing `LlmDriver`:

- **Binary resolution**: default `agent` on `PATH`; optional override via `DriverConfig.base_url` (same pattern as `claude-code` reusing `base_url` for CLI path) — or introduce a dedicated optional field later; **minimize schema churn** by reusing existing `base_url`/config patterns used for CLI overrides.
- **Invocation (MVP)**:
  - `agent -p --output-format stream-json --workspace <abs cwd> --model <id>`
  - Append `--resume <session_id>` when resuming (see session persistence below).
  - Append **`--mode ask`** or **`--mode plan`** only when the ArmaraOS request explicitly requests those modes (future: map from agent profile or graph metadata).
  - **Force / trust**: configurable strategy:
    - **Recommended default for autonomous agents**: `--yolo` *or* `--force` (docs: same effect) **or** `--trust`** depending on whether we need workspace trust vs command force — Cursor distinguishes `--trust` (workspace, headless) vs `--force` / `--yolo` (command execution). Paperclip adds `--yolo` when user did not pass trust flags. ArmaraOS should document the matrix and default to the least surprising option for “daemon applies edits.”
- **Stdin**: feed **serialized prompt** (same string construction as other drivers: system + messages → one text blob, or structured per product conventions).
- **Stdout/stderr**: read fully with timeout + PID tracking (reuse Claude Code’s **concurrent stdout/stderr drain** pattern to avoid pipe deadlocks on large output).
- **Timeouts**: reuse `LlmConfig` / per-driver timeout constants like Claude Code.

### 3. Parsing strategy

Two phases:

**Phase A (MVP)** — parse **`stream-json`** NDJSON (Paperclip-compatible):

- Scan lines; for each JSON object, handle **`type`**: `assistant`, `result`, `error`, `system`, `tool_call` (optional for cost estimation), per [Output format](https://cursor.com/docs/cli/reference/output-format).
- Final assistant-visible **`summary`**: concatenate **`assistant`** message text and/or terminal **`result.result`** field (docs show full text in terminal `result`).
- Extract **`session_id`** from **`system`** / **`result`** / line events (Cursor documents `session_id` on multiple event types).
- **Usage / cost**: map **`result`** events’ usage fields if present (Paperclip aggregates `input_tokens`, `output_tokens`, cached fields, optional cost). Cursor field names may vary — implement **lenient** serde with defaults like Paperclip.

**Phase B (optional)** — switch or fallback to **`--output-format json`** for simpler “single blob” parsing when streaming telemetry is not needed (smaller parser, but no incremental events).

**Partial streaming**: optionally support **`--stream-partial-output`** only if the kernel exposes streaming to the UI; Cursor warns about **duplicate assistant events** — must implement [their filtering rules](https://cursor.com/docs/cli/reference/output-format) if we turn this on.

### 4. Session persistence

- Cursor **`--resume [chatId]`** and **`--continue`** are documented in [Parameters](https://cursor.com/docs/cli/reference/parameters).
- ArmaraOS should store **`session_id`** + **`cwd`** (and optionally workspace identity) in the same layer that already stores runtime session params for other CLI tools, only resuming when **`cwd` matches** (Paperclip’s behavior).
- On “unknown session” errors, **retry once without `--resume`** (Paperclip pattern).

### 5. `create_driver` wiring

In **`drivers/mod.rs`**:

- Add branch: `if provider == "cursor" { ... CursorAgentDriver::new(...) }`.
- Extend **`effective_base_url_for_cache`** with a stable key, e.g. `cursor-cli:<override or agent>`.

### 6. Model catalog & detection

In **`model_catalog.rs`**:

- **`ProviderInfo`**: `cursor`, display name **Cursor Agent CLI**, `key_required: false` **if** we only support subscription/local login — *or* set `key_required: true` when we require **`CURSOR_API_KEY`** for unattended operation. Product decision: support both and set **`AuthStatus`** based on `agent status` / presence of `CURSOR_API_KEY` (mirror Paperclip’s `api` vs `subscription` split).
- **`detect_auth`**: probe **`agent --version`** or **`agent status`** (needs spike) to mark Configured vs Missing.
- **Models list**: seed from Paperclip’s public list in `cursor-local/src/index.ts` as a starting point, trim to models we are willing to support; document **`agent models`** as the source of truth for operators.

### 7. Kernel allowlists

Any provider allowlist that currently includes **`claude-code`** should be extended to **`cursor`** where CLI backends are permitted (`openfang-kernel` grep for provider checks).

### 8. Security & environment

- **Strip or isolate secrets** in child env (pattern from `ClaudeCodeDriver`: remove unrelated cloud API keys where appropriate so subprocesses do not accidentally use wrong credentials).
- **Document** that **`--force`/`--yolo`/`--trust`** are **high privilege** and should be gated by ArmaraOS RBAC / capability flags.
- **`HOME`**: ensure set so Cursor can find login state (`~/.cursor/…`) when the service runs without a login shell.

### 9. Configuration surface (proposal)

Minimal additions to agent/runtime config (exact shape follows existing TOML/JSON patterns in repo):

- `cursor_cli_path` (optional; maps to driver path override).
- `cursor_force_mode`: `none` | `yolo` | `force` | `trust` | `yolo_and_trust` (names TBD — must map 1:1 to Cursor flags).
- `cursor_output_format`: `stream-json` | `json` (default `stream-json`).
- `cursor_timeout_sec`: number.

### 10. Testing

- **Unit tests**: NDJSON parser fixtures copied from Cursor doc **Example sequence** in [output-format](https://cursor.com/docs/cli/reference/output-format) and Paperclip tests if any.
- **Integration tests** (optional, CI-gated): skip if `agent` not installed; if present, `agent -p --output-format json "ping"` smoke test.

### 11. Documentation updates

- Extend **`docs/providers.md`** with a **Cursor Agent CLI** section: install link, env vars, model selection, force/trust warning, link to this plan.
- Mention **`CURSOR_API_KEY`** for headless per [headless doc](https://cursor.com/docs/cli/headless).

---

## Milestones

| Milestone | Deliverable |
| --- | --- |
| M1 — Spike | Confirm **`agent`** invocations from a Rust `tokio::process::Command` with **`-p`**, **`--workspace`**, **`--output-format stream-json`**, stdin prompt; verify exit codes and stderr on failure per docs. |
| M2 — Parser | Implement NDJSON parser + unit tests from doc samples. |
| M3 — Driver | `CursorAgentDriver` implements `complete` (+ optional streaming hook later). |
| M4 — Registry | `create_driver`, catalog provider, aliases, `detect_auth`. |
| M5 — Session resume | Persist `session_id` + cwd; resume + unknown-session fallback. |
| M6 — UX & docs | Wizard strings, `providers.md`, operational runbook. |

---

## Risks and open questions

1. **CLI stability**: Cursor may add fields or rename events; parser must be **tolerant** (unknown fields ignored).
2. **Binary name**: Docs use **`agent`**; some systems might use a shim. Spikes should confirm **`which agent`** after official install.
3. **Prompt path**: Positional vs stdin — verify on Windows and Linux CI; document the chosen approach ([Parameters](https://cursor.com/docs/cli/reference/parameters) suggests positional prompt exists).
4. **Billing / subscription**: Usage and “cost” reporting may differ between API key and subscription login; Paperclip infers **biller** from env — ArmaraOS cost accounting should align with existing **`LlmCallMetrics`** without double-counting.
5. **Tool calls in output**: Full `tool_call` stream may be large; for LLM driver **completion** we might only need **`result`** + **`assistant`** summaries unless we expose tooling traces to the UI in a later iteration.

---

## Related files (existing)

- `crates/openfang-runtime/src/drivers/claude_code.rs` — subprocess + JSON handling patterns.
- `crates/openfang-runtime/src/drivers/mod.rs` — `create_driver`, cache keys.
- `crates/openfang-runtime/src/model_catalog.rs` — providers, aliases, `detect_auth`.
- Paperclip: `packages/adapters/cursor-local/src/server/execute.ts`, `server/parse.ts`.

---

## References

- Cursor: [CLI overview](https://cursor.com/docs/cli/overview), [Parameters](https://cursor.com/docs/cli/reference/parameters), [Output format](https://cursor.com/docs/cli/reference/output-format), [Headless CLI](https://cursor.com/docs/cli/headless)
- Paperclip adapter: [github.com/paperclipai/paperclip](https://github.com/paperclipai/paperclip) — `packages/adapters/cursor-local`
- ArmaraOS: existing Claude Code driver and provider plumbing (see paths above).
