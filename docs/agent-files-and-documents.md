# Agent tools: files and documents

Built-in tools for reading workspace files and extracting text from common office formats. Definitions live in `openfang-runtime` (`builtin_tool_definitions`, `execute_tool`).

## Path rules

- Paths are **relative to the agent workspace** unless you use the virtual prefix **`ainl-library/...`**, which resolves into `~/.armaraos/ainl-library/` (synced upstream tree + embedded programs). The same resolution applies to **`file_read`**, **`file_list`**, and **`document_extract`** (see `resolve_file_path_read` in the runtime).

## `file_read`

Plain **UTF-8 text** files. For PDFs, spreadsheets, and Word documents, the implementation returns a short hint telling the model to use **`document_extract`** instead.

## `file_list`

Lists a directory under the workspace or under `ainl-library/`.

## `file_write`

Writes **UTF-8 text** to a path under the workspace. Tool JSON must include both **`path`** and **`content`** (non-null strings). Empty or `{}` calls fail — **v0.6.5+** returns a rich error listing every missing required field with its type and description. See [agent-automation-hardening.md](agent-automation-hardening.md) for recovery and when **not** to redo expensive acquisition steps after a failed write.

Paths are sandboxed to the agent workspace. To write to an absolute path outside the workspace, use `shell_exec` with a `python3 -c` one-liner. See [Cross-workspace writes](agent-automation-hardening.md#cross-workspace-writes-access-denied-path-resolves-outside-workspace).

## `apply_patch`

Applies a multi-hunk diff patch to add, update, move, or delete files. All file paths within the patch are resolved through the same workspace sandbox as `file_write` — cross-workspace paths will be denied with the same "Access denied: path resolves outside workspace" error and the same `shell_exec` workaround applies.

## `document_extract`

Extracts human-readable content for model context:

| Format | Behavior |
|--------|----------|
| **PDF** | Text extraction via in-memory parse. |
| **DOCX** | Body text from `word/document.xml`. |
| **Spreadsheets** (`.xlsx`, `.xls`, `.xlsb`, `.ods`) | Tab-separated rows per sheet; optional limits on sheets, rows, and columns. |

**Limits (defaults / caps):** file size, total output characters, and per-sheet dimensions are capped in `document_tools.rs` (see `MAX_DOC_BYTES`, `MAX_OUTPUT_CHARS`, and the `max_*` tool arguments). Spreadsheet cells reflect **cached values**; original Excel formulas may not appear in the extract.

**Tool arguments (JSON):**

- `path` (required) — workspace-relative or `ainl-library/...`
- `max_sheets` — optional, default 8, cap 20  
- `max_rows_per_sheet` — optional, default 400, cap 2000  
- `max_cols` — optional, default 40, cap 100  

## Process management tools

Four tools manage long-running background processes (bots, servers, REPLs). All require `process_start` / `process_kill` / `process_poll` / `process_write` / `process_list` in `[capabilities].tools`.

### `process_start`

Starts a process and returns a `process_id`. Required field: `command`. Optional: `args` (array), `cwd` (string).

**`cwd` is important** for any script that loads a local `.env`, uses relative imports, or depends on `os.getcwd()`. Without it, the process inherits the daemon's working directory (`~/.armaraos/`).

```json
{
  "command": "python3",
  "args": ["bot.py"],
  "cwd": "/Users/you/.armaraos/workspaces/MyBot"
}
```

Max 5 processes per agent. `cwd` was added in **v0.6.5**.

### `process_poll`

Non-blocking drain of stdout/stderr buffered since the last poll. Use after `process_start` to detect immediate crashes.

### `process_write`

Writes a string to the process's stdin (newline appended automatically if absent).

### `process_kill`

Terminates a process and cleans up. Required: `process_id`.

### `process_list`

Returns all alive processes for the calling agent. **Do not report the process as running unless `process_start` was actually called** — the runtime (v0.6.5+) detects phantom claims and re-prompts.

Recommended health-check pattern:

```
1. process_list               → check if alive
2. (if empty) process_start   → start with command + args + cwd
3. process_poll {process_id}  → confirm no immediate crash
4. Report status
```

---

## MCP and external clients

When ArmaraOS/ArmaraOS exposes tools over **MCP** (`POST /mcp`), the same names and schemas appear in `tools/list`. Agent manifests must **grant** `document_extract` in `[capabilities].tools` where templates include it (e.g. coding / research agents).

## Related

- [Agent automation hardening](agent-automation-hardening.md) — valid `file_write` / `shell_exec` args, cross-workspace writes, process management, loop protections  
- [MCP & A2A](mcp-a2a.md) — protocol wiring  
- [Scheduled AINL](scheduled-ainl.md) — cron runs and host adapter policy  
- [OOTB AINL](ootb-ainl.md) — `ainl-library/` layout  
