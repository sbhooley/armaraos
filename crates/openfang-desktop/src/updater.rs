//! Update checker for the ArmaraOS desktop app.

use serde::{Deserialize, Serialize};
use tauri_plugin_notification::NotificationExt;
use url::Url;

use crate::notification_icon::apply_notification_icon;
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
                let _ = apply_notification_icon(
                    app_handle
                        .notification()
                        .builder()
                        .title("ArmaraOS Updating...")
                        .body(format!("Installing v{version}. App will restart shortly.")),
                )
                .show();
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
                let _ = apply_notification_icon(
                    app_handle
                        .notification()
                        .builder()
                        .title("ArmaraOS Update Available")
                        .body(format!("v{version} is available. Download: {url}")),
                )
                .show();
            }
            Ok(_) => info!("No updates available"),
            Err(e) => warn!("Startup update check failed: {e}"),
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
        Ok(None) => Ok(UpdateInfo {
            available: false,
            version: None,
            body: None,
            source: "website".to_string(),
            download_url: None,
            installable: false,
            feed_url: feed.to_string(),
        }),
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
    let latest =
        semver::Version::parse(tag).map_err(|e| format!("github version parse: {e}"))?;

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
