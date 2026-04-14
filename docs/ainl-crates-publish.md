# Publishing `ainl-*` crates to crates.io

Workspace crates under **`crates/ainl-memory`**, **`crates/ainl-persona`**, **`crates/ainl-graph-extractor`**, **`crates/ainl-semantic-tagger`**, and **`crates/ainl-runtime`** are published as **separate** packages (not the ArmaraOS application binary). Use this order so each publish’s **`cargo publish`** verification step can **`cargo build`** against **already-indexed** dependencies.

## Order

1. **`ainl-memory`** — substrate; no in-repo Rust deps on other `ainl-*` crates.
2. **`ainl-semantic-tagger`** — independent of memory; can be published in parallel with step 1 after a version bump, but keeping a linear playbook avoids mistakes.
3. **`ainl-persona`** — depends on **`ainl-memory`** (version pin in `Cargo.toml` must match what is on crates.io).
4. **`ainl-graph-extractor`** — depends on **`ainl-memory`**, **`ainl-persona`**, **`ainl-semantic-tagger`**.
5. **`ainl-runtime`** — depends on all of the above; bump its **`[dependencies]`** version strings before publishing so the published tarball resolves.

**Also update** any workspace crate that pins these versions (e.g. **`openfang-runtime`**, **`openfang-api`**) so **`cargo check --workspace`** stays consistent.

## Dry-run vs live

- **`cargo publish -p ainl-memory --dry-run`** — always safe; validates packaging + compile of that crate alone.
- **Dependents** (`ainl-persona`, …): **`cargo publish -p … --dry-run`** resolves dependencies from the **registry**. If the new **`ainl-memory`** (or other) version is **not** on crates.io yet, dry-run fails with *“failed to select a version for the requirement …”*. That is expected.
- **Workflow:** dry-run **foundation** crates first → **`cargo publish -p ainl-memory`** (live) → wait until `cargo search ainl-memory` shows the new version → dry-run/publish the next crate.

## After publishing

- Bump **workspace path `version = "…"`** strings in downstream `Cargo.toml` files to the new releases.
- Run **`cargo check -p openfang-runtime`** (or full workspace) and commit **`Cargo.lock`** if it changes.
- Refresh **registry alignment** tables in **`docs/ainl-runtime-graph-patch.md`** and **`crates/ainl-runtime/README.md`** when **`ainl-runtime`**’s own version or its dependency floor moves.

## Credentials

Publishing requires **`cargo login`** (or **`CARGO_REGISTRY_TOKEN`**) for the **`sbhooley`** crates.io account that owns these crate names.
