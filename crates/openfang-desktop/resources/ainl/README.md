## Bundled AINL (Option A)

This directory is reserved for bundling AINL into the desktop app.

### What belongs here

- **A pinned `ainativelang` wheel**, for example:
  - `ainativelang-1.3.1-py3-none-any.whl`

Option A bundling uses an **internal virtualenv** under the app data directory and installs this wheel offline on first run.

### How to populate this directory (recommended)

From the repo root:

```bash
cargo run -p xtask -- bundle-ainl-wheel --version 1.3.1
```

### Why the wheel is not committed

Wheel files are large binaries and should be attached by the release pipeline (or a packaging step) rather than committed to git.

### How the desktop app finds it

The desktop app looks for a wheel matching:

- `resources/ainl/ainativelang-*-py3-none-any.whl`

and installs it into:

- `<app_data_dir>/ainl/venv/`

