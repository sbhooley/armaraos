# Release candidate validation

Run these **before** tagging `vX.Y.Z` so embedded dashboard assets, API samples, updater paths, and embedded **AINL** stay aligned. Pair with **[RELEASING.md](RELEASING.md)** and **[release-desktop.md](release-desktop.md)**.

**Human GA sign-off (product, runtime, security/privacy, data/ML):** After automated checks pass, run the step-by-step approvals in **[ga-signoff-checklist.md](ga-signoff-checklist.md)** and attach evidence to the release ticket.

## Automated checks (repo root)

```bash
cargo fmt --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
bash scripts/check-version-consistency.sh
bash scripts/check-memory-ga-gates.sh --offline
```

Optional: `cargo build --release -p openfang-cli`

**CI:** On **`main`** PRs, **`dashboard-smoke`** (runs **`scripts/ci-dashboard-smoke.sh`**) and **`desktop-ainl-resources`** (bundles **`AINL_PYPI_VERSION`** from **`.github/workflows/ci.yml`**) are **merge-gating**. Failures block merges until fixed.

## Daemon + dashboard smoke (local)

With the daemon listening, use the same base URL as **`api_listen`** from **`GET /api/status`** (common values: `http://127.0.0.1:4200`, `http://127.0.0.1:50051`).

```bash
BASE="http://127.0.0.1:4200"
./scripts/verify-dashboard-smoke.sh "$BASE"
bash scripts/check-memory-ga-gates.sh --base "$BASE"
```

See **[dashboard-testing.md](dashboard-testing.md)** for the full manual browser checklist (Get started, Quick actions including **Browse Skills/MCP**, notification bell, command palette, chat, diagnostics).

## 0.7.7 release risks (CHANGELOG)

These are **not** automatic CI gates; confirm before tagging or wide rollout.

### `ainl_runtime_engine` defaulting on + legacy migration

- **Default:** New **`AgentManifest`** / wizard agents have **`ainl_runtime_engine = true`**; **`openfang-runtime`** default features include the engine.
- **Migration:** On daemon boot, agents with **no** explicit **`ainl_runtime_engine`** in on-disk **`agent.toml`** migrate to **`true`** and persist to SQLite; explicit **`true`/`false`** on disk are unchanged.
- **Validate:** If you rely on the engine **off** for a specific agent, open **Agents → Config**, set **`ainl_runtime_engine`** explicitly to **`false`**, save, and confirm it survives restart and **`GET /api/agents`** list/detail.

### Adaptive eco

- Touches budgeting, compression, and usage exports.
- **Validate:** Staging policy + replay using **`scripts/verify-adaptive-eco-usage.sh`** and the **[prompt-compression-efficient-mode.md](prompt-compression-efficient-mode.md)** / Budget surfaces as needed before org-wide rollout.

### Desktop updater and marketing site

- **Secret:** **`AINLATIVELANGWEB_DEPLOY_TOKEN`** on the **armaraos** repo (fine-grained PAT for **ainativelangweb**) — without it, **`sync-desktop-updates-to-website`** cannot push **`latest.json`** / binaries.
- **After tag + CI:** Confirm **`https://ainativelang.com/downloads/armaraos/latest.json`** (and **`beta.json`** if applicable), in-app **Check for updates**, and GitHub Release assets — see **Post-release verification** in **[release-desktop.md](release-desktop.md)**.

### Embedded AINL wheel vs PyPI

- **`AINL_PYPI_VERSION`** in **`.github/workflows/ci.yml`** and **`release.yml`** must match a published **[PyPI `ainativelang`](https://pypi.org/project/ainativelang/)** version and stay aligned with **ainativelangweb** **`latestAinlRelease`** where practical.
- **Validate (desktop):** **Settings → AINL** — **Check versions**, bundled wheel install, and optional upgrade path (**`ARMARAOS_AINL_PYPI_SPEC`** override if testing mirrors).

## Desktop / updater (after tag + CI)

Post-tag verification: **`latest.json`** / **`beta.json`** on **ainativelang.com**, in-app **Check for updates**, and **`sync-desktop-updates-to-website`** success — see **Post-release verification** in **[release-desktop.md](release-desktop.md)**.
