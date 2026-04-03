# Desktop release smoke (Tauri)

Use this after changes that touch the embedded dashboard, AINL bootstrap, or SSE. For architecture and updater signing, see [`desktop.md`](desktop.md). For AINL venv/bootstrap checks only, see [`DESKTOP_AINL_SMOKE.md`](DESKTOP_AINL_SMOKE.md).

## Build

Requires [Tauri CLI v2](https://v2.tauri.app/) (`cargo install tauri-cli --version "^2"`).

```bash
cd crates/openfang-desktop
cargo tauri build
```

Install the produced artifact on a **physical machine** (not only CI): run the app, confirm the window loads the dashboard.

## Updater manifests on ainativelang.com

The desktop app reads the stable feed from `https://ainativelang.com/downloads/armaraos/latest.json` (see `crates/openfang-desktop/tauri.conf.json` and `crates/openfang-desktop/src/ui_prefs.rs`). Users who enable the **beta** channel in Settings use `https://ainativelang.com/downloads/armaraos/beta.json`.

On each **tagged release**, the `sync-desktop-updates-to-website` job in `.github/workflows/release.yml`:

1. Downloads `latest.json` and referenced installer archives from that GitHub Release (from `tauri-apps/tauri-action`).
2. Rewrites every `url` in the manifest to `https://ainativelang.com/downloads/armaraos/<filename>`.
3. Commits into **`sbhooley/ainativelangweb`** at `public/downloads/armaraos/` so Amplify deploys static files.

### Stable vs prerelease tags

| Tag shape | Example | Website behavior |
|-----------|---------|------------------|
| **Stable** (no semver pre-release segment) | `v0.6.2` | Replaces the whole `public/downloads/armaraos/` tree (except `README.md`), writes **`latest.json`** and **`beta.json`** (same manifest until you split feeds). |
| **Prerelease** (semver pre-release after `-`) | `v0.7.0-beta.1` | Updates **`beta.json`** and copies new binaries; **does not** delete or overwrite **`latest.json`**, so stable users stay on the previous stable until you ship a stable tag. |

**One-time setup (armaraos repo secrets):** add **`AINLATIVELANGWEB_DEPLOY_TOKEN`** — a [fine-grained personal access token](https://github.com/settings/tokens?type=beta) with **Contents: Read and write** on repository **`sbhooley/ainativelangweb`** only. Without this secret, the sync job fails (desktop builds and GitHub Release still succeed).

If `ainativelangweb` uses a default branch other than `main`, adjust the `git push` branch in that workflow step.

## Post-release verification (CI + manual)

**Before tagging** (local or CI):

```bash
cd /path/to/armaraos
cargo build --workspace --lib
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

**After the workflow finishes** (especially `sync-desktop-updates-to-website`):

- [ ] `https://ainativelang.com/downloads/armaraos/latest.json` — valid JSON, `url` fields point at `ainativelang.com`, signatures present.
- [ ] Same for `beta.json` (after first successful sync; stable tags copy `latest.json` → `beta.json`).
- [ ] For a **prerelease** tag only: confirm `latest.json` on the site was **not** replaced; `beta.json` reflects the new build.
- [ ] Install a previous desktop build, enable beta if testing beta feed, confirm **Check for updates** sees the new version when expected.

## Smoke checklist

1. **Tauri updater** — Install the previous build, launch, confirm an update is offered and applies (manifest at ainativelang.com after the first successful sync job).
2. **AINL** — **Settings → AINL** (desktop shell): **Check versions** runs; **Last checked** updates; upgrade path if you ship pip upgrades.
3. **Kernel SSE** — Sidebar **SSE** badge shows when connected; Overview **Last kernel event** updates when agents spawn or system events fire (optional spot-check).
4. **Core flows** — Spawn an agent, send a message, open Logs/Scheduler as needed for your release.
5. **Dashboard errors** — Disconnect daemon or force a 401; on **Overview**, **Chat (agents)**, and **Settings**, confirm structured error text, **Retry**, **Copy debug info**, and **Generate + copy bundle** behave as expected.

## API tests (non-desktop)

```bash
cargo test -p openfang-api --test api_integration_test test_kernel_events_stream_sse_smoke
cargo test -p openfang-api --test sse_stream_auth
```
