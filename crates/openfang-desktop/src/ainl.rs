//! AINL bundling bootstrap (Option A).
//!
//! Creates an internal venv under the app data directory and installs a bundled
//! `ainativelang` wheel (offline) when present. This is designed to be called
//! from Tauri commands and/or on app startup.

use std::path::{Path, PathBuf};
use std::process::Command;

use tauri::{AppHandle, Manager};

#[derive(Debug, serde::Serialize)]
pub struct AinlStatus {
    pub ok: bool,
    pub venv_exists: bool,
    pub ainl_ok: bool,
    pub ainl_mcp_ok: bool,
    pub wheel_found: bool,
    pub wheel_path: Option<String>,
    pub detail: String,
}

fn app_ainl_root(app: &AppHandle) -> Result<PathBuf, String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to resolve app_data_dir: {e}"))?;
    Ok(data_dir.join("ainl"))
}

fn venv_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app_ainl_root(app)?.join("venv"))
}

fn venv_python(venv: &Path) -> PathBuf {
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

fn venv_bin(venv: &Path, name: &str) -> PathBuf {
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

fn find_bundled_wheel(app: &AppHandle) -> Result<Option<PathBuf>, String> {
    let base = app
        .path()
        .resource_dir()
        .map_err(|e| format!("Failed to resolve resource_dir: {e}"))?
        .join("resources")
        .join("ainl");

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
    let out = cmd.output().map_err(|e| format!("Failed to run {cmd:?}: {e}"))?;
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

pub fn ensure_ainl_installed(app: &AppHandle) -> Result<AinlStatus, String> {
    // Fast path: if we already have a working venv with ainl + ainl-mcp, do nothing.
    if let Ok(st) = ainl_status(app) {
        if st.ok {
            return Ok(AinlStatus {
                detail: "AINL already installed in internal venv".to_string(),
                ..st
            });
        }
    }

    let venv = venv_dir(app)?;
    std::fs::create_dir_all(&venv).map_err(|e| format!("Failed to create venv dir: {e}"))?;

    let wheel = find_bundled_wheel(app)?;
    if wheel.is_none() {
        return Ok(AinlStatus {
            ok: false,
            venv_exists: venv.is_dir(),
            ainl_ok: false,
            ainl_mcp_ok: false,
            wheel_found: false,
            wheel_path: None,
            detail: "Bundled ainativelang wheel not found under resources/ainl/".to_string(),
        });
    }
    let wheel = wheel.unwrap();

    // Create venv if python not present yet.
    let py = venv_python(&venv);
    if !py.exists() {
        // Use system python for bootstrap. Later we can embed Python per-platform.
        run(Command::new("python3").args(["-m", "venv"]).arg(&venv))
            .or_else(|_| run(Command::new("python").args(["-m", "venv"]).arg(&venv)))?;
    }

    let py = venv_python(&venv);
    if !py.exists() {
        return Err("Venv created but python executable not found in venv".to_string());
    }

    // Install wheel offline. (Pip may still do some environment checks, but no network required.)
    run(Command::new(&py).args(["-m", "pip", "install", "--upgrade", "pip"]))?;
    run(Command::new(&py).args(["-m", "pip", "install", "--no-deps"]).arg(&wheel))?;

    // Smoke: ainl + ainl-mcp entrypoints.
    let ainl = venv_bin(&venv, "ainl");
    let ainl_mcp = venv_bin(&venv, "ainl-mcp");
    let ainl_ok = Command::new(&ainl).arg("--help").output().map(|o| o.status.success()).unwrap_or(false);
    let ainl_mcp_ok =
        Command::new(&ainl_mcp).arg("--help").output().map(|o| o.status.success()).unwrap_or(false);

    Ok(AinlStatus {
        ok: ainl_ok && ainl_mcp_ok,
        venv_exists: venv.is_dir(),
        ainl_ok,
        ainl_mcp_ok,
        wheel_found: true,
        wheel_path: Some(wheel.display().to_string()),
        detail: "AINL installed into internal venv".to_string(),
    })
}

pub fn ainl_status(app: &AppHandle) -> Result<AinlStatus, String> {
    let venv = venv_dir(app)?;
    let wheel = find_bundled_wheel(app)?;
    let py = venv_python(&venv);
    let ainl = venv_bin(&venv, "ainl");
    let ainl_mcp = venv_bin(&venv, "ainl-mcp");
    let ainl_ok = ainl.exists();
    let ainl_mcp_ok = ainl_mcp.exists();

    Ok(AinlStatus {
        ok: venv.is_dir() && py.exists() && ainl_ok && ainl_mcp_ok,
        venv_exists: venv.is_dir() && py.exists(),
        ainl_ok,
        ainl_mcp_ok,
        wheel_found: wheel.is_some(),
        wheel_path: wheel.map(|p| p.display().to_string()),
        detail: "Status only (no install attempted)".to_string(),
    })
}

