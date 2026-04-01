# Bundled portable Python (desktop)

Release builds of `openfang-desktop` can ship a **CPython 3.12** “install only” tree under:

`crates/openfang-desktop/resources/python/<rust-target-triple>/python/`

so first launch does not depend on a system Python.

## Populate locally

From the repo root:

```bash
cargo xtask bundle-portable-python --target x86_64-unknown-linux-gnu
cargo xtask bundle-portable-python --target aarch64-apple-darwin
cargo xtask bundle-portable-python --target x86_64-apple-darwin
cargo xtask bundle-portable-python --target x86_64-pc-windows-msvc
```

The xtask downloads the matching **indygreg** `cpython-3.12.7+20241007-*-install_only.tar.gz`, extracts it, and lays out `python/bin/python3` (or `python.exe` on Windows).

## CI / release

- **CI** (`.github/workflows/ci.yml`): `desktop-ainl-resources` bundles Linux GNU triple and verifies the wheel + portable tree.
- **Release** (`.github/workflows/release.yml`): bundles portable Python per matrix `rust_target`.

Directories under `resources/python/*/` are gitignored; only this README is tracked.
