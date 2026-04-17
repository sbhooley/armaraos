#!/usr/bin/env bash
# Verifies workspace semver matches Tauri manifest, README badge, and canonical
# sample JSON in docs/api-reference.md. Run from repo root (CI + pre-tag).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

CARGO_VER="$(grep -A40 '^\[workspace.package\]' Cargo.toml | grep -E '^version = ' | head -1 | sed -E 's/^version = "([^"]+)".*/\1/')"

if [[ -z "$CARGO_VER" ]]; then
  echo "check-version-consistency: could not read [workspace.package].version from Cargo.toml" >&2
  exit 1
fi

TAURI_VER="$(python3 -c 'import json; print(json.load(open("crates/openfang-desktop/tauri.conf.json"))["version"])')"

if [[ "$CARGO_VER" != "$TAURI_VER" ]]; then
  echo "check-version-consistency: mismatch Cargo.toml workspace=$CARGO_VER tauri.conf.json=$TAURI_VER" >&2
  exit 1
fi

if ! grep -q "version-${CARGO_VER}-green" README.md; then
  echo "check-version-consistency: README.md badge must contain version-${CARGO_VER}-green" >&2
  exit 1
fi

if ! grep -q "\"version\": \"${CARGO_VER}\"" docs/api-reference.md; then
  echo "check-version-consistency: docs/api-reference.md must include GET /api/status sample \"version\": \"${CARGO_VER}\"" >&2
  exit 1
fi

if ! grep -q "\"tag_name\": \"v${CARGO_VER}\"" docs/api-reference.md; then
  echo "check-version-consistency: docs/api-reference.md must include github-latest sample tag_name v${CARGO_VER}" >&2
  exit 1
fi

if ! grep -q "releases/tag/v${CARGO_VER}" docs/api-reference.md; then
  echo "check-version-consistency: docs/api-reference.md must include html_url .../releases/tag/v${CARGO_VER}" >&2
  exit 1
fi

echo "check-version-consistency: OK (workspace + Tauri + README + docs/api-reference.md → ${CARGO_VER})"
