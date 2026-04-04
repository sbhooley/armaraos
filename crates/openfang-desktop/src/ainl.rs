//! AINL bundling bootstrap (Option A).
//!
//! Creates an internal venv under the app data directory and installs AINL by:
//! - **Preferred:** bundled `ainativelang-*-py3-none-any.whl` under `resources/ainl/` (offline).
//! - **Fallback:** `pip install` of `ainativelang[mcp]` from PyPI (needs network) when no wheel is bundled.
//!
//! Python is resolved automatically: **bundled** portable CPython under `resources/python/<triple>/` (from
//! `xtask bundle-portable-python`) is the zero-install path for release builds; then `ARMARAOS_PYTHON`, then
//! versioned interpreters (`python3.12` … `python3.10`, then `python3` / `python`; on Windows `py -3.12` … `py -3.10`,
//! then `py -3`). The desktop app runs `ensure_ainl_installed` on startup (unless `ARMARAOS_AINL_AUTO_BOOTSTRAP=0`).
//! **PyPI `ainativelang` 1.3+ requires Python ≥3.10** — older interpreters are skipped and existing venvs using
//! older Pythons are removed on bootstrap/upgrade so a new venv can be created.
//! Override PyPI spec with `ARMARAOS_AINL_PYPI_SPEC` (default [`DEFAULT_AINL_PYPI_SPEC`]).
//!
//! **`ARMARAOS_AINL_AUTO_PIP_UPGRADE`**: when unset or any value other than `0` / `false`, the healthy
//! venv path runs `pip install --upgrade` for [`pypi_spec`] at most once every 7 days (see
//! `.last_auto_pip_upgrade_unix` under the app data `ainl/` dir). Set to `0` to disable.
//!
//! Designed to be called from Tauri commands and/or on app startup.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use openfang_kernel::ainl_library::AINL_BIN_CACHE_FILENAME;
use openfang_kernel::config::openfang_home;
use tauri::{AppHandle, Manager};
use tracing::warn;

/// Default spec for online install; override at runtime with `ARMARAOS_AINL_PYPI_SPEC`.
const DEFAULT_AINL_PYPI_SPEC: &str = "ainativelang[mcp]>=1.3.0,<2";

/// Throttle for [`maybe_auto_upgrade_ainl_pip`] (marker: `.last_auto_pip_upgrade_unix` in app `ainl/`).
const AUTO_PIP_UPGRADE_INTERVAL_SECS: u64 = 7 * 24 * 3600;

/// `ainativelang` on PyPI declares `Requires-Python >=3.10` for current 1.3.x lines.
const MIN_PYTHON_MAJOR: u32 = 3;
const MIN_PYTHON_MINOR: u32 = 10;

const INSTALL_SOURCE_FILE: &str = "install_source.txt";

#[derive(Debug, serde::Serialize)]
pub struct AinlStatus {
    pub ok: bool,
    pub venv_exists: bool,
    pub ainl_ok: bool,
    pub ainl_mcp_ok: bool,
    pub wheel_found: bool,
    pub wheel_path: Option<String>,
    pub detail: String,
    /// After a healthy venv, desktop runs the same MCP + `ainl-run` steps as `ainl install armaraos`
    /// (see AI_Native_Lang `tooling/mcp_host_install.py`), using this venv on `PATH` so config points at bundled binaries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub armaraos_host_ok: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub armaraos_host_detail: Option<String>,
    /// `bundled_wheel` or `pypi` after a successful install (read from app data).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_source: Option<String>,
    /// True when `resources/python/<target-triple>/python/...` is present (release desktop bundles).
    #[serde(default)]
    pub portable_python_bundled: bool,
    /// e.g. `"3.12"` for the internal venv interpreter, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub venv_python_version: Option<String>,
    /// False when the venv uses Python below 3.10 (PyPI AINL 1.3+ will not install).
    #[serde(default)]
    pub venv_python_meets_ainl: bool,
    /// Sync of `demo/`, `examples/`, `intelligence/` from GitHub `sbhooley/ainativelang` (desktop only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library_sync_ok: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library_sync_detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library_mirror: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream_commit: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct ArmaraosHostStatus {
    pub ok: bool,
    pub detail: String,
}

pub(crate) fn app_ainl_root(app: &AppHandle) -> Result<PathBuf, String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to resolve app_data_dir: {e}"))?;
    Ok(data_dir.join("ainl"))
}

