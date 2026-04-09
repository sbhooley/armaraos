# ArmaraOS data directory

Configuration and local state default to **`~/.armaraos/`** (on Windows, `~` is your user profile).

| Path | Purpose |
|------|---------|
| `~/.armaraos/config.toml` | Main configuration file (see **Config schema version** below) |
| `~/.armaraos/data/openfang.db` | SQLite database (filename is historical) |
| `~/.armaraos/skills/` | Installed skills |
| `~/.armaraos/agents/` | Agent manifests and per-agent data |
| `~/.armaraos/daemon.json` | Daemon PID and port when `armaraos` / `openfang start` is running |
| `~/.armaraos/logs/daemon.log` | **CLI daemon** (`openfang start` / `openfang gateway start`): `tracing` output mirrored here and on stderr (created with the `logs/` directory when the daemon starts). The dashboard **Logs → Daemon** tab reads this file via the API. |
| `~/.armaraos/tui.log` | **TUI / `openfang chat`** sessions: tracing is written here so the terminal UI is not corrupted. If `daemon.log` is absent, the daemon log API falls back to this file when present. |
| `~/.armaraos/.env`, `~/.armaraos/secrets.env` | Optional API keys (loaded by CLI and desktop; not committed) |
| `~/.armaraos/ui-prefs.json` | Dashboard UI preferences persisted by the daemon (e.g. **pinned agent** IDs for the sidebar Quick open list). Atomic write (same pattern as `slash-templates.json`). Survives desktop reinstalls that clear WebView `localStorage`. |
| `~/.armaraos/slash-templates.json` | Slash message templates (`/t …`); see [api-reference.md](api-reference.md#slash-templates-endpoints) |

## Overrides

| Variable | Purpose |
|----------|---------|
| `ARMARAOS_HOME` | Preferred: absolute path to the data directory (replaces `~/.armaraos`). |
| `OPENFANG_HOME` | Legacy alias; same effect as `ARMARAOS_HOME`. |

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
