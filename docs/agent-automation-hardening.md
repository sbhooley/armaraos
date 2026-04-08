# Agent automation hardening

Guidance for **reliable agent workflows** across browser scraping, file I/O, shell, and long tasks. Complements the loop guard described in [troubleshooting.md](troubleshooting.md) (Agent Issues).

---

## Table of contents

- [Symptom: "Missing path/command parameter"](#symptom-missing-pathcommand-parameter)
- [Why agents restart expensive work](#why-agents-restart-expensive-work)
- [Loop guard vs malformed tools](#loop-guard-vs-malformed-tools)
- [Phase model: acquire, extract, persist, verify](#phase-model-acquire-extract-persist-verify)
- [Persistence patterns that scale](#persistence-patterns-that-scale)
- [Workspace and cross-session habits](#workspace-and-cross-session-habits)
- [Cross-workspace writes: "Access denied"](#cross-workspace-writes-access-denied-path-resolves-outside-workspace)
- [Process management: process_start, process_kill, process_list](#process-management-process_start-process_kill-process_list)
- [Runtime loop protections (v0.6.5)](#runtime-loop-protections-v065)
- [Curated AINL: skill mint stub (reference)](#curated-ainl-skill-mint-stub-reference)
- [Caveats if you implement stricter checks](#caveats-if-you-implement-stricter-checks)

---

## Symptom: "Missing path/command parameter"

Agents may report **`file_write` failed: Missing 'path' parameter** or **`shell_exec` failed: Missing 'command' parameter** when the tool was invoked with an **empty JSON object** `{}` or without required fields.

**v0.6.5+:** The error now lists every missing required field with its type and description so the model can self-correct in one step without retrying the whole task.

This is **not** usually a disk permission or sandbox denial. The runtime expects:

- **`file_write`:** `{"path": "<workspace-relative or allowed path>", "content": "<string>"}`  
- **`shell_exec`:** `{"command": "<non-empty shell string>", ...}`  

Implementation reference: `tool_file_write` and `shell_exec` handling in `crates/openfang-runtime/src/tool_runner.rs`.

**What to do**

1. Inspect the **audit / tool trace**: if `INPUT` is `{}`, fix the **tool call shape**, not the scraper.
2. Retry **only the persist step** with valid parameters (see [Phase model](#phase-model-acquire-extract-persist-verify)).
3. For very large payloads, avoid a single huge `file_write` body; use [Persistence patterns](#persistence-patterns-that-scale) below.

---

## Why agents restart expensive work

A common failure mode:

1. Data is already in the browser (DOM, `localStorage`, or prior tool output).
2. **`file_write` / `shell_exec` fails** (often empty args).
3. The model assumes "nothing was saved" and **re-runs navigation, search, or scroll** from scratch.

That wastes time and hits **loop guard** limits when the same bad tool call repeats.

**Rule of thumb:** **Persistence failure does not imply acquisition failure.** Re-fetch only if **verify** shows the data is actually missing (see phases below).

---

## Loop guard vs malformed tools

OpenFang's **loop guard** (`crates/openfang-runtime/src/loop_guard.rs`) tracks repeated `(tool_name, serialized_params)` and can **warn**, **block**, or **circuit-break** the run. Identical **empty** `{}` calls hash the same way, so you may see:

- Early **warnings** about repeated identical calls  
- **Blocks** after several identical failures  

**v0.6.5+:** After **3 consecutive iterations** where every tool call was blocked, the loop exits early with a diagnostic summary rather than running until the iteration limit.

That protects the system but does **not** replace **correct tool arguments**. See [troubleshooting.md](troubleshooting.md) → *Agent stuck in a loop*.

---

## Phase model: acquire, extract, persist, verify

| Phase | Typical tools | Rule |
|--------|----------------|------|
| **Acquire** | `browser_navigate`, `web_fetch`, API calls | Expensive; aim for **once per session** unless verify fails. |
| **Extract** | `browser_run_js`, parsing, LLM structuring | Turn raw UI/API into structured data in memory or a temp artifact. |
| **Persist** | `file_write`, `shell_exec` (script writes file) | **Retry here** when only saving failed. |
| **Verify** | `file_read`, `file_list`, `shell_exec` (`wc`, `head`, `test -f`) | Confirm bytes on disk before declaring success. |

Encode this in workspace **`AGENTS.md`** / **`TOOLS.md`**: *If persist fails, do not return to Acquire unless Verify shows the artifact or data is missing.*

---

## Persistence patterns that scale

- **Browser download:** For large CSV/JSON, run a short `browser_run_js` snippet that builds a `Blob` and triggers a download (same origin as the data). No giant chat/tool JSON.
- **Shell + heredoc or script:** Write a small file with `python3` / `printf` that reads from stdin or an existing chunk file, then run it with a **full** `command` string.
- **Chunked extraction:** Store chunks in `localStorage` or numbered files, then **concatenate in one persist step** — avoid redoing Acquire for each chunk.

Paths must stay within the agent **workspace** (and tool policy) unless your manifest explicitly allows broader access.

---

## Workspace and cross-session habits

- **`TOOLS.md` / `AGENT.json`:** Document tool-specific rules (e.g. "after lien extract, save via download or `output/liens.csv` only").
- **Artifacts:** Keep canonical outputs under `output/` or `data/` so the next turn can `file_read` instead of re-scraping.
- **Skills:** For repeatable flows (recorder → CSV), use a skill with explicit steps and **stop conditions** ("do not re-open search URL after Extract succeeds").
- **Long-term memory:** Promote stable conventions to workspace `MEMORY.md` or org docs when they apply across projects.

---

## Cross-workspace writes: "Access denied: path resolves outside workspace"

**Cause:** `file_write` or `apply_patch` was called with a path that belongs to a different workspace. Both tools are sandboxed to the agent's own workspace directory.

**What the error says (v0.6.5+):**
> *Access denied: path '...' resolves outside workspace. To write to a file outside your workspace, use `shell_exec` instead (e.g. a Python one-liner or heredoc). To read synced AINL programs, use paths starting with `ainl-library/`...*

**Fix:** Use `shell_exec` for cross-workspace writes:

```json
{
  "command": "python3",
  "args": ["-c", "open('/absolute/path/to/file.py', 'w').write('content here')"]
}
```

The sandbox applies to `file_write`, `file_list`, and `apply_patch`. `shell_exec` is **not** workspace-sandboxed. For reads, `ainl-library/...` and `~/.armaraos/` paths are permitted without `shell_exec`.

---

## Process management: `process_start`, `process_kill`, `process_list`

### Always include `cwd` for bot/server processes

When starting a process that relies on a local `.env` file, relative imports, or `os.getcwd()`, supply the **`cwd`** parameter (added in v0.6.5):

```json
{
  "command": "python3",
  "args": ["bot.py"],
  "cwd": "/Users/you/.armaraos/workspaces/MyBot"
}
```

Omitting `cwd` inherits the daemon's working directory (`~/.armaraos/`), which causes `.env` lookups and relative imports to silently fail.

### Don't claim the process is running until `process_start` is called

A common phantom action: the agent calls `process_list`, sees `[]`, then responds "Starting it now — proc_1 is up" **without** calling `process_start`. The runtime (v0.6.5+) detects this and re-prompts.

**Recommended health-check pattern:**

```
1. process_list               → check if already alive
2. (if empty) process_start   → start with correct command + cwd
3. process_poll {process_id}  → confirm no immediate crash (check stderr)
4. Report status
```

---

## Runtime loop protections (v0.6.5)

Shipped in v0.6.5 (were design notes in earlier versions of this doc):

1. **Tool preflight validation:** Each tool call is validated against its JSON Schema `required` fields before dispatch. Missing fields return a single rich error listing every required field with its type and description.
2. **All-blocked early exit:** After 3 consecutive iterations where every tool call is blocked by the loop guard, the loop exits gracefully with a summary.
3. **Process phantom detection:** If the model's final response claims a process was started or stopped but `process_start`/`process_kill` was not called in that turn, the runtime re-prompts once.
4. **Channel phantom detection (streaming path):** Phantom channel-send claims are now caught in both the standard and streaming agent loops.
5. **`browser_run_js` auto-wrap:** Expressions containing top-level `return` statements are automatically wrapped in an IIFE so `Runtime.evaluate` handles them without a syntax error.

---

## Curated AINL: skill mint stub (reference)

This is **orthogonal** to browser/file workflows but was clarified alongside automation discussions:

| Item | Detail |
|------|--------|
| **Cron name** | `armaraos-skill-mint-stub-monthly` |
| **Source** | `programs/skill-mint-stub/skill_mint_stub.ainl` (mirrored under `ainl-library/armaraos-programs/…` when embedded) |
| **Registration** | `crates/openfang-kernel/src/curated_ainl_cron.json` |
| **Default** | **`enabled: false`** (opt-in from Scheduler UI) |
| **Schedule** | `0 10 2 * *` (10:00 on the 2nd of each month, when enabled) |
| **Frame** | Learning frame v1 with `op: skill_mint` — passed to `ainl run` as `--frame-json` |

The graph builds a **minimal deterministic Markdown body** (`# {intent}` + `## Episode` + episode text). Full SKILL metadata for interactive flows is owned by the host; see `render_skill_draft_markdown` in `crates/openfang-kernel/src/skills_staging.rs`.

Full curated-job tables: [ootb-ainl.md](ootb-ainl.md). Learning frame schema: [learning-frame-v1.md](learning-frame-v1.md).

---

## Caveats if you implement stricter checks

- **Per-tool schemas:** Rejecting `{}` globally can break tools that legitimately use only optional fields; validate **required keys per tool**.
- **Tool name aliases:** Run validation **after** normalizing names (e.g. `fs-write` → `file_write`).
- **Prompt bloat:** Long universal rules in system prompts compete with task instructions; prefer a **short** kernel invariant plus **workspace** / **skill** detail.
- **False recovery:** "Never re-acquire" can be wrong if the session expired or storage was cleared — tie recovery to **verify** outcomes.
- **Logging:** Error paths should avoid leaking full payloads; cap or redact secrets in diagnostics.

---

## See also

- [Troubleshooting — Agent Issues](troubleshooting.md#agent-issues)
- [Out-of-the-box AINL — Curated cron](ootb-ainl.md#curated-cron-jobs)
- [Learning frame v1](learning-frame-v1.md)
- [Agent files and documents](agent-files-and-documents.md) (workspace paths and tools)