fn read_install_source(app: &AppHandle) -> Option<String> {
    let p = app_ainl_root(app).ok()?.join(INSTALL_SOURCE_FILE);
    fs::read_to_string(p)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn write_install_source(app: &AppHandle, source: &str) -> Result<(), String> {
    let root = app_ainl_root(app)?;
    fs::create_dir_all(&root).map_err(|e| format!("Failed to create ainl dir: {e}"))?;
    let p = root.join(INSTALL_SOURCE_FILE);
    fs::write(&p, source).map_err(|e| format!("Failed to write install source: {e}"))
}

fn pypi_spec() -> String {
    std::env::var("ARMARAOS_AINL_PYPI_SPEC").unwrap_or_else(|_| DEFAULT_AINL_PYPI_SPEC.to_string())
}

fn auto_pip_upgrade_enabled() -> bool {
    !std::env::var("ARMARAOS_AINL_AUTO_PIP_UPGRADE")
        .map(|v| v == "0" || v.eq_ignore_ascii_case("false"))
        .unwrap_or(false)
}

fn write_auto_pip_upgrade_marker(app: &AppHandle) -> Result<(), String> {
    let root = app_ainl_root(app)?;
    fs::create_dir_all(&root).map_err(|e| format!("Failed to create ainl dir: {e}"))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let p = root.join(".last_auto_pip_upgrade_unix");
    fs::write(&p, format!("{now}\n"))
        .map_err(|e| format!("Failed to write pip upgrade marker: {e}"))
}

/// Throttled `pip install --upgrade` so PyPI picks up compiler/runtime fixes without a manual upgrade click.
fn maybe_auto_upgrade_ainl_pip(app: &AppHandle) {
    if !auto_pip_upgrade_enabled() {
        return;
    }
    let root = match app_ainl_root(app) {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, "AINL auto pip upgrade: no app data dir");
            return;
        }
    };
    let marker = root.join(".last_auto_pip_upgrade_unix");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(s) = fs::read_to_string(&marker) {
        if let Ok(last) = s.trim().parse::<u64>() {
            if now.saturating_sub(last) < AUTO_PIP_UPGRADE_INTERVAL_SECS {
                return;
            }
        }
    }
    let venv = match venv_dir(app) {
        Ok(v) => v,
        Err(e) => {
            warn!(error = %e, "AINL auto pip upgrade: no venv");
            return;
        }
    };
    let py = venv_python(&venv);
    if !py.exists() {
        return;
    }
    let spec = pypi_spec();
    match Command::new(&py)
        .args(["-m", "pip", "install", "--upgrade"])
        .arg(&spec)
        .env("PIP_DISABLE_PIP_VERSION_CHECK", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
    {
        Ok(out) if out.status.success() => {
            if let Err(e) = write_auto_pip_upgrade_marker(app) {
                warn!(error = %e, "AINL auto pip upgrade: wrote pip ok but marker failed");
            } else {
                tracing::info!(spec = %spec, "AINL auto pip upgrade applied");
            }
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            warn!(
                code = ?out.status.code(),
                stderr = %stderr.trim(),
                "AINL auto pip upgrade failed (offline or index unreachable); continuing"
            );
        }
        Err(e) => {
            warn!(error = %e, "AINL auto pip upgrade could not run pip");
        }
    }
}

/// Rust host triple folder under `resources/python/` (must match `xtask bundle-portable-python --target`).
fn portable_python_triple_dir() -> Option<&'static str> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return Some("aarch64-apple-darwin");
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return Some("x86_64-apple-darwin");
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return Some("aarch64-unknown-linux-gnu");
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return Some("x86_64-unknown-linux-gnu");
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    return Some("aarch64-pc-windows-msvc");
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return Some("x86_64-pc-windows-msvc");
    #[allow(unreachable_code)]
    None
}

fn find_bundled_portable_python(app: &AppHandle) -> Result<Option<PathBuf>, String> {
    let Some(triple) = portable_python_triple_dir() else {
        return Ok(None);
    };
    let root = match app.path().resource_dir() {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, "resource_dir unavailable; skipping bundled portable Python");
            return Ok(None);
        }
    };
    let base = root.join("resources").join("python").join(triple);
    let unix = base.join("python").join("bin").join("python3");
    if unix.is_file() {
        return Ok(Some(unix));
    }
    let win = base.join("python").join("python.exe");
    if win.is_file() {
        return Ok(Some(win));
    }
    Ok(None)
}

