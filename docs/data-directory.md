# ArmaraOS data directory

Configuration and local state default to **`~/.armaraos/`** (on Windows, `~` is your user profile).

| Path | Purpose |
|------|---------|
| `~/.armaraos/config.toml` | Main configuration file (see **Config schema version** below) |
| `~/.armaraos/data/openfang.db` | SQLite database (filename is historical) — kernel memory, sessions, task board, audit, etc. |
| `~/.armaraos/agents/<agent_id>/ainl_memory.db` | Optional **per-agent graph memory** SQLite file (**`ainl-memory`** / **`GraphMemoryWriter`**). Created when the agent loop first opens graph memory (separate from **`data/openfang.db`**). Holds typed **episode**, **semantic**, **procedural**, and **persona** nodes, plus an optional **`runtime_state`** row when **`ainl-runtime`**’s **`AinlRuntime`** uses the same file (persisted **`turn_count`**, extraction cadence, persona snapshot JSON). Persona traits with strength ≥ **0.1** in the last **90** days are summarized into the chat **system prompt**. Safe to delete only if you accept losing that substrate; back up with the agent folder. See **[graph-memory.md](graph-memory.md)**. |
| `~/.armaraos/agents/<agent_id>/ainl_graph_memory_inbox.json` | Optional **Python → Rust graph inbox** (JSON envelope: **`nodes`**, **`edges`**, **`source_features`**, **`schema_version`**). Written by **ainativelang** **`AinlMemorySyncWriter`** when **`ARMARAOS_AGENT_ID`** is set; drained at the start of each agent loop into **`ainl_memory.db`**, then reset to an empty file. Safe to delete (pending writes are lost). See **[graph-memory.md](graph-memory.md)** (*Python inbox*). |
| `~/.armaraos/agents/<agent_id>/ainl_graph_memory_export.json` (default) **or** **`$AINL_GRAPH_MEMORY_ARMARAOS_EXPORT/<agent_id>_graph_export.json`** | JSON **export** of the agent subgraph after post-turn graph work so Python **`ainl_graph_memory`** can refresh. See **[graph-memory.md](graph-memory.md)** (*On-disk layout*) and **`GraphMemoryWriter::armaraos_graph_memory_export_json_path`**. |
| `~/.armaraos/agents/<agent_id>/bundle.ainlbundle` | Optional **AINL bundle** JSON (workflow + **memory** snapshot + **persona** + tools). Used by scheduled **`ainl_run`**: the kernel sets **`AINL_BUNDLE_PATH`** before **`ainl run`**; Python **`boot()`** pre-seeds **persona** and **non-persona memory** rows into the JSON graph file when ids are new, then after a successful exit the host best-effort rewrites this file from the live **`ainl_graph_memory`** bridge. See **[scheduled-ainl.md](scheduled-ainl.md)** (*AINL bundle + graph memory*) and **AINL** [`docs/adapters/AINL_GRAPH_MEMORY.md`](https://github.com/sbhooley/ainativelang/blob/main/docs/adapters/AINL_GRAPH_MEMORY.md). |
| `~/.armaraos/skills/` | Installed skills |
| `~/.armaraos/agents/` | Agent manifests and per-agent data |
| `~/.armaraos/daemon.json` | Daemon PID and port when `armaraos` / `openfang start` is running |
| `~/.armaraos/logs/daemon.log` | **CLI daemon** (`openfang start` / `openfang gateway start`): `tracing` output mirrored here and on stderr (created with the `logs/` directory when the daemon starts). The dashboard **Logs → Daemon** tab reads this file via the API. |
| `~/.armaraos/tui.log` | **TUI / `openfang chat`** sessions: tracing is written here so the terminal UI is not corrupted. If `daemon.log` is absent, the daemon log API falls back to this file when present. |
| `~/.armaraos/.env`, `~/.armaraos/secrets.env` | Optional API keys (loaded by CLI and desktop; not committed) |
| `~/.armaraos/ui-prefs.json` | Dashboard UI preferences persisted by the daemon (e.g. **pinned agent** IDs for the sidebar Quick open list). Atomic write (same pattern as `slash-templates.json`). Survives desktop reinstalls that clear WebView `localStorage`. |
| `~/.armaraos/slash-templates.json` | Slash message templates (`/t …`); see [api-reference.md](api-reference.md#slash-templates-endpoints) |
| `~/.armaraos/voice/` | Optional **local voice** bundle: Whisper GGML model, Piper runtime, Piper ONNX voice. Populated when **`[local_voice] auto_download = true`** (default) on first daemon boot — see **[local-voice.md](local-voice.md)**. |

## Overrides

| Variable | Purpose |
|----------|---------|
| `ARMARAOS_HOME` | Preferred: absolute path to the data directory (replaces `~/.armaraos`). |
| `OPENFANG_HOME` | Legacy alias; same effect as `ARMARAOS_HOME`. |
| `AINL_GRAPH_MEMORY_ARMARAOS_EXPORT` | Optional **directory** for per-agent graph JSON exports (**`<agent_id>_graph_export.json`**) instead of the default file next to **`ainl_memory.db`**. See **[graph-memory.md](graph-memory.md)**. |

When either is set, automatic migration (below) does not run for the default home path.

## Migration from `~/.openfang`

Older installs used **`~/.openfang/`**. On first run, if **`~/.armaraos`** does not exist but **`~/.openfang`** is a directory, the runtime **renames** `~/.openfang` → `~/.armaraos` (best-effort). If rename fails (permissions, cross-device move), the process keeps using **`~/.openfang`** until you fix the layout or set `ARMARAOS_HOME` / `OPENFANG_HOME`.

Fresh installs with no prior directory **create** `~/.armaraos` automatically.

Implementation lives in `openfang_types::config` (`openfang_home_dir`, `ensure_armaraos_data_home`), used by the kernel and CLI.

## Config schema version

`config.toml` includes an optional top-level field:

```toml
config_schema_version = 1
```

- **Omitted or `0`:** treated as a **legacy** file from before versioning. On startup the daemon runs **in-memory migrations** (for example aligning old default model IDs), then **appends** `config_schema_version = N` to the file (other content is left as-is).
- **`N` matching the running binary:** no migration.
- **`N` greater than the binary:** the kernel logs a warning (a **newer** app wrote the file; this binary may ignore unknown keys).

The current target version is the `CONFIG_SCHEMA_VERSION` constant in `crates/openfang-types/src/config.rs` (also re-exported from `openfang_kernel::config`). Bump it when you add a new migration step.

**Seeing it live:** The dashboard **Settings** page shows **Config schema** under the tab bar and on **System**; **Daemon & runtime** shows the same pair. **`GET /api/status`** returns `config_schema_version` and `config_schema_version_binary`. Support bundles include both numbers in **`diagnostics_snapshot.json`** and **`meta.json`** (see [troubleshooting.md](troubleshooting.md#dashboard-support-bundle-redacted-zip)).

## Backup and reset (troubleshooting upgrades)

Reinstalling the desktop app or CLI **does not** remove `~/.armaraos/`. If something behaves like a “sticky” error across versions, compare against a **clean profile**:

1. **Quit** the daemon / desktop app.
2. **Back up** the whole folder, e.g. `mv ~/.armaraos ~/.armaraos.bak` (or copy it elsewhere).
3. Start again — a **fresh** `config.toml` and state will be created on first run.

To restore later, rename the backup back. For a partial reset, keep `secrets.env` / `.env` and only replace `config.toml` or the SQLite DB under `data/` as needed.
