# Releasing ArmaraOS (semver tags)

Use this for **routine patch/minor releases** once signing keys, icons, and CI are already set up. For the **first production** gate (Tauri keygen, secrets, icons from scratch), see **[production-checklist.md](production-checklist.md)**. For **desktop smoke** after a tag (Tauri, updater, dashboard), see **[release-desktop.md](release-desktop.md)**.

---

## 1. Version bump (workspace)

All crates inherit **`[workspace.package].version`** in the repo-root **`Cargo.toml`**. Bump **once**:

```toml
[workspace.package]
version = "0.7.x"   # example
```

Also bump:

| Location | Field |
|----------|--------|
| **`crates/openfang-desktop/tauri.conf.json`** | `"version"` (must match the Rust semver users see) |
| **Root `README.md`** | Version badge URL + `alt` if present |
| **`docs/api-reference.md`** | Example JSON for **`GET /api/status`** / **`GET /api/version/github-latest`** where versions appear as samples |
| **`docs/ainl-showcases.md`** | Example JSON `armaraos_running` / `armaraos_upstream_tag` if you keep them aligned |
| **`docs/launch-roadmap.md`**, **`docs/release-desktop.md`** | Example tag cells in tables (e.g. `v0.7.x`) |

Regenerate lockfile metadata:

```bash
cargo build -p openfang-cli
```

Commit **`Cargo.lock`** with the version bump.

---

## 2. Changelog

Edit root **`CHANGELOG.md`**:

- Add **`## [x.y.z] - YYYY-MM-DD`** under **`[Unreleased]`** (or move **`[Unreleased]`** notes into the new section).
- Keep **`[Unreleased]`** empty or stubbed after the release section.
- Add a compare link at the bottom: **`[x.y.z]: https://github.com/sbhooley/armaraos/releases/tag/vx.y.z`**

---

## 3. Marketing site (GitHub fallback)

The Next.js site (**ainativelangweb**) uses **`config/site.ts`** → **`latestArmaraosReleaseTag`** when **`public/downloads/armaraos/latest.json`** is missing. Set it to the **tag you are about to publish** (e.g. **`v0.7.2`**) so homepage/`/download` GitHub fallbacks stay consistent. The **`sync-desktop-updates-to-website`** job in **armaraos** `release.yml` pushes installer manifests into that repo after the release workflow runs.

---

## 4. Pre-tag verification

From the repo root:

```bash
cargo fmt --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Optional: **`cargo build --release -p openfang-cli`** for a final binary sanity check.

---

## 5. Tag and GitHub Release

```bash
git tag -a vx.y.z -m "ArmaraOS x.y.z"
git push origin main --tags
```

Publish a **GitHub Release** from that tag (release notes can be copied from **`CHANGELOG.md`**). CI **`release.yml`** builds desktop artifacts and, when configured, syncs **`latest.json`** / binaries to **ainativelangweb**.

---

## 6. Post-release checks

See **[release-desktop.md](release-desktop.md)** (post-release verification + smoke checklist): **`latest.json`** on **ainativelang.com**, **`beta.json`**, updater behavior, core dashboard flows.

---

## Audit / API reminders (0.7.2+)

- Successful **`PUT /api/agents/{id}/update`** records **`AgentManifestUpdate`** in the audit log (older rows may still say **`ConfigChange`**).
- **`GET /api/agents/{id}?omit=manifest_toml`** avoids returning the large TOML blob.
