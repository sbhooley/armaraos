//! Counters for `shell_exec` argv guards (path / PID). Exposed on `/api/status` JSON and Prometheus.

use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};

static PATH_ENFORCE_DENIED: AtomicU64 = AtomicU64::new(0);
static PATH_WARN: AtomicU64 = AtomicU64::new(0);
static PID_ENFORCE_DENIED: AtomicU64 = AtomicU64::new(0);

#[inline]
pub fn inc_path_enforce_denied() {
    PATH_ENFORCE_DENIED.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn inc_path_warn() {
    PATH_WARN.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn inc_pid_enforce_denied() {
    PID_ENFORCE_DENIED.fetch_add(1, Ordering::Relaxed);
}

#[must_use]
pub fn shell_guard_metrics_snapshot() -> serde_json::Value {
    json!({
        "shell_path_guard_enforce_denies_total": PATH_ENFORCE_DENIED.load(Ordering::Relaxed),
        "shell_path_guard_warn_events_total": PATH_WARN.load(Ordering::Relaxed),
        "shell_pid_guard_enforce_denies_total": PID_ENFORCE_DENIED.load(Ordering::Relaxed),
    })
}

/// Append Prometheus counter lines for shell argv guards.
#[must_use]
pub fn render_prometheus_counters() -> String {
    let pe = PATH_ENFORCE_DENIED.load(Ordering::Relaxed);
    let pw = PATH_WARN.load(Ordering::Relaxed);
    let pid = PID_ENFORCE_DENIED.load(Ordering::Relaxed);
    let mut s = String::with_capacity(512);
    s.push_str("# HELP openfang_shell_path_guard_enforce_denies_total shell_exec path guard enforce-mode denials.\n");
    s.push_str("# TYPE openfang_shell_path_guard_enforce_denies_total counter\n");
    s.push_str(&format!(
        "openfang_shell_path_guard_enforce_denies_total {pe}\n"
    ));
    s.push_str("# HELP openfang_shell_path_guard_warn_events_total shell_exec path guard warn-mode violations (still executed).\n");
    s.push_str("# TYPE openfang_shell_path_guard_warn_events_total counter\n");
    s.push_str(&format!(
        "openfang_shell_path_guard_warn_events_total {pw}\n"
    ));
    s.push_str("# HELP openfang_shell_pid_guard_enforce_denies_total shell_exec PID guard enforce-mode denials.\n");
    s.push_str("# TYPE openfang_shell_pid_guard_enforce_denies_total counter\n");
    s.push_str(&format!(
        "openfang_shell_pid_guard_enforce_denies_total {pid}\n\n"
    ));
    s
}
