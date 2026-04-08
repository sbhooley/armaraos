//! Update checker for the ArmaraOS desktop app.
//!
//! Flow: try the Tauri updater against the marketing-site feed (`latest.json` / `beta.json`) first
//! so signed in-app installs work when the mirror is current. If that feed errors (network, parse),
//! or if it reports “no update” while the site may be stale, we compare against
//! GitHub’s latest release API so users still see new releases and a download link.
//!
//! After startup ([`spawn_startup_check`]), [`spawn_periodic_update_check`] re-runs the same check
//! every hour so releases propagate without waiting for the next app relaunch. Set
//! `ARMARAOS_DESKTOP_UPDATE_PERIODIC=0` to disable the hourly loop.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri::Manager;
use url::Url;

use crate::os_notify;
use tauri_plugin_updater::UpdaterExt;
use tracing::{info, warn};

/// Structured result from an update check.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    /// Whether a newer version is available.
    pub available: bool,
    /// The new version string, if available.
    pub version: Option<String>,
    /// Release notes body, if available.
    pub body: Option<String>,
    /// Where the update metadata came from: `website` (tauri updater) or `github` (fallback check).
    pub source: String,
    /// If present, a page the user can open to download manually.
    pub download_url: Option<String>,
    /// True if the app can download+install automatically (tauri updater flow).
    pub installable: bool,
    /// Updater JSON URL used for this check (or channel feed when fallback).
    pub feed_url: String,
}

fn updater_for_feed(
    app_handle: &tauri::AppHandle,
    feed_url: &str,
) -> Result<tauri_plugin_updater::Updater, String> {
    let url = Url::parse(feed_url).map_err(|e| e.to_string())?;
    app_handle
        .updater_builder()
        .endpoints(vec![url])
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())
}

/// Spawn a background task that checks for updates after a 10-second delay.
///
/// If an update is found, installs it silently and restarts the app.
/// All errors are logged but never panic.
pub fn spawn_startup_check(app_handle: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;

        let result = check_for_update(&app_handle).await;

        match result {
            Ok(info) if info.available && info.installable => {
                let version = info.version.as_deref().unwrap_or("unknown");
                info!("Update available: v{version}, installing silently...");
                os_notify::post_from_app(
                    &app_handle,
                    "ArmaraOS Updating...",
                    format!("Installing v{version}. App will restart shortly."),
                );
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                if let Err(e) = download_and_install_update(&app_handle).await {
                    warn!("Auto-update install failed: {e}");
                }
            }
            Ok(info) if info.available && !info.installable => {
                let version = info.version.as_deref().unwrap_or("unknown");
                let url = info
                    .download_url
                    .clone()
                    .unwrap_or_else(|| "https://github.com/sbhooley/armaraos/releases".to_string());
                os_notify::post_from_app(
                    &app_handle,
                    "ArmaraOS Update Available",
                    format!("v{version} is available. Download: {url}"),
                );
            }
            Ok(_) => info!("No updates available"),
            Err(e) => warn!("Startup update check failed: {e}"),
        }
    });
}

/// Interval between background update checks after the first hour of runtime.
const DESKTOP_PERIODIC_UPDATE_INTERVAL_SECS: u64 = 60 * 60;

fn periodic_update_state_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir: {e}"))?;
    Ok(dir.join("desktop_periodic_update_state.json"))
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct PeriodicUpdateNotifyState {
    /// Non-installable (e.g. GitHub-only) release we last showed a notification for.
    last_notified_noninstallable_version: Option<String>,
}

fn load_periodic_update_state(app: &AppHandle) -> PeriodicUpdateNotifyState {
    let Ok(path) = periodic_update_state_path(app) else {
        return PeriodicUpdateNotifyState::default();
    };
    let Ok(data) = fs::read_to_string(&path) else {
        return PeriodicUpdateNotifyState::default();
    };
    serde_json::from_str(&data).unwrap_or_default()
}