pub fn portable_python_is_bundled(app: &AppHandle) -> bool {
    find_bundled_portable_python(app).ok().flatten().is_some()
}

/// `python -c` → major minor for any interpreter (including `py -3.12` on Windows).
fn interpreter_version(exe: &Path, extra: &[&str]) -> Option<(u32, u32)> {
    let mut cmd = Command::new(exe);
    for a in extra {
        cmd.arg(a);
    }
    cmd.args([
        "-c",
        "import sys; print(sys.version_info[0], sys.version_info[1])",
    ]);
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let mut parts = s.split_whitespace();
    let a: u32 = parts.next()?.parse().ok()?;
    let b: u32 = parts.next()?.parse().ok()?;
    Some((a, b))
}

fn interpreter_meets_ainl_minimum(exe: &Path, extra: &[&str]) -> bool {
    interpreter_version(exe, extra)
        .is_some_and(|(a, b)| (a, b) >= (MIN_PYTHON_MAJOR, MIN_PYTHON_MINOR))
}

/// Version string and whether it can run PyPI `ainativelang` 1.3+.
fn venv_python_meta(venv: &Path) -> (Option<String>, bool) {
    let py = venv_python(venv);
    if !venv.is_dir() || !py.exists() {
        return (None, false);
    }
    match interpreter_version(&py, &[]) {
        Some((a, b)) => (
            Some(format!("{a}.{b}")),
            (a, b) >= (MIN_PYTHON_MAJOR, MIN_PYTHON_MINOR),
        ),
        None => (None, false),
    }
}

fn try_create_venv_with(exe: &Path, extra: &[&str], venv: &Path) -> bool {
    let mut cmd = Command::new(exe);
    for a in extra {
        cmd.arg(a);
    }
    cmd.args(["-m", "venv"]).arg(venv);
    cmd.output().map(|o| o.status.success()).unwrap_or(false) && venv_python(venv).exists()
}

fn collect_python_candidates(app: &AppHandle) -> Vec<(PathBuf, Vec<&'static str>)> {
    let mut v = Vec::new();
    if let Ok(Some(p)) = find_bundled_portable_python(app) {
        v.push((p, vec![]));
    }
    if let Ok(s) = std::env::var("ARMARAOS_PYTHON") {
        let t = s.trim();
        if !t.is_empty() {
            v.push((PathBuf::from(t), vec![]));
        }
    }
    // macOS .app bundles often get a minimal PATH (no Homebrew). Try common install locations
    // before relying on `python3` resolving to the system stub (may be 3.9).
    #[cfg(target_os = "macos")]
    {
        for p in [
            "/opt/homebrew/bin/python3.13",
            "/opt/homebrew/bin/python3.12",
            "/opt/homebrew/bin/python3.11",
            "/opt/homebrew/bin/python3.10",
            "/opt/homebrew/bin/python3",
            "/usr/local/opt/python@3.12/bin/python3.12",
            "/usr/local/opt/python@3.11/bin/python3.11",
            "/usr/local/opt/python@3.10/bin/python3.10",
            "/usr/local/bin/python3.13",
            "/usr/local/bin/python3.12",
            "/usr/local/bin/python3.11",
            "/usr/local/bin/python3.10",
            "/usr/local/bin/python3",
        ] {
            let pb = PathBuf::from(p);
            if pb.is_file() {
                v.push((pb, vec![]));
            }
        }
    }
    #[cfg(windows)]
    {
        for flag in ["-3.13", "-3.12", "-3.11", "-3.10", "-3"] {
            v.push((PathBuf::from("py"), vec![flag]));
        }
        v.push((PathBuf::from("python3"), vec![]));
        v.push((PathBuf::from("python"), vec![]));
    }
    #[cfg(not(windows))]
    {
        for name in [
            "python3.13",
            "python3.12",
            "python3.11",
            "python3.10",
            "python3",
            "python",
        ] {
            v.push((PathBuf::from(name), vec![]));
        }
    }
    v
}

