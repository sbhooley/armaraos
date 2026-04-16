//! Cached snapshot of local AINL toolchain versions for canonical context injection.
//!
//! Runs `ainl --version` and `pip show ainativelang` (best-effort) with a short TTL so
//! agents get factual host numbers without a web round-trip or per-message shell spam.

use serde::Serialize;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long to reuse a snapshot before refreshing.
const TTL: Duration = Duration::from_secs(60);

static CACHE: Mutex<Option<(Instant, String)>> = Mutex::new(None);

/// Returns a compact, human-readable block for injection into the canonical-context message.
pub fn host_ainl_snapshot_cached() -> String {
    let now = Instant::now();
    if let Ok(mut guard) = CACHE.lock() {
        if let Some((t, ref s)) = *guard {
            if now.duration_since(t) < TTL {
                return s.clone();
            }
        }
        let s = build_snapshot();
        *guard = Some((now, s.clone()));
        return s;
    }
    build_snapshot()
}

fn build_snapshot() -> String {
    let cli = ainl_cli_version();
    let pip = pip_show_ainativelang();
    format!(
        "[Host AINL — this machine]\n\
         - **ainl CLI** (`ainl --version`): {cli}\n\
         - **pip package** `ainativelang` (filtered `pip show`):\n{pip}"
    )
}

/// Local `ainl` CLI + pip package snapshot for dashboards (`GET /api/ainl/runtime-version`).
#[derive(Debug, Clone, Serialize)]
pub struct HostAinlToolchainProbe {
    /// Raw `ainl --version` stdout (or error text).
    pub ainl_cli_line: String,
    /// Parsed `Version:` from `pip show ainativelang`, when available.
    pub pip_version: Option<String>,
    /// Filtered `pip show` excerpt for display.
    pub pip_excerpt: String,
}

/// Best-effort probe of the host AINL toolchain (blocking: runs subprocesses).
#[must_use]
pub fn probe_host_ainl_toolchain() -> HostAinlToolchainProbe {
    let pip_stdout = pip_show_ainativelang_stdout();
    let pip_version = pip_stdout
        .as_deref()
        .and_then(parse_pip_version_line);
    let pip_excerpt = pip_stdout
        .as_ref()
        .map(|raw| cap_str(&filter_pip_show(raw), 900))
        .unwrap_or_else(|| "not installed or pip not found (try: python3 -m pip show ainativelang)".into());
    HostAinlToolchainProbe {
        ainl_cli_line: ainl_cli_version(),
        pip_version,
        pip_excerpt,
    }
}

fn parse_pip_version_line(stdout: &str) -> Option<String> {
    for line in stdout.lines() {
        if let Some(rest) = line.trim().strip_prefix("Version:") {
            let v = rest.trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn pip_show_ainativelang_stdout() -> Option<String> {
    let attempts: &[(&str, &[&str])] = &[
        ("python3", &["-m", "pip", "show", "ainativelang"]),
        ("python", &["-m", "pip", "show", "ainativelang"]),
        ("pip3", &["show", "ainativelang"]),
        ("pip", &["show", "ainativelang"]),
    ];
    for (cmd, args) in attempts {
        if let Ok(o) = Command::new(cmd).args(*args).output() {
            if o.status.success() {
                return Some(String::from_utf8_lossy(&o.stdout).to_string());
            }
        }
    }
    None
}

fn ainl_cli_version() -> String {
    match Command::new("ainl").arg("--version").output() {
        Ok(o) if o.status.success() => {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if s.is_empty() {
                "(empty stdout)".into()
            } else {
                s
            }
        }
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
            if err.is_empty() {
                format!("(exit {:?})", o.status.code())
            } else {
                format!("(failed) {err}")
            }
        }
        Err(e) => format!("not on PATH ({e})"),
    }
}

fn pip_show_ainativelang() -> String {
    pip_show_ainativelang_stdout()
        .map(|raw| cap_str(&filter_pip_show(&raw), 900))
        .unwrap_or_else(|| "not installed or pip not found (try: python3 -m pip show ainativelang)".into())
}

fn filter_pip_show(stdout: &str) -> String {
    let mut lines: Vec<&str> = Vec::new();
    for line in stdout.lines() {
        let t = line.trim();
        if t.starts_with("Name:")
            || t.starts_with("Version:")
            || t.starts_with("Location:")
            || t.starts_with("Summary:")
            || t.starts_with("Home-page:")
        {
            lines.push(t);
        }
    }
    if lines.is_empty() {
        stdout.trim().to_string()
    } else {
        lines.join("\n")
    }
}

fn cap_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let end = s
            .char_indices()
            .nth(max_chars)
            .map(|(i, _)| i)
            .unwrap_or(s.len());
        format!("{}...", &s[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_non_empty() {
        let s = host_ainl_snapshot_cached();
        assert!(s.contains("[Host AINL"));
        assert!(s.contains("ainl CLI"));
        assert!(s.contains("pip package"));
    }

    #[test]
    fn filter_pip_show_extracts_key_lines() {
        let raw =
            "Name: ainativelang\nVersion: 1.4.1\nLocation: /tmp/x\nSummary: hi\n\nOther: junk\n";
        let f = filter_pip_show(raw);
        assert!(f.contains("Version: 1.4.1"));
        assert!(!f.contains("Other:"));
    }
}
