//! PyPI + GitHub version visibility for the bundled AINL venv (`pip` / `ainativelang`).

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use semver::Version;
use serde::Serialize;
use tauri::AppHandle;
use tauri::Manager;
use tauri_plugin_notification::NotificationExt;

use crate::notification_icon::apply_notification_icon;

use crate::ainl::{venv_dir, venv_python};

const PYPI_PACKAGE_JSON: &str = "https://pypi.org/pypi/ainativelang/json";

/// Result of comparing the internal venv to PyPI and GitHub `main`.
#[derive(Debug, Clone, Serialize)]
pub struct AinlVersionInfo {
    pub installed_version: Option<String>,
    pub pypi_latest_version: Option<String>,
    pub upgrade_available: bool,
    /// Short SHA of `main` on `sbhooley/ainativelang` (upstream examples/library sync).
    pub github_main_sha: Option<String>,
    pub pypi_error: Option<String>,
    pub github_error: Option<String>,
}

fn pip_show_version(py: &Path, package: &str) -> Option<String> {
    let out = Command::new(py)
        .args(["-m", "pip", "show", package])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("Version:") {
            let v = rest.trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn ainl_cli_version_line(venv: &Path) -> Option<String> {
    let ainl = crate::ainl::venv_bin(venv, "ainl");
    if !ainl.exists() {
        return None;
    }
    let out = Command::new(&ainl).arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        return None;
    }
    // Typical: "ainl 1.3.3" or "1.3.3"
    s.split_whitespace()
        .last()
        .map(|t| t.to_string())
        .or(Some(s))
}

fn fetch_pypi_latest_version() -> Result<String, String> {
    let resp = ureq::get(PYPI_PACKAGE_JSON)
        .set(
            "User-Agent",
            &format!("ArmaraOS-Desktop/{}", env!("CARGO_PKG_VERSION")),
        )
        .timeout(Duration::from_secs(45))
        .call()
        .map_err(|e| format!("PyPI request: {e}"))?;
    if resp.status() != 200 {
        return Err(format!("PyPI HTTP {}", resp.status()));
    }
    let mut body = String::new();
    resp.into_reader()
        .read_to_string(&mut body)
        .map_err(|e| format!("read PyPI body: {e}"))?;
    let v: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("parse PyPI JSON: {e}"))?;
    v["info"]["version"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "PyPI response missing info.version".to_string())
}

fn remote_is_newer(installed: &str, remote: &str) -> bool {
    match (
        Version::parse(installed.trim()),
        Version::parse(remote.trim()),
    ) {
        (Ok(a), Ok(b)) => b > a,
        _ => installed.trim() != remote.trim(),
    }
}

/// Compare installed `ainativelang` in the app venv to PyPI and report `main` SHA on GitHub.
pub fn ainl_version_info(app: &AppHandle) -> AinlVersionInfo {
    let mut out = AinlVersionInfo {
        installed_version: None,
        pypi_latest_version: None,
        upgrade_available: false,
        github_main_sha: None,
        pypi_error: None,
        github_error: None,
    };

    let venv = match venv_dir(app) {
        Ok(v) => v,
        Err(_) => return out,
    };
    let py = venv_python(&venv);
    if !py.exists() {
        return out;
    }

    out.installed_version =
        pip_show_version(&py, "ainativelang").or_else(|| ainl_cli_version_line(&venv));

    match fetch_pypi_latest_version() {
        Ok(ver) => {
            out.pypi_latest_version = Some(ver.clone());
            if let Some(ref ins) = out.installed_version {
                out.upgrade_available = remote_is_newer(ins, &ver);
            } else {
                out.upgrade_available = true;
            }
        }
        Err(e) => out.pypi_error = Some(e),
    }

    match crate::ainl_upstream::fetch_main_commit_sha() {
        Ok(sha) => {
            out.github_main_sha = Some(sha.chars().take(12).collect());
        }
        Err(e) => out.github_error = Some(e),
    }

    out
}

const AINL_PYPI_NOTIFY_INITIAL_DELAY_SECS: u64 = 120;
const AINL_PYPI_NOTIFY_RECHECK_SECS: u64 = 6 * 60 * 60;

fn ainl_notify_state_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir: {e}"))?;
    Ok(dir.join("ainl_pypi_notify_state.json"))
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct AinlPypiNotifyState {
    /// PyPI version we last raised a desktop notification for.
    last_notified_pypi_version: Option<String>,
}

fn load_ainl_notify_state(app: &AppHandle) -> AinlPypiNotifyState {
    let Ok(path) = ainl_notify_state_path(app) else {
        return AinlPypiNotifyState::default();
    };
    let Ok(data) = fs::read_to_string(&path) else {
        return AinlPypiNotifyState::default();
    };
    serde_json::from_str(&data).unwrap_or_default()
}

fn save_ainl_notify_state(app: &AppHandle, state: &AinlPypiNotifyState) -> Result<(), String> {
    let path = ainl_notify_state_path(app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let data = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
    fs::write(&path, data).map_err(|e| e.to_string())
}

/// Compare the app venv to PyPI on a schedule and show one OS notification per new PyPI version.
///
/// Waits [`AINL_PYPI_NOTIFY_INITIAL_DELAY_SECS`] before the first check (AINL bootstrap may still run),
/// then rechecks every [`AINL_PYPI_NOTIFY_RECHECK_SECS`]. Set `ARMARAOS_AINL_PYPI_NOTIFY=0` to disable.
pub fn spawn_ainl_pypi_notify_check(app_handle: AppHandle) {
    let skip = std::env::var("ARMARAOS_AINL_PYPI_NOTIFY")
        .map(|v| v == "0" || v.eq_ignore_ascii_case("false"))
        .unwrap_or(false);
    if skip {
        return;
    }

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(
            AINL_PYPI_NOTIFY_INITIAL_DELAY_SECS,
        ))
        .await;

        loop {
            let app = app_handle.clone();
            let info = match tokio::task::spawn_blocking(move || ainl_version_info(&app)).await {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("AINL PyPI notify join failed: {e}");
                    tokio::time::sleep(std::time::Duration::from_secs(
                        AINL_PYPI_NOTIFY_RECHECK_SECS,
                    ))
                    .await;
                    continue;
                }
            };

            if info.pypi_error.is_some() {
                tracing::debug!("AINL PyPI notify: {:?}", info.pypi_error);
            } else if info.upgrade_available && info.installed_version.is_some() {
                if let Some(pypi_ver) = &info.pypi_latest_version {
                    let mut state = load_ainl_notify_state(&app_handle);
                    if state.last_notified_pypi_version.as_deref() == Some(pypi_ver.as_str()) {
                        // Already notified for this PyPI release.
                    } else {
                        let installed = info.installed_version.as_deref().unwrap_or("?");
                        let _ = apply_notification_icon(
                            app_handle
                                .notification()
                                .builder()
                                .title("AINL update available")
                                .body(format!(
                                    "PyPI has ainativelang v{pypi_ver} (you have v{installed}). Open Settings → AINL to upgrade."
                                )),
                        )
                        .show();
                        state.last_notified_pypi_version = Some(pypi_ver.clone());
                        if let Err(e) = save_ainl_notify_state(&app_handle, &state) {
                            tracing::warn!("Failed to save AINL notify state: {e}");
                        }
                    }
                }
            }

            tokio::time::sleep(std::time::Duration::from_secs(
                AINL_PYPI_NOTIFY_RECHECK_SECS,
            ))
            .await;
        }
    });
}