/// Remove `venv` when it is missing Python, unreadable, or Python below 3.10 (cannot install `ainativelang` 1.3+ from PyPI).
fn remove_venv_if_python_below_minimum(venv: &Path) -> Result<(), String> {
    if !venv.is_dir() {
        return Ok(());
    }
    let vp = venv_python(venv);
    if !vp.exists() {
        fs::remove_dir_all(venv).map_err(|e| format!("Remove incomplete AINL venv: {e}"))?;
        return Ok(());
    }
    match interpreter_version(&vp, &[]) {
        Some((a, b)) if (a, b) >= (MIN_PYTHON_MAJOR, MIN_PYTHON_MINOR) => Ok(()),
        Some((a, b)) => {
            fs::remove_dir_all(venv).map_err(|e| {
                format!(
                    "Removed AINL venv: Python {a}.{b} is too old (need {MIN_PYTHON_MAJOR}.{MIN_PYTHON_MINOR}+ for PyPI ainativelang 1.3+). {e}"
                )
            })?;
            Ok(())
        }
        None => {
            fs::remove_dir_all(venv).map_err(|e| format!("Remove unreadable AINL venv: {e}"))?;
            Ok(())
        }
    }
}

/// Create a venv with Python ≥3.10: bundled portable, `ARMARAOS_PYTHON`, then versioned interpreters.
fn create_venv_with_bootstrap_python(app: &AppHandle, venv: &Path) -> Result<(), String> {
    if venv.exists() {
        let vp = venv_python(venv);
        if !vp.exists() {
            fs::remove_dir_all(venv).map_err(|e| format!("Remove broken venv: {e}"))?;
        }
    }

    for (exe, extra) in collect_python_candidates(app) {
        if !interpreter_meets_ainl_minimum(&exe, &extra) {
            continue;
        }
        if venv.exists() {
            fs::remove_dir_all(venv).map_err(|e| format!("Remove venv before recreate: {e}"))?;
        }
        if try_create_venv_with(&exe, &extra, venv) {
            let vp = venv_python(venv);
            if interpreter_meets_ainl_minimum(&vp, &[]) {
                return Ok(());
            }
            let _ = fs::remove_dir_all(venv);
        }
    }

    Err(format!(
        "This computer does not have Python {MIN_PYTHON_MAJOR}.{MIN_PYTHON_MINOR} or newer available to ArmaraOS. \
         AINL needs a recent Python to run. Easiest fix: install the latest Python from python.org/downloads (Windows: check \"Add python.exe to PATH\"), then restart ArmaraOS. \
         Or use an official ArmaraOS build that includes bundled Python (no install needed). Advanced: set ARMARAOS_PYTHON to a 3.10+ interpreter path."
    ))
}

pub(crate) fn venv_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app_ainl_root(app)?.join("venv"))
}

pub(crate) fn venv_python(venv: &Path) -> PathBuf {
    // Works for macOS/Linux ("bin/python") and Windows ("Scripts/python.exe").
    let unix = venv.join("bin").join("python");
    if unix.exists() {
        return unix;
    }
    let win = venv.join("Scripts").join("python.exe");
    if win.exists() {
        return win;
    }
    // Default to unix path; caller will get a good error message if missing.
    unix
}

pub(crate) fn venv_bin(venv: &Path, name: &str) -> PathBuf {
    let unix = venv.join("bin").join(name);
    if unix.exists() {
        return unix;
    }
    let win = venv.join("Scripts").join(format!("{name}.exe"));
    if win.exists() {
        return win;
    }
    unix
}

/// Writes [`openfang_kernel::ainl_library::AINL_BIN_CACHE_FILENAME`] under [`openfang_home`] so the
/// background daemon (cron, no GUI `PATH`) can spawn the internal venv `ainl` without `ARMARAOS_AINL_BIN`.
fn sync_ainl_executable_cache(venv: &Path) {
    let ainl = venv_bin(venv, "ainl");
    if !ainl.exists() {
        return;
    }
    let home = openfang_home();
    if let Err(e) = fs::create_dir_all(&home) {
        warn!(error = %e, "failed to create armaraos home for ainl bin cache");
        return;
    }
    let path = ainl.canonicalize().unwrap_or(ainl);
    let cache_path = home.join(AINL_BIN_CACHE_FILENAME);
    if let Err(e) = fs::write(&cache_path, format!("{}\n", path.display())) {
        warn!(error = %e, path = %cache_path.display(), "failed to write ainl bin cache");
    }
}

