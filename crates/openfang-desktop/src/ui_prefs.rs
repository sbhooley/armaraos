//! Desktop UI preferences persisted on disk.
//!
//! The embedded dashboard is loaded from `http://127.0.0.1:{random_port}/` each launch.
//! `localStorage` is per-origin, so theme choice would otherwise reset on every restart.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::AppHandle;
use tauri::Manager;

const PREFS_FILE: &str = "desktop_ui_prefs.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DesktopUiPrefs {
    #[serde(default = "default_theme_mode")]
    theme_mode: String,
}

fn default_theme_mode() -> String {
    "dark".to_string()
}

impl Default for DesktopUiPrefs {
    fn default() -> Self {
        Self {
            theme_mode: default_theme_mode(),
        }
    }
}

fn prefs_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(dir.join(PREFS_FILE))
}

fn normalize_mode(m: &str) -> String {
    match m {
        "light" | "dark" | "system" => m.to_string(),
        _ => default_theme_mode(),
    }
}

/// Load saved theme mode for the first navigation URL (`light` | `dark` | `system`).
pub fn load_theme_mode(app: &AppHandle) -> String {
    let Ok(path) = prefs_path(app) else {
        return default_theme_mode();
    };
    let Ok(bytes) = fs::read(&path) else {
        return default_theme_mode();
    };
    let Ok(prefs) = serde_json::from_slice::<DesktopUiPrefs>(&bytes) else {
        return default_theme_mode();
    };
    normalize_mode(&prefs.theme_mode)
}

/// Persist theme mode from the dashboard (mirrors localStorage for cross-port restarts).
pub fn save_theme_mode(app: &AppHandle, mode: &str) -> Result<(), String> {
    let mode = normalize_mode(mode);
    let path = prefs_path(app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut prefs = DesktopUiPrefs::default();
    if path.exists() {
        if let Ok(bytes) = fs::read(&path) {
            if let Ok(p) = serde_json::from_slice::<DesktopUiPrefs>(&bytes) {
                prefs = p;
            }
        }
    }
    prefs.theme_mode = mode;
    let json = serde_json::to_string_pretty(&prefs).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(())
}
