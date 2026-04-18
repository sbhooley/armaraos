//! Prometheus-friendly counters for native infer + deterministic planner (`ARMARA_NATIVE_INFER_URL` path).
//!
//! Per-step counters use a fixed-capacity sharded table keyed by tool name so they stay
//! allocation-free at runtime. Tool names are truncated to 64 bytes and stored in a
//! `RwLock<HashMap>` initialised lazily on first use.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

static NATIVE_INFER_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static NATIVE_INFER_HTTP_ERRORS: AtomicU64 = AtomicU64::new(0);
static PLAN_INVALID_OR_EMPTY_STRUCTURED: AtomicU64 = AtomicU64::new(0);
static PLAN_DESERIALIZE_FAILED: AtomicU64 = AtomicU64::new(0);
static PLAN_EXEC_COMPLETED: AtomicU64 = AtomicU64::new(0);
static PLAN_EXEC_LEGACY_FALLBACK: AtomicU64 = AtomicU64::new(0);
static PLAN_EXEC_ERROR: AtomicU64 = AtomicU64::new(0);
/// Path B: AINL runtime `GraphPatchAdapter` dispatches (ainl_runtime_bridge).
static GRAPH_PATCH_ADAPTER_DISPATCHES: AtomicU64 = AtomicU64::new(0);
static GRAPH_PATCH_ADAPTER_ERRORS: AtomicU64 = AtomicU64::new(0);
/// `LocalPatch` replan call count (within PlanExecutor, any call to `ni.infer` for repair).
static LOCAL_PATCH_REPLAN_CALLS: AtomicU64 = AtomicU64::new(0);

// --- Per-step counters (keyed by tool name) ---

fn tool_key(tool: &str) -> String {
    let t = tool.trim();
    if t.len() <= 64 {
        t.to_string()
    } else {
        t[..64].to_string()
    }
}

static STEP_DISPATCHED: std::sync::OnceLock<RwLock<HashMap<String, AtomicU64>>> =
    std::sync::OnceLock::new();
static STEP_SUCCESS: std::sync::OnceLock<RwLock<HashMap<String, AtomicU64>>> =
    std::sync::OnceLock::new();
static STEP_ERROR: std::sync::OnceLock<RwLock<HashMap<String, AtomicU64>>> =
    std::sync::OnceLock::new();
static STEP_OPTIONAL_SKIPPED: std::sync::OnceLock<RwLock<HashMap<String, AtomicU64>>> =
    std::sync::OnceLock::new();

fn inc_tool_counter(
    cell: &std::sync::OnceLock<RwLock<HashMap<String, AtomicU64>>>,
    tool: &str,
) {
    let map = cell.get_or_init(|| RwLock::new(HashMap::new()));
    let key = tool_key(tool);
    // Fast path: counter already exists.
    if let Ok(r) = map.read() {
        if let Some(c) = r.get(&key) {
            c.fetch_add(1, Ordering::Relaxed);
            return;
        }
    }
    // Slow path: insert then increment.
    if let Ok(mut w) = map.write() {
        w.entry(key).or_insert_with(|| AtomicU64::new(0)).fetch_add(1, Ordering::Relaxed);
    }
}

fn snapshot_tool_counter(
    cell: &std::sync::OnceLock<RwLock<HashMap<String, AtomicU64>>>,
) -> Vec<(String, u64)> {
    let map = match cell.get() {
        Some(m) => m,
        None => return vec![],
    };
    if let Ok(r) = map.read() {
        r.iter()
            .map(|(k, v)| (k.clone(), v.load(Ordering::Relaxed)))
            .collect()
    } else {
        vec![]
    }
}

