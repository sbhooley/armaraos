//! Skills workspace export: `.learnings/` → `memory/YYYY-MM-DD.md` + `.pipeline-state.json`.
//! Mirrors the shell script in the OpenClaw workspace repo (cross-platform).

use chrono::{Local, Utc};
use openfang_types::config::{default_skills_workspace_path, KernelConfig};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

const STATUS_PENDING: &str = "**Status**: pending";

/// Result of [`export_learnings_digest`].
#[derive(Debug, Clone)]
pub struct ExportSummary {
    /// Whether a new digest block was appended to the daily file.
    pub digest_appended: bool,
    pub daily_path: PathBuf,
}

fn env_workspace_path(name: &str) -> Option<PathBuf> {
    std::env::var(name).ok().and_then(|p| {
        let t = p.trim();
        if t.is_empty() {
            None
        } else {
            Some(PathBuf::from(t))
        }
    })
}

/// Resolve workspace root: `OPENCLAW_WORKSPACE` → `ARMARAOS_SKILLS_WORKSPACE` →
/// `config.openclaw_workspace.workspace_path` → [`default_skills_workspace_path`].
pub fn resolve_openclaw_workspace_root(config: &KernelConfig) -> PathBuf {
    if let Some(p) = env_workspace_path("OPENCLAW_WORKSPACE") {
        return p;
    }
    if let Some(p) = env_workspace_path("ARMARAOS_SKILLS_WORKSPACE") {
        return p;
    }
    config
        .openclaw_workspace
        .workspace_path
        .clone()
        .unwrap_or_else(default_skills_workspace_path)
}

fn count_pending(path: &Path) -> u32 {
    let Ok(content) = std::fs::read_to_string(path) else {
        return 0;
    };
    content.matches(STATUS_PENDING).count() as u32
}

fn tail_lines(path: &Path, max: usize) -> String {
    let Ok(content) = std::fs::read_to_string(path) else {
        return String::new();
    };
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(max);
    lines[start..].join("\n")
}

/// Sum `pending.learnings + pending.errors + pending.features` from `.pipeline-state.json` if present.
pub fn read_pipeline_pending_total(root: &Path) -> Option<u32> {
    let path = root.join(".learnings").join(".pipeline-state.json");
    let data = std::fs::read_to_string(&path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&data).ok()?;
    let p = v.get("pending")?;
    let l = p.get("learnings")?.as_u64()? as u32;
    let e = p.get("errors")?.as_u64()? as u32;
    let f = p.get("features")?.as_u64()? as u32;
    Some(l.saturating_add(e).saturating_add(f))
}

/// Run export on kernel startup when configured (best-effort; logs only).
pub fn run_startup_export_if_configured(config: &KernelConfig) {
    if !config.openclaw_workspace.enabled {
        return;
    }
    if !config.openclaw_workspace.run_export_on_startup {
        return;
    }
    let root = resolve_openclaw_workspace_root(config);
    if !root.exists() {
        tracing::debug!(
            path = %root.display(),
            "Skills workspace path does not exist; skipping learnings export"
        );
        return;
    }
    match export_learnings_digest(&root) {
        Ok(s) => {
            tracing::info!(
                appended = s.digest_appended,
                daily = %s.daily_path.display(),
                "Skills workspace learnings export finished"
            );
        }
        Err(e) => tracing::warn!(error = %e, "Skills workspace learnings export failed"),
    }
}

