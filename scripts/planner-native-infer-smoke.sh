#!/usr/bin/env bash
# Manual smoke: planner path + native infer URL + metrics snippet.
# Usage:
#   export ARMARA_NATIVE_INFER_URL=http://127.0.0.1:8787
#   export ARMARAOS_API=http://127.0.0.1:4200   # daemon
#   ./scripts/planner-native-infer-smoke.sh
set -euo pipefail

ARMARAOS_API="${ARMARAOS_API:-http://127.0.0.1:4200}"
INF="${ARMARA_NATIVE_INFER_URL:-}"

if [[ -z "$INF" ]]; then
  echo "Set ARMARA_NATIVE_INFER_URL to your ainl-inference-server base URL." >&2
  exit 1
fi

echo "== Infer health (optional) =="
curl -fsS "${INF%/}/health" 2>/dev/null || echo "(no /health — skip)"

echo "== API metrics (planner counters) =="
curl -fsS "${ARMARAOS_API%/}/metrics" | grep -E '^openfang_planner_' || echo "(no planner counters yet — send a planner-mode message first)"

echo "== Recent orchestration traces =="
curl -fsS "${ARMARAOS_API%/}/api/orchestration/traces?limit=5" | head -c 2000
echo

echo "Done. For a full turn, use the dashboard or POST /api/agents/{id}/message with a planner_mode agent."