fn save_periodic_update_state(
    app: &AppHandle,
    state: &PeriodicUpdateNotifyState,
) -> Result<(), String> {
    let path = periodic_update_state_path(app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let data = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
    fs::write(&path, data).map_err(|e| e.to_string())
}

/// Hourly update check: retry signed installs silently; notify once per version for download-only updates.
///
/// First tick runs after [`DESKTOP_PERIODIC_UPDATE_INTERVAL_SECS`] so it does not duplicate
/// [`spawn_startup_check`] (which runs at ~10s). Disabled when `ARMARAOS_DESKTOP_UPDATE_PERIODIC=0`.
pub fn spawn_periodic_update_check(app_handle: AppHandle) {
    let skip = std::env::var("ARMARAOS_DESKTOP_UPDATE_PERIODIC")
        .map(|v| v == "0" || v.eq_ignore_ascii_case("false"))
        .unwrap_or(false);
    if skip {
        return;
    }

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(
            DESKTOP_PERIODIC_UPDATE_INTERVAL_SECS,
        ))
        .await;

        loop {
            match check_for_update(&app_handle).await {
                Ok(info) if info.available && info.installable => {
                    let version = info.version.as_deref().unwrap_or("unknown");
                    info!("Periodic check: installable update v{version}, attempting install…");
                    if let Err(e) = download_and_install_update(&app_handle).await {
                        warn!("Periodic update install failed (will retry on next interval): {e}");
                    }
                }
                Ok(info) if info.available && !info.installable => {
                    let version = info
                        .version
                        .clone()
                        .unwrap_or_else(|| "unknown".to_string());
                    let mut state = load_periodic_update_state(&app_handle);
                    if state.last_notified_noninstallable_version.as_deref()
                        == Some(version.as_str())
                    {
                        // Already notified for this release.
                    } else {
                        let url = info.download_url.clone().unwrap_or_else(|| {
                            "https://github.com/sbhooley/armaraos/releases".to_string()
                        });
                        os_notify::post_from_app(
                            &app_handle,
                            "ArmaraOS Update Available",
                            format!("v{version} is available. Download: {url}"),
                        );
                        state.last_notified_noninstallable_version = Some(version);
                        if let Err(e) = save_periodic_update_state(&app_handle, &state) {
                            warn!("Failed to save periodic update notify state: {e}");
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => warn!("Periodic update check failed: {e}"),
            }

            tokio::time::sleep(std::time::Duration::from_secs(
                DESKTOP_PERIODIC_UPDATE_INTERVAL_SECS,
            ))
            .await;
        }
    });
}

/// Perform an on-demand update check. Returns structured result.
pub async fn check_for_update(app_handle: &tauri::AppHandle) -> Result<UpdateInfo, String> {
    let r = do_check(app_handle).await;
    match &r {
        Ok(_) => crate::ui_prefs::record_updater_check(app_handle, None),
        Err(e) => crate::ui_prefs::record_updater_check(app_handle, Some(e.as_str())),
    }
    r
}

/// Download and install the latest update, then restart the app.
/// Should only be called after `check_for_update()` confirms availability.
///
/// On success, calls `app_handle.restart()` which terminates the process —
/// the function never returns `Ok`. On failure, returns `Err(message)`.
pub async fn download_and_install_update(app_handle: &tauri::AppHandle) -> Result<(), String> {
    let channel = crate::ui_prefs::load_release_channel(app_handle);
    let feed = crate::ui_prefs::feed_url_for_channel(&channel);
    let updater = updater_for_feed(app_handle, feed)?;
    let update = updater
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "No update available".to_string())?;

    info!("Downloading update v{}...", update.version);
    update
        .download_and_install(|_downloaded, _total| {}, || {})
        .await
        .map_err(|e| e.to_string())?;

    info!("Update installed, restarting...");
    app_handle.restart()
}

fn no_update_from_website(feed: &str) -> UpdateInfo {
    UpdateInfo {
        available: false,
        version: None,
        body: None,
        source: "website".to_string(),
        download_url: None,
        installable: false,
        feed_url: feed.to_string(),
    }
}

async fn do_check(app_handle: &tauri::AppHandle) -> Result<UpdateInfo, String> {
    let channel = crate::ui_prefs::load_release_channel(app_handle);
    let feed = crate::ui_prefs::feed_url_for_channel(&channel);
    let updater = updater_for_feed(app_handle, feed)?;
    match updater.check().await {
        Ok(Some(update)) => Ok(UpdateInfo {
            available: true,
            version: Some(update.version.clone()),
            body: update.body.clone(),
            source: "website".to_string(),
            download_url: None,
            installable: true,
            feed_url: feed.to_string(),
        }),
        Ok(None) => {
            // Website mirror may lag behind GitHub Releases; confirm against API.
            match github_fallback_check(feed).await {
                Ok(g) if g.available => Ok(g),
                Ok(_) => Ok(no_update_from_website(feed)),
                Err(e) => {
                    warn!(
                        "GitHub secondary update check failed (website reported up to date): {e}"
                    );
                    Ok(no_update_from_website(feed))
                }
            }
        }
        Err(e) => match github_fallback_check(feed).await {
            Ok(info) => Ok(info),
            Err(_) => Err(e.to_string()),
        },
    }
}

#[derive(Debug, Deserialize)]
struct GithubLatestRelease {
    tag_name: String,
    html_url: String,
    body: Option<String>,
}

async fn github_fallback_check(channel_feed: &str) -> Result<UpdateInfo, String> {
    let url = "https://api.github.com/repos/sbhooley/armaraos/releases/latest";
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let rel: GithubLatestRelease = client
        .get(url)
        .header("User-Agent", "ArmaraOS-Updater")
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;

    let current = semver::Version::parse(env!("CARGO_PKG_VERSION"))
        .map_err(|e| format!("current version parse: {e}"))?;
    let tag = rel.tag_name.trim().trim_start_matches('v');
    let latest = semver::Version::parse(tag).map_err(|e| format!("github version parse: {e}"))?;

    let available = latest > current;
    Ok(UpdateInfo {
        available,
        version: Some(latest.to_string()),
        body: rel.body,
        source: "github".to_string(),
        download_url: Some(rel.html_url),
        installable: false,
        feed_url: format!("{channel_feed} (fallback: GitHub API)"),
    })
}
