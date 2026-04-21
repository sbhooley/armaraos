#!/usr/bin/env bash
# Pin: tooling/posthog-js.version  (single line, semver of posthog-js on npm).
# Fetches array.full.es5.js into the embedded dashboard vendor path (automatic for CI + releases).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VER_FILE="$ROOT/tooling/posthog-js.version"
OUT="$ROOT/crates/openfang-api/static/vendor/posthog.array.full.es5.js"
VER="$(tr -d ' \t\r\n' < "$VER_FILE")"
if [[ -z "${VER:-}" ]]; then
  echo "error: empty or missing ${VER_FILE}" >&2
  exit 1
fi
URL="https://unpkg.com/posthog-js@${VER}/dist/array.full.es5.js"
echo "Vendoring posthog-js@${VER} -> ${OUT}"
TMP="${OUT}.$$.tmp"
curl -fsSL "$URL" -o "$TMP"
{
  printf '%s\n' "/* posthog-js@${VER} array.full.es5 - bump tooling/posthog-js.version; run scripts/vendor-posthog-js.sh */"
  cat "$TMP"
} > "${OUT}"
rm -f "$TMP"
echo "Done ($(wc -c < "$OUT" | tr -d ' ') bytes)."
