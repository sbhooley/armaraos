#!/usr/bin/env bash
# Publish-prep for AINL workspace crates that ship to crates.io.
#
# Order (see also **docs/ainl-runtime-graph-patch.md** — *Pre-release versions*):
#   **ainl-memory** → **ainl-persona** → **ainl-graph-extractor** → **ainl-runtime**
# Optional: **ainl-semantic-tagger** when it changes (extractor/runtime pin it).
#
# Whenever **ainl-memory** bumps its **pre-release** line, published dependents that
# still declare an older caret floor (e.g. **^0.1.3-alpha**) can prevent Cargo from
# unifying with **^0.1.5-alpha** / **^0.1.8-alpha** on another edge — **cargo publish**
# for **ainl-runtime** then fails until **new** persona / extractor **versions** are on
# crates.io with aligned **ainl-memory** requirements.
#
# This script does NOT publish. It:
#   1. Prints workspace **ainl-memory** version from Cargo.toml (sanity)
#   2. Verifies workspace members build
#   3. Runs `cargo publish --dry-run` for **ainl-memory** (must succeed)
#   4. Runs the same for downstream crates (may fail until the prior crate is
#      published and indexed — still catches unrelated packaging errors)
#
# Typical extra flags: `--allow-dirty` when validating before commit.
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "=============================================="
echo " 1) Workspace ainl-memory version (sanity)"
echo "=============================================="
grep '^version' crates/ainl-memory/Cargo.toml || true
echo ""
echo "Changelog + schema notes: crates/ainl-memory/CHANGELOG.md"
echo "crates.io alignment table: docs/ainl-runtime-graph-patch.md"
echo ""

echo "=============================================="
echo " 2) Workspace compile check (AINL crates)"
echo "=============================================="
cargo check -p ainl-memory -p ainl-persona -p ainl-graph-extractor -p ainl-runtime

echo ""
echo "=============================================="
echo " 3) crates.io publish --dry-run (dependency order)"
echo "=============================================="
echo "Exact publish commands (run for real after each step lands on crates.io):"
echo ""
echo "  cd \"\$REPO_ROOT\""
echo "  cargo publish -p ainl-memory $@          # wait for index if downstream dry-run fails"
echo "  cargo publish -p ainl-semantic-tagger $@ # when tagger API/version changed"
echo "  cargo publish -p ainl-persona $@"
echo "  cargo publish -p ainl-graph-extractor $@"
echo "  cargo publish -p ainl-runtime $@"
echo ""

echo ">>> cargo publish -p ainl-memory --dry-run $@"
if ! cargo publish -p ainl-memory --dry-run "$@"; then
  echo "ERROR: ainl-memory dry-run failed — fix before any publish."
  exit 1
fi

DOWNSTREAM=(ainl-persona ainl-graph-extractor ainl-runtime)
downstream_fail=0
for pkg in "${DOWNSTREAM[@]}"; do
  echo ""
  echo ">>> cargo publish -p ${pkg} --dry-run $@"
  if cargo publish -p "${pkg}" --dry-run "$@"; then
    echo "OK: ${pkg} (dependencies already on crates.io at required versions)"
  else
    downstream_fail=1
    echo "NOTE: ${pkg} dry-run failed — expected until prior crates in the chain are"
    echo "      published and indexed (or fix pre-release version unification; see"
    echo "      docs/ainl-runtime-graph-patch.md)."
  fi
done

echo ""
if [[ "$downstream_fail" -eq 1 ]]; then
  echo "Summary: ainl-memory packaging verified. Re-run this script after publishing"
  echo "         each crate in order to validate the next hop."
else
  echo "Summary: all listed dry-runs succeeded (crates.io already has required versions)."
fi
