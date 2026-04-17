#!/usr/bin/env bash
# Fetch adaptive eco + compression usage JSON from a running daemon (staging / local).
# Requires real traffic after [adaptive_eco] enabled — empty DB returns zeros.
#
# Usage:
#   ./scripts/verify-adaptive-eco-usage.sh [BASE_URL] [window]
# Examples:
#   ./scripts/verify-adaptive-eco-usage.sh
#   ./scripts/verify-adaptive-eco-usage.sh http://127.0.0.1:4200 7d
#
# Auth: if OPENFANG_API_KEY is set, sends Authorization: Bearer <key>.
set -euo pipefail

BASE="${1:-http://127.0.0.1:4200}"
WIN="${2:-7d}"
WIN_Q="window=${WIN}"

CURL_AUTH=()
if [[ -n "${OPENFANG_API_KEY:-}" ]]; then
  CURL_AUTH=(-H "Authorization: Bearer ${OPENFANG_API_KEY}")
fi

json_fmt() {
  if command -v python3 >/dev/null 2>&1; then
    python3 -m json.tool 2>/dev/null || cat
  elif command -v jq >/dev/null 2>&1; then
    jq .
  else
    cat
  fi
}

echo "== Adaptive eco usage (BASE=$BASE window=$WIN) =="
if ! curl -sS -m 5 -o /dev/null "${CURL_AUTH[@]}" "$BASE/api/health" 2>/dev/null; then
  echo "WARN: No response from $BASE/api/health — start the daemon first."
fi

echo ""
echo "-- GET /api/usage/adaptive-eco?$WIN_Q"
curl -sS -f -m 15 "${CURL_AUTH[@]}" "$BASE/api/usage/adaptive-eco?$WIN_Q" | json_fmt | head -n 80
echo ""

echo "-- GET /api/usage/adaptive-eco/replay?$WIN_Q"
curl -sS -f -m 15 "${CURL_AUTH[@]}" "$BASE/api/usage/adaptive-eco/replay?$WIN_Q" | json_fmt | head -n 100
echo ""

echo "-- GET /api/usage/compression?$WIN_Q (includes adaptive_eco bundle when present)"
curl -sS -f -m 20 "${CURL_AUTH[@]}" "$BASE/api/usage/compression?$WIN_Q" | json_fmt | head -n 120
echo ""

echo "Done. See docs/operations/ADAPTIVE_ECO_STAGING_AND_ENFORCEMENT.md for interpretation."
