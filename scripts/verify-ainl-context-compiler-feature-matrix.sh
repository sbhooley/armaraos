#!/usr/bin/env bash
# §16 Phase 6 boundary check: `ainl-context-compiler` must build with each default
# feature omitted in turn (sources remain independently optional at the Cargo level).
# Run from the armaraos repo root: ./scripts/verify-ainl-context-compiler-feature-matrix.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# Must match the `[features] default = [ ... ]` list in
# `crates/ainl-context-compiler/Cargo.toml` **exactly** (set equality; order not significant).
# After changing that `default` array, update this line and re-run the script.
read -r -a DEFAULT_FEATS <<<"sources-bulk sources-graph-memory sources-failure-warnings sources-trajectory-recap freshness tagger vitals"

echo "==> ainl-context-compiler: full default set"
cargo test -q -p ainl-context-compiler

for omit in "${DEFAULT_FEATS[@]}"; do
  feats=()
  for f in "${DEFAULT_FEATS[@]}"; do
    if [[ "$f" != "$omit" ]]; then
      feats+=("$f")
    fi
  done
  echo "==> omitting feature: $omit  (kept: ${feats[*]})"
  cargo test -q -p ainl-context-compiler --no-default-features --features "${feats[*]}"
done

echo "==> feature-matrix check OK"
