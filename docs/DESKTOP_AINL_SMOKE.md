# Desktop AINL bootstrap — smoke checklist

Use this after changing `openfang-desktop` AINL code, CI resource steps, or Tauri permissions.

For a broader desktop release smoke (Tauri build, updater, SSE badge), see [`release-desktop.md`](release-desktop.md).

## Prerequisites

1. Build or run the desktop app with embedded dashboard (Tauri).
2. Optional: pre-bundle resources for offline behavior:
   - `cargo xtask bundle-ainl-wheel`
   - `cargo xtask bundle-portable-python --target <your rust triple>`

## Manual checks

1. **First launch**
   - App starts; no crash during background AINL bootstrap (if enabled).
   - `~/.armaraos/config.toml` contains an `ainl-mcp` entry after success (see [data-directory.md](data-directory.md) if your home path differs).

2. **Settings → AINL** (desktop only)
   - Tab is visible in the Tauri shell, hidden in a plain browser session.
   - **Refresh status** shows fields consistent with the internal venv (Ready, venv, `ainl` / `ainl-mcp`, portable Python flag, wheel flag).
   - **Bootstrap AINL** completes without error when wheel or PyPI is available.
   - **Update host config only** succeeds when the venv is already healthy.

3. **Air-gap / no wheel**
   - With no wheel and no network, bootstrap should fail with a clear error mentioning PyPI / bundling / `ARMARAOS_AINL_PYPI_SPEC`.

4. **Environment overrides**
   - `ARMARAOS_PYTHON`: forces the interpreter used to create the venv when no bundled portable Python matches.
   - `ARMARAOS_AINL_PYPI_SPEC`: alternate pip requirement when the bundled wheel is absent.

## CLI parity (optional)

On a machine with the same venv layout, compare behavior to:

`ainl install armaraos`

The desktop host integration is intended to mirror that flow for `ainl-mcp` and wrapper scripts under `~/.armaraos/`.
