# Agent tools: files and documents

Built-in tools for reading workspace files and extracting text from common office formats. Definitions live in `openfang-runtime` (`builtin_tool_definitions`, `execute_tool`).

## Path rules

- Paths are **relative to the agent workspace** unless you use the virtual prefix **`ainl-library/...`**, which resolves into `~/.armaraos/ainl-library/` (synced upstream tree + embedded programs). The same resolution applies to **`file_read`**, **`file_list`**, and **`document_extract`** (see `resolve_file_path_read` in the runtime).

## `file_read`

Plain **UTF-8 text** files. For PDFs, spreadsheets, and Word documents, the implementation returns a short hint telling the model to use **`document_extract`** instead.

## `file_list`

Lists a directory under the workspace or under `ainl-library/`.

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

## MCP and external clients

When OpenFang/ArmaraOS exposes tools over **MCP** (`POST /mcp`), the same names and schemas appear in `tools/list`. Agent manifests must **grant** `document_extract` in `[capabilities].tools` where templates include it (e.g. coding / research agents).

## Related

- [MCP & A2A](mcp-a2a.md) — protocol wiring  
- [Scheduled AINL](scheduled-ainl.md) — cron runs and host adapter policy  
- [OOTB AINL](ootb-ainl.md) — `ainl-library/` layout  