fn find_bundled_wheel(app: &AppHandle) -> Result<Option<PathBuf>, String> {
    let base = match app.path().resource_dir() {
        Ok(r) => r.join("resources").join("ainl"),
        Err(e) => {
            warn!(error = %e, "resource_dir unavailable; skipping bundled AINL wheel");
            return Ok(None);
        }
    };

    if !base.is_dir() {
        return Ok(None);
    }

    let entries = std::fs::read_dir(&base).map_err(|e| format!("Failed to read {base:?}: {e}"))?;
    for ent in entries {
        let ent = ent.map_err(|e| format!("Failed to read dir entry: {e}"))?;
        let p = ent.path();
        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if name.starts_with("ainativelang-") && name.ends_with("-py3-none-any.whl") {
            return Ok(Some(p));
        }
    }
    Ok(None)
}

fn run(cmd: &mut Command) -> Result<(), String> {
    let out = cmd
        .output()
        .map_err(|e| format!("Failed to run {cmd:?}: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    Err(format!(
        "Command failed (exit={})\nstdout:\n{}\nstderr:\n{}",
        out.status.code().unwrap_or(-1),
        stdout,
        stderr
    ))
}

fn venv_bin_dir(venv: &Path) -> PathBuf {
    let unix = venv.join("bin");
    if unix.exists() {
        return unix;
    }
    venv.join("Scripts")
}

fn path_with_venv_bin_prepended(venv: &Path) -> String {
    let bin = venv_bin_dir(venv);
    let sep = if cfg!(windows) { ';' } else { ':' };
    match std::env::var("PATH") {
        Ok(existing) => format!("{}{sep}{}", bin.display(), existing),
        Err(_) => bin.display().to_string(),
    }
}

/// `PATH` with the app venv `bin`/`Scripts` first — for spawning `ainl` from Tauri commands.
pub(crate) fn subprocess_path_with_venv_bin(venv: &Path) -> String {
    path_with_venv_bin_prepended(venv)
}

/// Same steps as `ainl install armaraos` MCP bootstrap, without `pip install ainativelang[mcp]` (bundled wheel already installed).
/// `PATH` must resolve `ainl-mcp` and `ainl` to the app venv so `~/.armaraos/config.toml` references the correct binaries.
const ARMARAOS_HOST_BOOTSTRAP_PY: &str = r#"
from pathlib import Path
from tooling.mcp_host_install import (
    ARMARAOS_PROFILE,
    ensure_mcp_registration,
    ensure_ainl_run_wrapper,
    ensure_path_hint_in_shell_rc,
)
home = Path.home()
ensure_mcp_registration(ARMARAOS_PROFILE, home=home, dry_run=False, verbose=False)
ensure_ainl_run_wrapper(ARMARAOS_PROFILE, home=home, dry_run=False, verbose=False)
ensure_path_hint_in_shell_rc(ARMARAOS_PROFILE, home=home, dry_run=False, verbose=False)
"#;

fn run_armaraos_host_bootstrap(venv: &Path) -> Result<String, String> {
    let py = venv_python(venv);
    if !py.exists() {
        return Err("venv Python not found".to_string());
    }
    let out = Command::new(&py)
        .arg("-c")
        .arg(ARMARAOS_HOST_BOOTSTRAP_PY)
        .env("PATH", path_with_venv_bin_prepended(venv))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("armaraos host bootstrap: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if out.status.success() {
        let detail = if stderr.is_empty() {
            "Registered AINL MCP + ainl-run under ~/.armaraos (and legacy ~/.openfang if present)"
                .to_string()
        } else {
            format!("Registered AINL MCP + ainl-run. Log: {stderr}")
        };
        if !stdout.is_empty() {
            return Ok(format!("{detail} ({stdout})"));
        }
        return Ok(detail);
    }
    Err(format!(
        "armaraos host bootstrap failed (exit={}): stdout={:?} stderr={:?}",
        out.status.code().unwrap_or(-1),
        stdout,
        stderr
    ))
}

fn apply_armaraos_host_bootstrap(app: &AppHandle, status: &mut AinlStatus) {
    if !status.ok {
        return;
    }
    let venv = match venv_dir(app) {
        Ok(v) => v,
        Err(e) => {
            status.armaraos_host_ok = Some(false);
            status.armaraos_host_detail = Some(e);
            return;
        }
    };
    match run_armaraos_host_bootstrap(&venv) {
        Ok(msg) => {
            status.armaraos_host_ok = Some(true);
            status.armaraos_host_detail = Some(msg);
        }
        Err(e) => {
            status.armaraos_host_ok = Some(false);
            status.armaraos_host_detail = Some(e);
        }
    }
}

