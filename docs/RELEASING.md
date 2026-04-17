# Releasing ArmaraOS (semver tags)

Use this for **routine patch/minor releases** once signing keys, icons, and CI are already set up. For the **first production** gate (Tauri keygen, secrets, icons from scratch), see **[production-checklist.md](production-checklist.md)**. For **desktop smoke** after a tag (Tauri, updater, dashboard), see **[release-desktop.md](release-desktop.md)**. For **pre-tag smoke + manual dashboard checks**, see **[release-candidate-validation.md](release-candidate-validation.md)**. For **human GA sign-off** (product, runtime, security/privacy, data/ML owners), see **[ga-signoff-checklist.md](ga-signoff-checklist.md)**.

---

## Documentation version samples (policy)

Example JSON in **`docs/api-reference.md`** (and related docs called out in the table below) must use the **same semver** as **`[workspace.package].version`** in the repo-root **`Cargo.toml`**. CI runs **`scripts/check-version-consistency.sh`** to catch drift. Prefer updating samples on each bump over leaving stale `0.7.x` literals; for narrative-only examples you may use placeholders like **`vX.Y.Z`** if the doc explicitly says “example tag”.

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

- Add **`## [x.y.z] - YYYY-MM-DD`** under **`[Unreleased]`** (or move **`[Unreleased]`** notes into the new section). While the release is still in progress, **`TBD`** is acceptable for the date; replace it with **`YYYY-MM-DD`** when you tag.
- Keep **`[Unreleased]`** for **post-`x.y.z`** work (or stubbed) after you freeze the release section.
- Add a compare link at the bottom: **`[x.y.z]: https://github.com/sbhooley/armaraos/releases/tag/vx.y.z`**

---

## 3. Marketing site (GitHub fallback)

The Next.js site (**ainativelangweb**) uses **`config/site.ts`** → **`latestArmaraosReleaseTag`** when **`public/downloads/armaraos/latest.json`** is missing. Set it to the **tag you are about to publish** (e.g. **`v0.7.5`**) so homepage/`/download` GitHub fallbacks stay consistent. The **`sync-desktop-updates-to-website`** job in **armaraos** `release.yml` bumps this field when the deploy token is configured; for the commit that **prepares** the release, align **`latestArmaraosReleaseTag`** manually if needed. **Post-tag:** confirm **`https://ainativelang.com/downloads/armaraos/latest.json`** and updater behavior per **[release-desktop.md](release-desktop.md)**.

---

## 4. Pre-tag verification

From the repo root:

```bash
cargo fmt --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
bash scripts/check-version-consistency.sh
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

## Audit / API reminders (0.7.x)

- Successful **`PUT /api/agents/{id}/update`** records **`AgentManifestUpdate`** in the audit log (older rows may still say **`ConfigChange`**).
- **`GET /api/agents/{id}?omit=manifest_toml`** avoids returning the large TOML blob.
