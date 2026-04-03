//! AINL library path helpers and curated cron registration.

use crate::OpenFangKernel;
use openfang_types::agent::AgentId;
use openfang_types::learning_frame::LearningFrameV1;
use openfang_types::scheduler::{CronAction, CronDelivery, CronJob, CronJobId, CronSchedule};
use serde::Deserialize;
use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Written beside the mirrored tree at `~/.armaraos/ainl-library/.armaraos-ainl-library.json` when the desktop sync runs.
pub const LIBRARY_SYNC_META_FILENAME: &str = ".armaraos-ainl-library.json";

/// Embedded catalog of optional cron jobs pointing at synced upstream examples.
pub static CURATED_AINL_CRON_JSON: &str = include_str!("curated_ainl_cron.json");

#[derive(Debug, Deserialize)]
struct CuratedCronEntry {
    name: String,
    /// Path under `ainl-library` (e.g. `armaraos-programs/armaraos_health_ping/armaraos_health_ping.ainl`
    /// or `examples/compact/hello_compact.ainl` after upstream sync).
    relative_path: String,
    cron_expr: String,
    #[serde(default)]
    timeout_secs: Option<u64>,
    /// Defaults to false so jobs are registered but do not run until enabled in the scheduler UI.
    #[serde(default)]
    enabled: bool,
    /// When true, uses `ainl run --json` for structured stdout.
    #[serde(default)]
    json_output: bool,
    /// When set, passed as `ainl run --frame-json` (must validate as [`LearningFrameV1`]).
    #[serde(default)]
    frame: Option<serde_json::Value>,
}

fn is_ainl_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e == "ainl" || e == "lang")
        .unwrap_or(false)
}

/// Recursively list `.ainl` / `.lang` files under `root` (skips missing dirs).
pub fn walk_ainl_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    walk_ainl_files_inner(root, &mut out)?;
    out.sort();
    Ok(out)
}

fn walk_ainl_files_inner(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let read = std::fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))?;
    for ent in read {
        let ent = ent.map_err(|e| format!("read_dir entry: {e}"))?;
        let p = ent.path();
        if p.is_dir() {
            walk_ainl_files_inner(&p, out)?;
        } else if is_ainl_extension(&p) {
            out.push(p);
        }
    }
    Ok(())
}

/// Resolve `program_path` to an existing file under `<home_dir>/ainl-library`.
pub fn resolve_program_under_ainl_library(
    home_dir: &Path,
    program_path: &str,
) -> Result<PathBuf, String> {
    let root = home_dir.join("ainl-library");
    let root_canon = root.canonicalize().map_err(|_| {
        format!(
            "ainl-library directory missing or inaccessible: {}",
            root.display()
        )
    })?;

    let p = Path::new(program_path);
    let full = if p.is_absolute() {
        p.to_path_buf()
    } else {
        root_canon.join(p)
    };

    let full_canon = full
        .canonicalize()
        .map_err(|e| format!("AINL program not found ({e}): {}", full.display()))?;

    if !full_canon.starts_with(&root_canon) {
        return Err("AINL program path escapes ainl-library".into());
    }
    if !full_canon.is_file() {
        return Err(format!("not a file: {}", full_canon.display()));
    }
    Ok(full_canon)
}

/// Resolve working directory for `ainl run` (defaults to `ainl-library` root).
pub fn resolve_cwd_under_ainl_library(
    home_dir: &Path,
    cwd_opt: &Option<String>,
) -> Result<PathBuf, String> {
    let root = home_dir.join("ainl-library");
    let root_canon = root.canonicalize().map_err(|_| {
        format!(
            "ainl-library directory missing or inaccessible: {}",
            root.display()
        )
    })?;

    match cwd_opt {
        None => Ok(root_canon),
        Some(cwd) => {
            let p = Path::new(cwd);
            let full = if p.is_absolute() {
                p.to_path_buf()
            } else {
                root_canon.join(p)
            };
            let canon = full
                .canonicalize()
                .map_err(|e| format!("cwd invalid ({e}): {}", full.display()))?;
            if !canon.starts_with(&root_canon) {
                return Err("cwd escapes ainl-library".into());
            }
            if !canon.is_dir() {
                return Err(format!("cwd is not a directory: {}", canon.display()));
            }
            Ok(canon)
        }
    }
}

/// Written by the desktop after a successful internal-venv AINL install (`openfang-desktop` ainl bootstrap).
/// One line: absolute path to the real `ainl` executable (so the daemon finds it without relying on GUI PATH).
pub const AINL_BIN_CACHE_FILENAME: &str = ".armaraos-ainl-bin";

/// First `# ...` comment line from the start of a source file (up to 2048 bytes), for library browsing hints.
pub fn ainl_source_first_hint(path: &Path) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = [0u8; 2048];
    let n = f.read(&mut buf).ok()?;
    let s = std::str::from_utf8(&buf[..n]).unwrap_or("");
    for line in s.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix('#') {
            let hint = rest.trim();
            if !hint.is_empty() {
                return Some(hint.chars().take(200).collect());
            }
        }
    }
    None
}