/// Re-run ArmaraOS host integration only (MCP + wrappers). Use when the internal venv is already OK.
pub fn ensure_armaraos_ainl_host(app: &AppHandle) -> Result<ArmaraosHostStatus, String> {
    let st = ainl_status(app)?;
    if !st.ok {
        return Ok(ArmaraosHostStatus {
            ok: false,
            detail: "Internal AINL venv is not ready; run ensure_ainl_installed first".to_string(),
        });
    }
    let venv = venv_dir(app)?;
    match run_armaraos_host_bootstrap(&venv) {
        Ok(msg) => {
            if let Ok(mut st) = ainl_status(app) {
                if st.ok {
                    crate::ainl_upstream::apply_library_sync(app, &mut st);
                }
            }
            Ok(ArmaraosHostStatus {
                ok: true,
                detail: msg,
            })
        }
        Err(e) => Ok(ArmaraosHostStatus {
            ok: false,
            detail: e,
        }),
    }
}

pub fn ensure_ainl_installed(app: &AppHandle) -> Result<AinlStatus, String> {
    let venv = venv_dir(app)?;
    remove_venv_if_python_below_minimum(&venv)?;

    // Fast path: if we already have a working venv with ainl + ainl-mcp, only refresh ArmaraOS host config.
    if let Ok(st) = ainl_status(app) {
        if st.ok {
            let mut out = AinlStatus {
                detail: "AINL already installed in internal venv".to_string(),
                armaraos_host_ok: None,
                armaraos_host_detail: None,
                ..st
            };
            maybe_auto_upgrade_ainl_pip(app);
            sync_ainl_executable_cache(&venv);
            apply_armaraos_host_bootstrap(app, &mut out);
            crate::ainl_upstream::apply_library_sync(app, &mut out);
            return Ok(out);
        }
    }

    std::fs::create_dir_all(&venv).map_err(|e| format!("Failed to create venv dir: {e}"))?;

    let wheel = find_bundled_wheel(app)?;
    let wheel_path_str = wheel.as_ref().map(|w| w.display().to_string());

    // Create venv if python not present yet.
    let py = venv_python(&venv);
    if !py.exists() {
        create_venv_with_bootstrap_python(app, &venv)?;
    }

    let py = venv_python(&venv);
    if !py.exists() {
        return Err(
            "Virtualenv was not created: Python executable missing. Install Python 3.10 or newer (or set ARMARAOS_PYTHON), then try Bootstrap AINL again."
                .to_string(),
        );
    }

    run(Command::new(&py).args(["-m", "pip", "install", "--upgrade", "pip"]))?;

    let install_detail: String;
    if let Some(ref w) = wheel {
        run(Command::new(&py)
            .args(["-m", "pip", "install", "--no-deps"])
            .arg(w))?;
        let _ = write_install_source(app, "bundled_wheel");
        install_detail = "AINL installed from bundled wheel (offline)".to_string();
        let _ = write_auto_pip_upgrade_marker(app);
    } else {
        let spec = pypi_spec();
        run(
            Command::new(&py)
                .args(["-m", "pip", "install"])
                .arg(&spec)
                .env("PIP_DISABLE_PIP_VERSION_CHECK", "1"),
        )
        .map_err(|e| {
            let mut msg = format!(
                "{e} (Bundled wheel was missing and PyPI install failed. Connect to the internet once, or set ARMARAOS_AINL_PYPI_SPEC / bundle a wheel under resources/ainl/.)"
            );
            if e.contains("Requires-Python") || e.contains("no matching distribution") {
                msg.push_str(
                    " PyPI `ainativelang` 1.3+ needs Python 3.10+. Install it, set ARMARAOS_PYTHON to a 3.10+ interpreter, or click Bootstrap AINL again after removing an old venv.",
                );
            }
            msg
        })?;
        let _ = write_install_source(app, "pypi");
        install_detail = format!(
            "AINL installed from PyPI ({spec}). First-time setup used the network; air-gapped machines should ship with a bundled wheel under resources/ainl/ or set PIP_INDEX_URL / extra index env vars."
        );
        let _ = write_auto_pip_upgrade_marker(app);
    }

    // Smoke: ainl + ainl-mcp entrypoints.
    let ainl = venv_bin(&venv, "ainl");
    let ainl_mcp = venv_bin(&venv, "ainl-mcp");
    let ainl_ok = Command::new(&ainl)
        .arg("--help")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let ainl_mcp_ok = Command::new(&ainl_mcp)
        .arg("--help")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    let (venv_python_version, venv_python_meets_ainl) = venv_python_meta(&venv);

    let mut status = AinlStatus {
        ok: ainl_ok && ainl_mcp_ok,
        venv_exists: venv.is_dir(),
        ainl_ok,
        ainl_mcp_ok,
        wheel_found: wheel.is_some(),
        wheel_path: wheel_path_str,
        detail: install_detail,
        armaraos_host_ok: None,
        armaraos_host_detail: None,
        install_source: read_install_source(app),
        portable_python_bundled: portable_python_is_bundled(app),
        venv_python_version,
        venv_python_meets_ainl,
        library_sync_ok: None,
        library_sync_detail: None,
        library_root: None,
        library_mirror: None,
        upstream_commit: None,
    };
    apply_armaraos_host_bootstrap(app, &mut status);
    crate::ainl_upstream::apply_library_sync(app, &mut status);
    if status.ok {
        sync_ainl_executable_cache(&venv);
    }
    Ok(status)
}

