# ArmaraOS data directory

Configuration and local state default to **`~/.armaraos/`** (on Windows, `~` is your user profile).

| Path | Purpose |
|------|---------|
| `~/.armaraos/config.toml` | Main configuration file |
| `~/.armaraos/data/openfang.db` | SQLite database (filename is historical) |
| `~/.armaraos/skills/` | Installed skills |
| `~/.armaraos/agents/` | Agent manifests and per-agent data |
| `~/.armaraos/daemon.json` | Daemon PID and port when `armaraos` / `openfang start` is running |
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
