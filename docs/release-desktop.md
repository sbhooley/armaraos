# Desktop release smoke (Tauri)

Use this after changes that touch the embedded dashboard, AINL bootstrap, or SSE. For architecture and updater signing, see [`desktop.md`](desktop.md). For AINL venv/bootstrap checks only, see [`DESKTOP_AINL_SMOKE.md`](DESKTOP_AINL_SMOKE.md).

## Build

Requires [Tauri CLI v2](https://v2.tauri.app/) (`cargo install tauri-cli --version "^2"`).

```bash
cd crates/openfang-desktop
cargo tauri build
```

Install the produced artifact on a **physical machine** (not only CI): run the app, confirm the window loads the dashboard.

## Updater manifest on ainativelang.com

The desktop app loads `https://ainativelang.com/downloads/armaraos/latest.json` (see `crates/openfang-desktop/tauri.conf.json`). On each **tagged release**, the `sync-desktop-updates-to-website` job in `.github/workflows/release.yml`:

1. Downloads all assets from that GitHub Release (including `latest.json` produced by `tauri-apps/tauri-action`).
2. Rewrites every `url` in `latest.json` to point at `https://ainativelang.com/downloads/armaraos/<filename>`.
3. Commits `latest.json` plus those binaries into **`sbhooley/ainativelangweb`** at `public/downloads/armaraos/` so Amplify deploys them as static files.

**One-time setup (armaraos repo secrets):** add **`AINLATIVELANGWEB_DEPLOY_TOKEN`** — a [fine-grained personal access token](https://github.com/settings/tokens?type=beta) with **Contents: Read and write** on repository **`sbhooley/ainativelangweb`** only. Without this secret, the sync job fails (desktop builds and GitHub Release still succeed).

If `ainativelangweb` uses a default branch other than `main`, adjust the `git push` branch in that workflow step.

## Smoke checklist

1. **Tauri updater** — Install the previous build, launch, confirm an update is offered and applies (manifest at ainativelang.com after the first successful sync job).
2. **AINL** — **Settings → AINL** (desktop shell): **Check versions** runs; **Last checked** updates; upgrade path if you ship pip upgrades.
3. **Kernel SSE** — Sidebar **SSE** badge shows when connected; Overview **Last kernel event** updates when agents spawn or system events fire (optional spot-check).
4. **Core flows** — Spawn an agent, send a message, open Logs/Scheduler as needed for your release.

## API tests (non-desktop)

```bash
cargo test -p openfang-api --test api_integration_test test_kernel_events_stream_sse_smoke
cargo test -p openfang-api --test sse_stream_auth
```
