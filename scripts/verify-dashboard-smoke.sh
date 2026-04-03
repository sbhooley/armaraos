#!/usr/bin/env bash
# Smoke-check a running ArmaraOS / OpenFang API.
# Default base URL is http://127.0.0.1:4200 — use the URL printed by `openfang start`
# (e.g. http://127.0.0.1:50051) if your config binds a different port.
# Usage: ./scripts/verify-dashboard-smoke.sh [BASE_URL]
set -euo pipefail

BASE="${1:-http://127.0.0.1:4200}"

echo "== ArmaraOS dashboard API smoke: $BASE =="

if ! curl -sS -m 3 -o /dev/null "$BASE/api/health"; then
  echo "ERROR: No response from $BASE/api/health — start the daemon first (e.g. openfang start)."
  exit 1
fi

echo "-- GET /api/health"
curl -sS -f "$BASE/api/health" | head -c 400
echo ""

echo "-- GET /api/status"
curl -sS -f "$BASE/api/status" | head -c 600
echo ""

echo "-- GET /api/schedules (expect 200 JSON)"
curl -sS -f "$BASE/api/schedules" | head -c 400
echo ""

echo "-- POST /api/support/diagnostics (loopback only; writes ~/.armaraos/support/*.zip)"
RID="$(curl -sS -D - -o /tmp/armaraos-diag-body.json -X POST "$BASE/api/support/diagnostics" \
  -H 'Content-Type: application/json' \
  -d '{}' | tr -d '\r' | awk -F': ' 'tolower($1)=="x-request-id"{print $2; exit}')"
if [[ -n "${RID:-}" ]]; then
  echo "x-request-id: $RID"
fi
head -c 300 /tmp/armaraos-diag-body.json
echo ""

echo "-- POST /api/agents (expect 401 or structured 4xx when API key required)"
code="$(curl -sS -o /tmp/armaraos-spawn-body.json -w '%{http_code}' -X POST "$BASE/api/agents" \
  -H 'Content-Type: application/json' \
  -d '{"manifest_toml":""}')"
echo "HTTP $code"
head -c 400 /tmp/armaraos-spawn-body.json
echo ""

echo "OK (smoke requests completed)."
