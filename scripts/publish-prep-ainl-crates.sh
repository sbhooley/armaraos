#!/usr/bin/env bash
# Publish-prep for AINL workspace crates that ship to crates.io.
#
# Prerequisite: **ainl-memory** must lead the train whenever its schema or public
# API moves ahead of the last crates.io release (currently 0.1.2-alpha). Local
# `EpisodicNode` additions (`user_message`, `assistant_response`) and any
# extractor-facing fields require **0.1.3-alpha** (or newer) before publishing
# **ainl-persona**, **ainl-graph-extractor**, or **ainl-runtime**.
#
# This script does NOT publish. It:
#   1. Summarizes local vs published assumptions
#   2. Verifies workspace members build
#   3. Runs `cargo publish --dry-run` for **ainl-memory** (must succeed)
#   4. Runs the same for downstream crates (expected to fail until the prior
#      crate exists on crates.io at the pinned version — still useful to catch
#      unrelated packaging errors once dependencies are live)
#
# Typical extra flags: `--allow-dirty` when validating before commit.
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "=============================================="
echo " 1) Local ainl-memory vs crates.io baseline"
echo "=============================================="
echo "Published on crates.io through: 0.1.2-alpha (per crates.io index / maintainer)."
echo "Workspace package version:"
grep '^version' crates/ainl-memory/Cargo.toml || true
echo ""
echo "Notable API/schema deltas since 0.1.2-alpha (see crates/ainl-memory/CHANGELOG.md):"
echo "  - EpisodicNode: optional user_message, assistant_response (JSON optional)."
echo "  - new_episode: initializes those fields to None."
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
echo "  cargo publish -p ainl-memory $@"
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
    echo "NOTE: ${pkg} dry-run failed — expected until ainl-memory 0.1.3-alpha (and any"
    echo "      intermediate persona release) is published and indexed on crates.io."
  fi
done

echo ""
if [[ "$downstream_fail" -eq 1 ]]; then
  echo "Summary: ainl-memory packaging verified. Re-run this script after publishing"
  echo "         each crate in order to validate the next hop."
else
  echo "Summary: all listed dry-runs succeeded (crates.io already has required versions)."
fi
