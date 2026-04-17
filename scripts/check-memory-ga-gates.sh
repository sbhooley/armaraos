#!/usr/bin/env bash
# Memory GA gate validator (offline tests + optional live daemon checks).
# Usage:
#   bash scripts/check-memory-ga-gates.sh --offline
#   bash scripts/check-memory-ga-gates.sh --base http://127.0.0.1:4200
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

MODE="offline"
BASE=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --offline)
      MODE="offline"
      shift
      ;;
    --base)
      MODE="live"
      BASE="${2:-}"
      if [[ -z "$BASE" ]]; then
        echo "ERROR: --base requires a URL" >&2
        exit 1
      fi
      shift 2
      ;;
    *)
      echo "ERROR: unknown arg: $1" >&2
      exit 1
      ;;
  esac
done

echo "== Memory GA gates: offline test suite =="
cargo test -p openfang-runtime --lib graph_memory_context -- --nocapture
cargo test -p openfang-runtime --lib ainl_inbox_reader -- --nocapture
cargo test -p openfang-api --lib graph_memory -- --nocapture

if [[ "$MODE" == "offline" ]]; then
  echo "OK: offline memory GA gates passed."
  exit 0
fi

echo "== Memory GA gates: live API checks @ $BASE =="
STATUS_JSON="$(curl -sS -f -m 10 "$BASE/api/status")"
export STATUS_JSON
python3 - <<'PY'
import json, os, sys

raw = os.environ.get("STATUS_JSON", "")
try:
    data = json.loads(raw)
except json.JSONDecodeError as e:
    print(f"ERROR: /api/status invalid JSON: {e}", file=sys.stderr)
    sys.exit(1)

metrics = data.get("graph_memory_context_metrics") or {}
required = [
    "provenance_coverage_ratio",
    "provenance_coverage_floor",
    "provenance_gate_pass",
    "conflict_ratio",
    "conflict_ratio_max",
    "contradiction_gate_pass",
    "temp_mode_suppressed_reads_total",
    "temp_mode_suppressed_writes_total",
    "graph_memory_kernel_notify_ok_total",
    "graph_memory_kernel_notify_err_total",
]
missing = [k for k in required if k not in metrics]
if missing:
    print("ERROR: missing graph_memory_context_metrics keys:", ", ".join(missing), file=sys.stderr)
    sys.exit(1)

if metrics.get("provenance_gate_pass") is not True:
    print("ERROR: provenance_gate_pass=false", file=sys.stderr)
    sys.exit(1)
if metrics.get("contradiction_gate_pass") is not True:
    print("ERROR: contradiction_gate_pass=false", file=sys.stderr)
    sys.exit(1)

print("OK: /api/status graph memory GA gates pass")
PY

echo "-- GET /api/graph-memory/controls key shape"
AGENTS_JSON="$(curl -sS -f -m 10 "$BASE/api/agents")"
export AGENTS_JSON BASE
python3 - <<'PY'
import json, os, sys, urllib.request

raw = os.environ.get("AGENTS_JSON", "")
base = os.environ.get("BASE", "")
try:
    agents = json.loads(raw)
except json.JSONDecodeError as e:
    print(f"ERROR: /api/agents invalid JSON: {e}", file=sys.stderr)
    sys.exit(1)
if not isinstance(agents, list) or not agents:
    print("WARN: no agents found; skipping controls shape check")
    raise SystemExit(0)

agent_id = str(agents[0].get("id") or "")
if not agent_id:
    print("WARN: first agent has no id; skipping controls shape check")
    raise SystemExit(0)

url = f"{base}/api/graph-memory/controls?agent_id={agent_id}"
with urllib.request.urlopen(url, timeout=10) as r:
    payload = json.loads(r.read().decode("utf-8"))
controls = payload.get("controls") or {}
required = [
    "memory_enabled",
    "temporary_mode",
    "shared_memory_enabled",
    "include_episodic_hints",
    "include_semantic_facts",
    "include_conflicts",
    "include_procedural_hints",
]
missing = [k for k in required if k not in controls]
if missing:
    print("ERROR: controls missing keys:", ", ".join(missing), file=sys.stderr)
    sys.exit(1)
print("OK: controls include global + per-block kill switches")
PY

echo "OK: live memory GA checks passed."
