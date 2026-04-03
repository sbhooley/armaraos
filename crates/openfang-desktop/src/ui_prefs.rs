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

/// Stable channel (default): Tauri updater JSON on the marketing site.
pub const STABLE_FEED_URL: &str = "https://ainativelang.com/downloads/armaraos/latest.json";
/// Beta channel: publish a signed `beta.json` alongside stable when ready.
pub const BETA_FEED_URL: &str = "https://ainativelang.com/downloads/armaraos/beta.json";

pub fn feed_url_for_channel(channel: &str) -> &'static str {
    match channel {
        "beta" => BETA_FEED_URL,
        _ => STABLE_FEED_URL,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DesktopUiPrefs {
    #[serde(default = "default_theme_mode")]
    theme_mode: String,
    /// `stable` or `beta` — selects Tauri updater feed URL.
    #[serde(default = "default_release_channel")]
    release_channel: String,
    #[serde(default)]
    updater_last_check_at: Option<String>,
    #[serde(default)]
    updater_last_error: Option<String>,
    #[serde(default)]
    daemon_update_last_check_at: Option<String>,
    #[serde(default)]
    daemon_update_last_error: Option<String>,
}

fn default_theme_mode() -> String {
    "dark".to_string()
}

fn default_release_channel() -> String {
    "stable".to_string()
}

impl Default for DesktopUiPrefs {
    fn default() -> Self {
        Self {
            theme_mode: default_theme_mode(),
            release_channel: default_release_channel(),
            updater_last_check_at: None,
            updater_last_error: None,
            daemon_update_last_check_at: None,
            daemon_update_last_error: None,
        }
    }
}

fn prefs_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(dir.join(PREFS_FILE))
}

fn load_full_prefs(app: &AppHandle) -> DesktopUiPrefs {
    let Ok(path) = prefs_path(app) else {
        return DesktopUiPrefs::default();
    };
    let Ok(bytes) = fs::read(&path) else {
        return DesktopUiPrefs::default();
    };
    serde_json::from_slice::<DesktopUiPrefs>(&bytes).unwrap_or_default()
}

fn save_full_prefs(app: &AppHandle, prefs: &DesktopUiPrefs) -> Result<(), String> {
    let path = prefs_path(app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(prefs).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| e.to_string())
}

fn normalize_mode(m: &str) -> String {
    match m {
        "light" | "dark" | "system" => m.to_string(),
        _ => default_theme_mode(),
    }
}

fn normalize_channel(c: &str) -> String {
    match c {
        "beta" => "beta".to_string(),
        _ => "stable".to_string(),
    }
}

/// Load saved theme mode for the first navigation URL (`light` | `dark` | `system`).
pub fn load_theme_mode(app: &AppHandle) -> String {
    normalize_mode(&load_full_prefs(app).theme_mode)
}

/// `stable` or `beta`.
pub fn load_release_channel(app: &AppHandle) -> String {
    normalize_channel(&load_full_prefs(app).release_channel)
}

/// Persist release channel for the Tauri updater feed.
pub fn save_release_channel(app: &AppHandle, channel: &str) -> Result<(), String> {
    let mut prefs = load_full_prefs(app);
    prefs.release_channel = normalize_channel(channel);
    save_full_prefs(app, &prefs)
}

/// Record desktop (Tauri) updater check outcome and timestamp.
pub fn record_updater_check(app: &AppHandle, err: Option<&str>) {
    let mut prefs = load_full_prefs(app);
    prefs.updater_last_check_at = Some(chrono::Utc::now().to_rfc3339());
    prefs.updater_last_error = err.map(|s| s.to_string());
    let _ = save_full_prefs(app, &prefs);
}

/// Record daemon/runtime vs-GitHub check (dashboard may also write via API later).
pub fn record_daemon_update_check(app: &AppHandle, err: Option<&str>) {
    let mut prefs = load_full_prefs(app);
    prefs.daemon_update_last_check_at = Some(chrono::Utc::now().to_rfc3339());
    prefs.daemon_update_last_error = err.map(|s| s.to_string());
    let _ = save_full_prefs(app, &prefs);
}

/// JSON for Settings / Runtime UI (desktop shell).
pub fn updater_prefs_snapshot(app: &AppHandle) -> serde_json::Value {
    let p = load_full_prefs(app);
    serde_json::json!({
        "release_channel": p.release_channel,
        "stable_feed_url": STABLE_FEED_URL,
        "beta_feed_url": BETA_FEED_URL,
        "active_feed_url": feed_url_for_channel(&p.release_channel),
        "updater_last_check_at": p.updater_last_check_at,
        "updater_last_error": p.updater_last_error,
        "daemon_update_last_check_at": p.daemon_update_last_check_at,
        "daemon_update_last_error": p.daemon_update_last_error,
    })
}

/// Persist theme mode from the dashboard (mirrors localStorage for cross-port restarts).
pub fn save_theme_mode(app: &AppHandle, mode: &str) -> Result<(), String> {
    let mode = normalize_mode(mode);
    let mut prefs = load_full_prefs(app);
    prefs.theme_mode = mode;
    save_full_prefs(app, &prefs)
}
