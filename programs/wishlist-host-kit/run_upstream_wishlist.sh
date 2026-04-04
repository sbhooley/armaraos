#!/usr/bin/env bash
# Run a numbered graph from the AI_Native_Lang (ainativelang) checkout: examples/wishlist/0N_*.ainl
# Usage:
#   export AINL_ROOT=/path/to/AI_Native_Lang
#   ./run_upstream_wishlist.sh 01
# Optional second arg: path to frame JSON (defaults to frames/0N.json next to this script).
set -eo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
N="${1:?usage: run_upstream_wishlist.sh <01-08|05b> [frame.json]}"
if [[ -n "${2:-}" ]]; then
  FRAME_JSON="$2"
elif [[ "$N" == "05b" ]]; then
  FRAME_JSON="$SCRIPT_DIR/frames/05.json"
else
  FRAME_JSON="$SCRIPT_DIR/frames/${N}.json"
fi

if [[ ! -d "${AINL_ROOT:-}" ]]; then
  echo "Set AINL_ROOT to your AI_Native_Lang (ainativelang) repository root." >&2
  exit 1
fi

graph_for_id() {
  case "$1" in
    01) echo "examples/wishlist/01_cache_and_memory.ainl" ;;
    02) echo "examples/wishlist/02_vector_semantic_search.ainl" ;;
    03) echo "examples/wishlist/03_parallel_fanout.ainl" ;;
    04) echo "examples/wishlist/04_validate_with_ext.ainl" ;;
    05) echo "examples/wishlist/05_route_then_llm_mock.ainl" ;;
    05b) echo "examples/wishlist/05b_unified_llm_offline_config.ainl" ;;
    06) echo "examples/wishlist/06_feedback_memory.ainl" ;;
    07) echo "examples/wishlist/07_parallel_http.ainl" ;;
    08) echo "examples/wishlist/08_code_review_context.ainl" ;;
    *) echo "" ;;
  esac
}

GRAPH="$(graph_for_id "$N")"
if [[ -z "$GRAPH" ]]; then
  echo "Unknown id: $N (use 01-08 or 05b)" >&2
  exit 1
fi

TMP="${TMPDIR:-/tmp}"
export AINL_CACHE_JSON="${AINL_CACHE_JSON:-$TMP/ainl_wishlist_cache.json}"
export AINL_MEMORY_DB="${AINL_MEMORY_DB:-$TMP/ainl_wishlist_mem.sqlite3}"
export AINL_VECTOR_MEMORY_PATH="${AINL_VECTOR_MEMORY_PATH:-$TMP/ainl_wishlist_vm.json}"

cd "$AINL_ROOT"

ARGS=(python -m cli.main run "$GRAPH" --json)
case "$N" in
  04)
    export AINL_EXT_ALLOW_EXEC=1
    ARGS+=(--enable-adapter ext)
    ;;
  05)
    export AINL_ENABLE_LLM_QUERY=true
    export AINL_LLM_QUERY_MOCK=true
    ARGS+=(--enable-adapter llm_query)
    ;;
  05b)
    ARGS+=(--config "$AINL_ROOT/examples/wishlist/fixtures/llm_offline.yaml")
    ;;
  07)
    ARGS+=(--enable-adapter http)
    ;;
  08)
    ARGS+=(--enable-adapter code_context)
    ;;
esac

if [[ -f "$FRAME_JSON" ]]; then
  FRAME_RAW="$(tr -d '\n\r' <"$FRAME_JSON" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')"
  if [[ "$FRAME_RAW" == "{}" ]] || [[ -z "$FRAME_RAW" ]]; then
    "${ARGS[@]}"
  else
    "${ARGS[@]}" --frame "$FRAME_RAW"
  fi
else
  "${ARGS[@]}"
fi
