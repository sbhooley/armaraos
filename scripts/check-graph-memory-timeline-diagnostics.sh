#!/usr/bin/env bash
# Interpret graph-memory policy + kernel notify counters from GET /api/status.
# Use when the graph updates (GET /api/graph-memory) but the live SSE timeline stays empty.
#
# Usage:
#   bash scripts/check-graph-memory-timeline-diagnostics.sh
#   bash scripts/check-graph-memory-timeline-diagnostics.sh http://127.0.0.1:4200
set -euo pipefail

BASE="${1:-http://127.0.0.1:4200}"
URL="${BASE%/}/api/status"

echo "== Graph memory timeline diagnostics ( ${URL} ) =="
JSON="$(curl -sS -f -m 12 "$URL")" || {
  echo "ERROR: could not GET /api/status (daemon running? base URL correct?)" >&2
  exit 1
}

export JSON
export BASE_URL="$BASE"
python3 - <<'PY'
import json, os, sys

data = json.loads(os.environ["JSON"])
metrics = data.get("graph_memory_context_metrics") or {}

def u64(name, default=0):
    v = metrics.get(name)
    if v is None:
        return default
    return int(v)

ok = u64("graph_memory_kernel_notify_ok_total")
err = u64("graph_memory_kernel_notify_err_total")
tw = u64("temp_mode_suppressed_writes_total")
rw = u64("rollout_suppressed_writes_total")
tr = u64("temp_mode_suppressed_reads_total")
rr = u64("rollout_suppressed_reads_total")

print("Counters (process lifetime, reset on daemon restart):")
print(f"  graph_memory_kernel_notify_ok_total:   {ok}")
print(f"  graph_memory_kernel_notify_err_total:  {err}")
print(f"  temp_mode_suppressed_writes_total:     {tw}")
print(f"  rollout_suppressed_writes_total:       {rw}")
print(f"  temp_mode_suppressed_reads_total:      {tr}")
print(f"  rollout_suppressed_reads_total:        {rr}")
print()

if err > 0:
    print("! notify_err > 0: kernel notify failed at least once — SSE may miss those writes.")
    print("  Check daemon.log for: GraphMemoryWrite kernel notify failed")
else:
    print("· notify_err is 0 so far (good for kernel → event bus path).")

if rw > 0 or rr > 0:
    print("! rollout_suppressed_* > 0: memory_rollout / AINL_MEMORY_ROLLOUT may be blocking reads/writes for this agent class.")
if tw > 0 or tr > 0:
    print("! temp_mode_suppressed_* > 0: memory_temporary_mode (manifest/env/controls) suppressed graph memory I/O.")

print()
print("Also verify (not in JSON):")
print("  - Per-agent: GET .../api/graph-memory/controls?agent_id=<uuid>  (temporary_mode, memory_enabled)")
base = os.environ.get("BASE_URL", "http://127.0.0.1:4200").rstrip("/")
print(f"  - SSE: curl -sN --max-time 45 '{base}/api/events/stream' | rg -m1 GraphMemoryWrite  (during a chat turn)")
print("  - Integration test: cargo test -p openfang-api --test api_integration_test test_kernel_events_stream_includes_graph_memory_write")
PY
