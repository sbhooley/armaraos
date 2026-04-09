#!/usr/bin/env bash
# Smoke-check a running ArmaraOS / ArmaraOS API.
# Default base URL is http://127.0.0.1:4200 — use the URL printed by `openfang start`
# (e.g. http://127.0.0.1:50051) if your config binds a different port.
# Covers health, status, schedules, support zip + downloads, spawn error shape, session digest,
# GET /api/version/github-latest (dashboard “vs GitHub” compare), and GET /api/logs/daemon/recent
# (empty lines OK until daemon.log exists).
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

echo "-- GET /api/version/github-latest (200 JSON; server-side GitHub fetch for dashboard)"
curl -sS -f -m 15 "$BASE/api/version/github-latest" | head -c 500
echo ""

echo "-- GET /api/logs/daemon/recent?lines=5 (200 JSON; lines may be empty if no log file yet)"
curl -sS -f -m 5 "$BASE/api/logs/daemon/recent?lines=5" | head -c 400
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

BUNDLE_FN="$(python3 -c "import json; d=json.load(open('/tmp/armaraos-diag-body.json')); print(d.get('bundle_filename') or '', end='')" 2>/dev/null || true)"
if [[ -n "${BUNDLE_FN:-}" ]]; then
  echo "-- GET /api/support/diagnostics/download?name=$BUNDLE_FN"
  curl -sS -f -o /dev/null -G "$BASE/api/support/diagnostics/download" --data-urlencode "name=$BUNDLE_FN"
  echo "OK (diag zip streamed)"
  echo "-- GET /api/armaraos-home/download?path=support/... (same file)"
  curl -sS -f -o /dev/null -G "$BASE/api/armaraos-home/download" --data-urlencode "path=support/$BUNDLE_FN"
  echo "OK (home download streamed)"
else
  echo "(no bundle_filename in diagnostics response — download checks skipped)"
fi

echo "-- POST /api/agents (expect 401 or structured 4xx when API key required)"
code="$(curl -sS -o /tmp/armaraos-spawn-body.json -w '%{http_code}' -X POST "$BASE/api/agents" \
  -H 'Content-Type: application/json' \
  -d '{"manifest_toml":""}')"
echo "HTTP $code"
head -c 400 /tmp/armaraos-spawn-body.json
echo ""

echo "-- GET /api/agents/:id/session/digest (first agent, if any)"
AGENT_JSON="$(curl -sS -m 5 "$BASE/api/agents" || true)"
AGENT_ID="$(printf '%s' "$AGENT_JSON" | python3 -c "import sys,json; \
  try: \
    a=json.load(sys.stdin); \
    print(a[0]['id'] if isinstance(a,list) and len(a)>0 else '', end=''); \
  except Exception: \
    print('', end='')" 2>/dev/null || true)"
if [[ -n "${AGENT_ID:-}" ]]; then
  curl -sS -f -m 5 "$BASE/api/agents/$AGENT_ID/session/digest" | head -c 400
  echo ""
else
  echo "(no agents in list — skipped)"
fi

echo "OK (smoke requests completed)."