#[inline]
pub fn record_native_infer_attempt() {
    NATIVE_INFER_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn record_native_infer_http_error() {
    NATIVE_INFER_HTTP_ERRORS.fetch_add(1, Ordering::Relaxed);
}

/// Infer returned `planner_invalid_plan` or no usable structured plan for the planner branch.
#[inline]
pub fn record_plan_invalid_or_unstructured() {
    PLAN_INVALID_OR_EMPTY_STRUCTURED.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn record_plan_body_deserialize_failed() {
    PLAN_DESERIALIZE_FAILED.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn record_plan_executor_completed() {
    PLAN_EXEC_COMPLETED.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn record_plan_executor_legacy_fallback() {
    PLAN_EXEC_LEGACY_FALLBACK.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn record_plan_executor_error() {
    PLAN_EXEC_ERROR.fetch_add(1, Ordering::Relaxed);
}

/// Path B: AINL runtime `GraphPatchAdapter` dispatched (regardless of graph_writes content).
#[inline]
pub fn record_graph_patch_adapter_dispatch() {
    GRAPH_PATCH_ADAPTER_DISPATCHES.fetch_add(1, Ordering::Relaxed);
}

/// Path B: AINL runtime `GraphPatchAdapter` dispatch returned an error.
#[inline]
pub fn record_graph_patch_adapter_error() {
    GRAPH_PATCH_ADAPTER_ERRORS.fetch_add(1, Ordering::Relaxed);
}

/// `LocalPatch` replan call fired inside `PlanExecutor` (gap-2).
#[inline]
pub fn record_local_patch_replan_call() {
    LOCAL_PATCH_REPLAN_CALLS.fetch_add(1, Ordering::Relaxed);
}

// --- Per-step counters ---

/// A plan step was dispatched (tool call started) for the given tool.
#[inline]
pub fn record_plan_step_dispatched(tool: &str) {
    inc_tool_counter(&STEP_DISPATCHED, tool);
}

/// A plan step completed successfully for the given tool.
#[inline]
pub fn record_plan_step_success(tool: &str) {
    inc_tool_counter(&STEP_SUCCESS, tool);
}

/// A plan step failed (after all escalation tiers) for the given tool.
#[inline]
pub fn record_plan_step_error(tool: &str) {
    inc_tool_counter(&STEP_ERROR, tool);
}

/// An optional plan step was skipped on failure for the given tool.
#[inline]
pub fn record_plan_step_optional_skipped(tool: &str) {
    inc_tool_counter(&STEP_OPTIONAL_SKIPPED, tool);
}

/// Append Prometheus text lines (included in `GET /metrics` / `GET /armara/v1/metrics`).
#[must_use]
pub fn render_prometheus() -> String {
    let mut out = String::with_capacity(512);
    out.push_str("# HELP openfang_planner_native_infer_attempts_total Planner path: POST /armara/v1/infer attempts.\n");
    out.push_str("# TYPE openfang_planner_native_infer_attempts_total counter\n");
    out.push_str(&format!(
        "openfang_planner_native_infer_attempts_total {}\n",
        NATIVE_INFER_ATTEMPTS.load(Ordering::Relaxed)
    ));
    out.push_str("# HELP openfang_planner_native_infer_http_errors_total Planner path: native infer HTTP/parse failures.\n");
    out.push_str("# TYPE openfang_planner_native_infer_http_errors_total counter\n");
    out.push_str(&format!(
        "openfang_planner_native_infer_http_errors_total {}\n",
        NATIVE_INFER_HTTP_ERRORS.load(Ordering::Relaxed)
    ));
    out.push_str("# HELP openfang_planner_invalid_or_unstructured_total Infer returned planner_invalid_plan or missing structured plan.\n");
    out.push_str("# TYPE openfang_planner_invalid_or_unstructured_total counter\n");
    out.push_str(&format!(
        "openfang_planner_invalid_or_unstructured_total {}\n",
        PLAN_INVALID_OR_EMPTY_STRUCTURED.load(Ordering::Relaxed)
    ));
    out.push_str("# HELP openfang_planner_plan_deserialize_errors_total deterministic_plan JSON did not deserialize to DeterministicPlan.\n");
    out.push_str("# TYPE openfang_planner_plan_deserialize_errors_total counter\n");
    out.push_str(&format!(
        "openfang_planner_plan_deserialize_errors_total {}\n",
        PLAN_DESERIALIZE_FAILED.load(Ordering::Relaxed)
    ));
    out.push_str("# HELP openfang_planner_plan_executor_completed_total PlanExecutor finished without inner legacy fallback.\n");
    out.push_str("# TYPE openfang_planner_plan_executor_completed_total counter\n");
    out.push_str(&format!(
        "openfang_planner_plan_executor_completed_total {}\n",
        PLAN_EXEC_COMPLETED.load(Ordering::Relaxed)
    ));
    out.push_str("# HELP openfang_planner_plan_executor_legacy_fallback_total PlanExecutor set fell_back_to_legacy.\n");
    out.push_str("# TYPE openfang_planner_plan_executor_legacy_fallback_total counter\n");
    out.push_str(&format!(
        "openfang_planner_plan_executor_legacy_fallback_total {}\n",
        PLAN_EXEC_LEGACY_FALLBACK.load(Ordering::Relaxed)
    ));
    out.push_str("# HELP openfang_planner_plan_executor_errors_total PlanExecutor returned Err (legacy loop follows).\n");
    out.push_str("# TYPE openfang_planner_plan_executor_errors_total counter\n");
    out.push_str(&format!(
        "openfang_planner_plan_executor_errors_total {}\n",
        PLAN_EXEC_ERROR.load(Ordering::Relaxed)
    ));
    out.push_str("# HELP openfang_planner_graph_patch_adapter_dispatches_total Path B: ainl-runtime GraphPatchAdapter dispatch calls.\n");
    out.push_str("# TYPE openfang_planner_graph_patch_adapter_dispatches_total counter\n");
    out.push_str(&format!(
        "openfang_planner_graph_patch_adapter_dispatches_total {}\n",
        GRAPH_PATCH_ADAPTER_DISPATCHES.load(Ordering::Relaxed)
    ));
    out.push_str("# HELP openfang_planner_graph_patch_adapter_errors_total Path B: ainl-runtime GraphPatchAdapter dispatch errors.\n");
    out.push_str("# TYPE openfang_planner_graph_patch_adapter_errors_total counter\n");
    out.push_str(&format!(
        "openfang_planner_graph_patch_adapter_errors_total {}\n",
        GRAPH_PATCH_ADAPTER_ERRORS.load(Ordering::Relaxed)
    ));
    out.push_str("# HELP openfang_planner_local_patch_replan_calls_total LocalPatch ni.infer calls fired inside PlanExecutor.\n");
    out.push_str("# TYPE openfang_planner_local_patch_replan_calls_total counter\n");
    out.push_str(&format!(
        "openfang_planner_local_patch_replan_calls_total {}\n",
        LOCAL_PATCH_REPLAN_CALLS.load(Ordering::Relaxed)
    ));

    // Per-step counters (labelled by tool name).
    fn emit_labeled(out: &mut String, metric: &str, help: &str, rows: Vec<(String, u64)>) {
        if rows.is_empty() {
            return;
        }
        out.push_str(&format!("# HELP {metric} {help}\n"));
        out.push_str(&format!("# TYPE {metric} counter\n"));
        for (tool, v) in rows {
            out.push_str(&format!("{metric}{{tool=\"{tool}\"}} {v}\n"));
        }
    }

    emit_labeled(
        &mut out,
        "openfang_planner_step_dispatched_total",
        "Plan step dispatched (tool call started) by tool.",
        snapshot_tool_counter(&STEP_DISPATCHED),
    );
    emit_labeled(
        &mut out,
        "openfang_planner_step_success_total",
        "Plan step completed successfully by tool.",
        snapshot_tool_counter(&STEP_SUCCESS),
    );
    emit_labeled(
        &mut out,
        "openfang_planner_step_error_total",
        "Plan step failed (after all escalation tiers) by tool.",
        snapshot_tool_counter(&STEP_ERROR),
    );
    emit_labeled(
        &mut out,
        "openfang_planner_step_optional_skipped_total",
        "Optional plan step skipped on failure by tool.",
        snapshot_tool_counter(&STEP_OPTIONAL_SKIPPED),
    );

    out.push('\n');
    out
}
