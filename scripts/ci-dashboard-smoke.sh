#!/usr/bin/env bash
# Start a short-lived OpenFang daemon with a throwaway ARMARAOS_HOME and run
# scripts/verify-dashboard-smoke.sh against it. Intended for CI (Linux).
# See docs/dashboard-testing.md (section "CI: temp daemon + same smoke script").
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

OPENFANG_BIN="${OPENFANG_BIN:-$ROOT/target/release/openfang}"
if [[ ! -x "$OPENFANG_BIN" ]]; then
  echo "ERROR: $OPENFANG_BIN not found. Build with: cargo build --release -p openfang-cli"
  exit 1
fi

export ARMARAOS_HOME="${ARMARAOS_HOME:-$(mktemp -d)}"
export OPENFANG_SKIP_DAEMON_CHECK="${OPENFANG_SKIP_DAEMON_CHECK:-1}"

mkdir -p "$ARMARAOS_HOME"

"$OPENFANG_BIN" init --quick

"$OPENFANG_BIN" start --yolo &
DAEMON_PID=$!

cleanup() {
  kill "$DAEMON_PID" 2>/dev/null || true
  wait "$DAEMON_PID" 2>/dev/null || true
}
trap cleanup EXIT

for _ in $(seq 1 45); do
  if curl -sfS -m 2 "http://127.0.0.1:4200/api/health" >/dev/null; then
    break
  fi
  sleep 1
done

bash "$ROOT/scripts/verify-dashboard-smoke.sh" "http://127.0.0.1:4200"
