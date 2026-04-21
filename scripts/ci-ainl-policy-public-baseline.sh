#!/usr/bin/env bash
# Public Track A checks: ainl policy crates + MCP readiness unit tests (no inference-server required).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
cargo test -p ainl-contracts -p ainl-repo-intel -p ainl-context-freshness -p ainl-impact-policy --lib
cargo test -p openfang-runtime mcp_readiness --lib -- --nocapture
