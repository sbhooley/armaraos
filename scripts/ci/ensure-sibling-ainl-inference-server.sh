#!/usr/bin/env bash
# openfang-runtime depends on armara-provider-api via a path that leaves the
# armaraos repo (../../../ainl-inference-server/...). Local dev often keeps
# ainl-inference-server as a sibling of armaraos; GitHub Actions only checks
# out armaraos, so clone the sibling when missing.
set -euo pipefail

ROOT="${GITHUB_WORKSPACE:-}"
if [[ -z "$ROOT" ]]; then
  echo "ensure-sibling-ainl-inference-server: GITHUB_WORKSPACE unset; skipping."
  exit 0
fi

PARENT="$(cd "$(dirname "$ROOT")" && pwd)"
DEST="$PARENT/ainl-inference-server"
MARKER="$DEST/crates/armara-provider-api/Cargo.toml"

if [[ -f "$MARKER" ]]; then
  echo "ensure-sibling-ainl-inference-server: OK ($MARKER exists)"
  exit 0
fi

REF="${AINL_INFERENCE_SERVER_REF:-main}"
URL="${AINL_INFERENCE_SERVER_REPO:-https://github.com/sbhooley/ainl-inference-server.git}"

echo "ensure-sibling-ainl-inference-server: cloning $URL (ref $REF) -> $DEST"
git clone --depth 1 --branch "$REF" "$URL" "$DEST"

if [[ ! -f "$MARKER" ]]; then
  echo "ensure-sibling-ainl-inference-server: clone failed — missing $MARKER" >&2
  exit 1
fi
