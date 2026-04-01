## Bundled AINL (Option A)

This directory is reserved for bundling AINL into the desktop app.

### What belongs here

- **A pinned `ainativelang` wheel**, for example:
  - `ainativelang-1.4.0-py3-none-any.whl`

Option A bundling uses an **internal virtualenv** under the app data directory. **If a wheel is present**, it is installed offline on first run. **If no wheel is bundled**, the app automatically runs `pip install` of `ainativelang[mcp]` from PyPI (one-time network; default spec is overridable via `ARMARAOS_AINL_PYPI_SPEC`). **Python** is auto-detected (`python3`, `python`, Windows `py -3`, or `ARMARAOS_PYTHON`).

### How to populate this directory (recommended)

From the repo root:

```bash
cargo run -p xtask -- bundle-ainl-wheel --version 1.4.0
```

### Why the wheel is not committed

Wheel files are large binaries and should be attached by the release pipeline (or a packaging step) rather than committed to git.

### How the desktop app finds it

The desktop app looks for a wheel matching:

- `resources/ainl/ainativelang-*-py3-none-any.whl`

and installs it into:

- `<app_data_dir>/ainl/venv/`

### After the venv is ready

The desktop app also runs the same **MCP + `ainl-run` registration** as `ainl install armaraos` (see `AI_Native_Lang/tooling/mcp_host_install.py`), with the app venv’s `ainl` / `ainl-mcp` on `PATH`, so the resolved **`config.toml`** under the ArmaraOS data home (default `~/.armaraos/`) references the correct binaries. That host step does not perform an extra PyPI install. Tauri command `ensure_armaraos_ainl_host` repeats only that step if needed.