/// Append digest to `memory/YYYY-MM-DD.md` (idempotent per day) and refresh pipeline state JSON.
pub fn export_learnings_digest(workspace_root: &Path) -> Result<ExportSummary, String> {
    std::fs::create_dir_all(workspace_root).map_err(|e| format!("create workspace root: {e}"))?;
    let learn_dir = workspace_root.join(".learnings");
    let mem_dir = workspace_root.join("memory");
    std::fs::create_dir_all(&learn_dir).map_err(|e| format!("create .learnings: {e}"))?;
    std::fs::create_dir_all(&mem_dir).map_err(|e| format!("create memory dir: {e}"))?;

    let day = Local::now().format("%Y-%m-%d").to_string();
    let daily_path = mem_dir.join(format!("{day}.md"));
    let marker = format!("<!-- openclaw-learnings-digest {day} -->");

    let l = count_pending(&learn_dir.join("LEARNINGS.md"));
    let e = count_pending(&learn_dir.join("ERRORS.md"));
    let f = count_pending(&learn_dir.join("FEATURE_REQUESTS.md"));

    let iso = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    write_pipeline_state(
        &learn_dir.join(".pipeline-state.json"),
        workspace_root,
        &iso,
        &daily_path,
        l,
        e,
        f,
    )?;

    let digest_appended = if daily_path.exists() {
        let existing = std::fs::read_to_string(&daily_path).unwrap_or_default();
        if existing.contains(&marker) {
            false
        } else {
            append_digest(&daily_path, &marker, &iso, l, e, f, &learn_dir)?;
            true
        }
    } else {
        append_digest(&daily_path, &marker, &iso, l, e, f, &learn_dir)?;
        true
    };

    Ok(ExportSummary {
        digest_appended,
        daily_path,
    })
}

fn write_pipeline_state(
    path: &Path,
    workspace_root: &Path,
    updated_at: &str,
    daily_path: &Path,
    learnings: u32,
    errors: u32,
    features: u32,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create .learnings: {e}"))?;
    }
    let v = serde_json::json!({
        "schema": "openclaw-pipeline-state-v1",
        "updated_at": updated_at,
        "workspace_root": workspace_root.to_string_lossy(),
        "pending": {
            "learnings": learnings,
            "errors": errors,
            "features": features,
        },
        "last_daily_path": daily_path.to_string_lossy(),
    });
    let s = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    std::fs::write(path, format!("{s}\n")).map_err(|e| e.to_string())
}

fn append_digest(
    daily_path: &Path,
    marker: &str,
    iso: &str,
    l: u32,
    e: u32,
    f: u32,
    learn_dir: &Path,
) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(daily_path)
        .map_err(|e| format!("open daily memory: {e}"))?;

    writeln!(file).map_err(|e| e.to_string())?;
    writeln!(file, "{marker}").map_err(|e| e.to_string())?;
    writeln!(file, "## Skills workspace / .learnings digest (auto {iso})")
        .map_err(|e| e.to_string())?;
    writeln!(file).map_err(|e| e.to_string())?;
    writeln!(
        file,
        "Pending counts — learnings: {l}, errors: {e}, features: {f}."
    )
    .map_err(|e| e.to_string())?;
    writeln!(file).map_err(|e| e.to_string())?;
    writeln!(
        file,
        "See `.learnings/` and promotion rules in your workspace `INTEGRATION.md` (if present)."
    )
    .map_err(|e| e.to_string())?;
    writeln!(file).map_err(|e| e.to_string())?;

    for (_name, fname) in [
        ("LEARNINGS", "LEARNINGS.md"),
        ("ERRORS", "ERRORS.md"),
        ("FEATURE_REQUESTS", "FEATURE_REQUESTS.md"),
    ] {
        let p = learn_dir.join(fname);
        if p.exists() {
            writeln!(file, "### {fname} (excerpt)").map_err(|e| e.to_string())?;
            writeln!(file).map_err(|e| e.to_string())?;
            writeln!(file, "```").map_err(|e| e.to_string())?;
            let body = tail_lines(&p, 80);
            write!(file, "{body}").map_err(|e| e.to_string())?;
            writeln!(file).map_err(|e| e.to_string())?;
            writeln!(file, "```").map_err(|e| e.to_string())?;
            writeln!(file).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn export_creates_state_and_daily() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let learn = root.join(".learnings");
        std::fs::create_dir_all(&learn).unwrap();
        std::fs::write(
            learn.join("LEARNINGS.md"),
            "# L\n\n## [LRN-20260101-001] x\n\n**Status**: pending\n",
        )
        .unwrap();

        let r = export_learnings_digest(root).unwrap();
        assert!(r.daily_path.exists());
        assert!(root
            .join(".learnings")
            .join(".pipeline-state.json")
            .exists());
        let t = read_pipeline_pending_total(root).unwrap();
        assert!(t >= 1);
    }
}
