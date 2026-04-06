# ArmaraOS data directory

Configuration and local state default to **`~/.armaraos/`** (on Windows, `~` is your user profile).

| Path | Purpose |
|------|---------|
| `~/.armaraos/config.toml` | Main configuration file |
| `~/.armaraos/data/openfang.db` | SQLite database (filename is historical) |
| `~/.armaraos/skills/` | Installed skills |
| `~/.armaraos/agents/` | Agent manifests and per-agent data |
| `~/.armaraos/daemon.json` | Daemon PID and port when `armaraos` / `openfang start` is running |
| `~/.armaraos/logs/daemon.log` | **CLI daemon** (`openfang start` / `openfang gateway start`): `tracing` output mirrored here and on stderr (created with the `logs/` directory when the daemon starts). The dashboard **Logs → Daemon** tab reads this file via the API. |
| `~/.armaraos/tui.log` | **TUI / `openfang chat`** sessions: tracing is written here so the terminal UI is not corrupted. If `daemon.log` is absent, the daemon log API falls back to this file when present. |
| `~/.armaraos/.env`, `~/.armaraos/secrets.env` | Optional API keys (loaded by CLI and desktop; not committed) |

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