pub fn ainl_status(app: &AppHandle) -> Result<AinlStatus, String> {
    let venv = venv_dir(app)?;
    let wheel = find_bundled_wheel(app)?;
    let py = venv_python(&venv);
    let ainl = venv_bin(&venv, "ainl");
    let ainl_mcp = venv_bin(&venv, "ainl-mcp");
    let ainl_ok = ainl.exists();
    let ainl_mcp_ok = ainl_mcp.exists();

    let (venv_python_version, venv_python_meets_ainl) = venv_python_meta(&venv);

    let ready = venv.is_dir() && py.exists() && ainl_ok && ainl_mcp_ok;
    let detail = if ready {
        "AINL is installed in the app virtualenv.".to_string()
    } else {
        "Probe only — startup auto-install runs in the background, or use Bootstrap AINL. \
         (If this stays empty, install Python 3.10+ or set ARMARAOS_PYTHON; GUI apps on macOS may not see Homebrew on PATH.)"
            .to_string()
    };
    let mut s = AinlStatus {
        ok: ready,
        venv_exists: venv.is_dir() && py.exists(),
        ainl_ok,
        ainl_mcp_ok,
        wheel_found: wheel.is_some(),
        wheel_path: wheel.map(|p| p.display().to_string()),
        detail,
        armaraos_host_ok: None,
        armaraos_host_detail: None,
        install_source: read_install_source(app),
        portable_python_bundled: portable_python_is_bundled(app),
        venv_python_version,
        venv_python_meets_ainl,
        library_sync_ok: None,
        library_sync_detail: None,
        library_root: None,
        library_mirror: None,
        upstream_commit: None,
    };
    crate::ainl_upstream::enrich_status_from_manifest(app, &mut s);
    if ready {
        sync_ainl_executable_cache(&venv);
    }
    Ok(s)
}

/// Run `pip install --upgrade` for [`pypi_spec`] in the app venv, then refresh MCP host + upstream library sync.
pub fn upgrade_ainl_pip(app: &AppHandle) -> Result<AinlStatus, String> {
    let venv = venv_dir(app)?;
    remove_venv_if_python_below_minimum(&venv)?;
    let py = venv_python(&venv);
    if !py.exists() {
        return Err(
            "AINL virtualenv is missing (removed because Python was below 3.10, or never created). Open Settings → AINL and click **Bootstrap AINL**."
                .to_string(),
        );
    }
    let spec = pypi_spec();
    run(Command::new(&py)
        .args(["-m", "pip", "install", "--upgrade"])
        .arg(&spec)
        .env("PIP_DISABLE_PIP_VERSION_CHECK", "1"))
    .map_err(|e| {
        let mut msg = format!("pip upgrade failed: {e}");
        if e.contains("Requires-Python") || e.contains("no matching distribution") {
            msg.push_str(
                " Install Python 3.10+, set ARMARAOS_PYTHON if needed, then Bootstrap AINL again.",
            );
        }
        msg
    })?;
    let _ = write_auto_pip_upgrade_marker(app);

    let mut status = ainl_status(app)?;
    if !status.ok {
        return Ok(status);
    }
    apply_armaraos_host_bootstrap(app, &mut status);
    crate::ainl_upstream::apply_library_sync(app, &mut status);
    Ok(status)
}