/// Optional JSON metadata from the last upstream sync (commit, branch, etc.).
pub fn read_ainl_library_sync_metadata(home_dir: &Path) -> Option<serde_json::Value> {
    let p = home_dir
        .join("ainl-library")
        .join(LIBRARY_SYNC_META_FILENAME);
    let f = std::fs::File::open(&p).ok()?;
    serde_json::from_reader(f).ok()
}

/// Pick `ARMARAOS_AINL_BIN`, per-job override, [`AINL_BIN_CACHE_FILENAME`] under `home_dir`, then
/// `home_dir/bin/ainl` (Unix) or `home_dir/bin/ainl.exe` (Windows) when present, else `ainl` for `PATH` lookup.
pub fn resolve_ainl_binary(home_dir: &Path, override_opt: &Option<String>) -> String {
    if let Ok(bin) = std::env::var("ARMARAOS_AINL_BIN") {
        if !bin.is_empty() {
            return bin;
        }
    }
    if let Some(b) = override_opt {
        if !b.is_empty() {
            return b.clone();
        }
    }
    let cache = home_dir.join(AINL_BIN_CACHE_FILENAME);
    if let Ok(s) = std::fs::read_to_string(&cache) {
        let line = s.lines().next().unwrap_or("").trim();
        if !line.is_empty() && Path::new(line).is_file() {
            return line.to_string();
        }
    }
    #[cfg(unix)]
    {
        let p = home_dir.join("bin").join("ainl");
        if p.is_file() {
            return p.display().to_string();
        }
    }
    #[cfg(windows)]
    {
        let p = home_dir.join("bin").join("ainl.exe");
        if p.is_file() {
            return p.display().to_string();
        }
    }
    "ainl".to_string()
}

/// Register curated jobs idempotently (skips missing files and existing names).
pub fn register_curated_ainl_cron_jobs(kernel: &OpenFangKernel) -> Result<usize, String> {
    if std::env::var("ARMARAOS_DISABLE_CURATED_AINL_CRON")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        tracing::debug!(
            "Curated AINL cron registration disabled (ARMARAOS_DISABLE_CURATED_AINL_CRON)"
        );
        return Ok(0);
    }

    let entries: Vec<CuratedCronEntry> =
        serde_json::from_str(CURATED_AINL_CRON_JSON).map_err(|e| format!("curated JSON: {e}"))?;

    let agent_id: AgentId = kernel
        .registry
        .list()
        .first()
        .map(|e| e.id)
        .ok_or_else(|| "no agents; cannot attach curated AINL cron jobs".to_string())?;

    let lib = kernel.config.home_dir.join("ainl-library");
    let mut existing_names: HashSet<String> = kernel
        .cron_scheduler
        .list_all_jobs()
        .into_iter()
        .map(|j| j.name)
        .collect();

    let mut added = 0usize;
    for entry in entries {
        if existing_names.contains(&entry.name) {
            continue;
        }
        let rel = entry.relative_path.trim_start_matches('/');
        let file_path = lib.join(rel);
        if !file_path.is_file() {
            tracing::debug!(
                path = %file_path.display(),
                "Skipping curated AINL cron: file not present yet"
            );
            continue;
        }

        let frame_opt: Option<serde_json::Value> =
            match &entry.frame {
                None | Some(serde_json::Value::Null) => None,
                Some(v) => {
                    let f: LearningFrameV1 = serde_json::from_value(v.clone())
                        .map_err(|e| format!("curated job {} frame JSON: {e}", entry.name))?;
                    f.validate_defaults()
                        .map_err(|e| format!("curated job {} frame invalid: {e}", entry.name))?;
                    Some(f.to_cron_json_value().map_err(|e| {
                        format!("curated job {} frame re-serialize: {e}", entry.name)
                    })?)
                }
            };

        let job = CronJob {
            id: CronJobId::new(),
            agent_id,
            name: entry.name.clone(),
            enabled: entry.enabled,
            schedule: CronSchedule::Cron {
                expr: entry.cron_expr.clone(),
                tz: None,
            },
            action: CronAction::AinlRun {
                program_path: rel.to_string(),
                cwd: None,
                ainl_binary: None,
                timeout_secs: entry.timeout_secs,
                json_output: entry.json_output,
                frame: frame_opt,
            },
            delivery: CronDelivery::None,
            created_at: chrono::Utc::now(),
            last_run: None,
            next_run: None,
        };

        match kernel.cron_scheduler.add_job(job, false) {
            Ok(_) => {
                added += 1;
                existing_names.insert(entry.name.clone());
                tracing::info!(name = %entry.name, "Registered curated AINL cron job");
            }
            Err(e) => {
                tracing::warn!(name = %entry.name, error = %e, "Failed to add curated AINL cron job");
            }
        }
    }

    Ok(added)
}
