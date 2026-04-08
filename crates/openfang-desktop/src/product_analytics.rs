//! PostHog product analytics (desktop): optional one-time `armaraos_desktop_first_open` event.
//!
//! - **API key**: compile-time `ARMARAOS_POSTHOG_KEY` (release CI) or runtime env override for dev.
//! - **Host**: `ARMARAOS_POSTHOG_HOST` (compile-time or runtime), default US ingest.
//! - **Opt-out**: persisted in `desktop_telemetry_prefs.json`; Setup Wizard step 1 can opt out before any ping.
//! - **Timing**: send only after **120s** *or* wizard “Get Started” with analytics allowed (`consent_instant`).
//! - **Kill switch**: `ARMARAOS_PRODUCT_ANALYTICS=0`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration as StdDuration, Instant as StdInstant};
use tauri::{AppHandle, Manager};
use tokio::time::{interval, Duration};
use tracing::{debug, warn};

const PREFS_FILE: &str = "desktop_telemetry_prefs.json";
const STATE_FILE: &str = "product_analytics_state.json";

#[derive(Debug, Default, Serialize, Deserialize)]
struct TelemetryPrefs {
    #[serde(default)]
    opt_out: bool,
    /// User continued from wizard step 1 — allow ping without waiting full deferral.
    #[serde(default)]
    consent_instant: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SendState {
    distinct_id: Option<String>,
    #[serde(default)]
    first_open_sent: bool,
}

fn baked_posthog_key() -> Option<&'static str> {
    option_env!("ARMARAOS_POSTHOG_KEY").and_then(|s| {
        let t = s.trim();
        if t.is_empty() {
            None
        } else {
            Some(t)
        }
    })
}

fn baked_posthog_host() -> Option<&'static str> {
    option_env!("ARMARAOS_POSTHOG_HOST").and_then(|s| {
        let t = s.trim();
        if t.is_empty() {
            None
        } else {
            Some(t)
        }
    })
}

fn resolve_posthog_key() -> Option<String> {
    if let Ok(s) = std::env::var("ARMARAOS_POSTHOG_KEY") {
        let t = s.trim();
        if !t.is_empty() {
            return Some(t.to_string());
        }
    }
    baked_posthog_key().map(|s| s.to_string())
}

fn resolve_posthog_host() -> String {
    if let Ok(s) = std::env::var("ARMARAOS_POSTHOG_HOST") {
        let t = s.trim();
        if !t.is_empty() {
            return t.to_string();
        }
    }
    baked_posthog_host()
        .unwrap_or("https://us.i.posthog.com")
        .trim_end_matches('/')
        .to_string()
}

fn disabled_by_env() -> bool {
    matches!(
        std::env::var("ARMARAOS_PRODUCT_ANALYTICS")
            .map(|v| v == "0" || v.eq_ignore_ascii_case("false")),
        Ok(true)
    )
}

fn app_data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path().app_data_dir().map_err(|e| e.to_string())
}

fn prefs_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_dir(app)?.join(PREFS_FILE))
}

fn state_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_dir(app)?.join(STATE_FILE))
}

fn load_prefs_file(path: &Path) -> Option<TelemetryPrefs> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn save_prefs_file(path: &Path, prefs: &TelemetryPrefs) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    let bytes =
        serde_json::to_vec_pretty(prefs).map_err(|e| format!("serialize telemetry prefs: {e}"))?;
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(())
}

/// Merge-update telemetry prefs from the dashboard wizard.
pub fn save_prefs_merged(
    opt_out: bool,
    from_wizard_continue: bool,
    app: &AppHandle,
) -> Result<(), String> {
    let path = prefs_path(app)?;
    let mut p = load_prefs_file(&path).unwrap_or_default();
    p.opt_out = opt_out;
    if from_wizard_continue {
        p.consent_instant = true;
    }
    save_prefs_file(&path, &p)
}

pub fn prefs_json(app: &AppHandle) -> serde_json::Value {
    let opt_out = prefs_path(app)
        .ok()
        .and_then(|p| load_prefs_file(&p))
        .map(|p| p.opt_out)
        .unwrap_or(false);
    serde_json::json!({ "opt_out": opt_out })
}

fn user_opted_out(app: &AppHandle) -> bool {
    prefs_path(app)
        .ok()
        .and_then(|p| load_prefs_file(&p))
        .map(|p| p.opt_out)
        .unwrap_or(false)
}

fn consent_instant(app: &AppHandle) -> bool {
    prefs_path(app)
        .ok()
        .and_then(|p| load_prefs_file(&p))
        .map(|p| p.consent_instant)
        .unwrap_or(false)
}

fn load_send_state(path: &Path) -> Option<SendState> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn save_send_state(path: &Path, state: &SendState) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(state)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn first_open_already_sent(app: &AppHandle) -> bool {
    state_path(app)
        .ok()
        .and_then(|p| load_send_state(&p))
        .map(|s| s.first_open_sent)
        .unwrap_or(false)
}

/// Returns `true` if the worker should stop (sent, opted out, or give up).
#[cfg(desktop)]
async fn try_send_first_open_once(app: &AppHandle, app_version: &str) -> bool {
    if disabled_by_env() || user_opted_out(app) {
        return true;
    }
    let Some(api_key) = resolve_posthog_key() else {
        debug!(target: "openfang_desktop", "product analytics: no PostHog key (set ARMARAOS_POSTHOG_KEY at build or runtime)");
        return true;
    };

    let host = resolve_posthog_host();
    let state_p = match state_path(app) {
        Ok(p) => p,
        Err(e) => {
            warn!(target: "openfang_desktop", "product analytics: state path: {e}");
            return true;
        }
    };

    let mut state = load_send_state(&state_p).unwrap_or_default();
    if state.first_open_sent {
        return true;
    }

    let distinct_id = state
        .distinct_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    state.distinct_id = Some(distinct_id.clone());
    if let Err(e) = save_send_state(&state_p, &state) {
        warn!(target: "openfang_desktop", "product analytics: save state: {e}");
        return false;
    }

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            warn!(target: "openfang_desktop", "product analytics: http client: {e}");
            return false;
        }
    };

    let mut props = serde_json::json!({
        "distinct_id": distinct_id,
        "$lib": "armaraos-desktop",
        "app_version": app_version,
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
    });
    if let Ok(src) = std::env::var("ARMARAOS_INSTALL_SOURCE") {
        let t = src.trim();
        if !t.is_empty() {
            if let Some(obj) = props.as_object_mut() {
                obj.insert(
                    "install_source".to_string(),
                    serde_json::Value::String(t.to_string()),
                );
            }
        }
    }

    let body = serde_json::json!({
        "api_key": api_key,
        "event": "armaraos_desktop_first_open",
        "properties": props,
    });

    let url = format!("{host}/capture/");
    match client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => {
            state.first_open_sent = true;
            if let Err(e) = save_send_state(&state_p, &state) {
                warn!(target: "openfang_desktop", "product analytics: save after send: {e}");
            }
            true
        }
        Ok(r) => {
            warn!(
                target: "openfang_desktop",
                "product analytics: PostHog HTTP {}",
                r.status()
            );
            false
        }
        Err(e) => {
            warn!(target: "openfang_desktop", "product analytics: request failed: {e}");
            false
        }
    }
}

#[cfg(not(desktop))]
async fn try_send_first_open_once(_app: &AppHandle, _app_version: &str) -> bool {
    true
}

pub fn spawn_first_open_worker(app: &AppHandle) {
    #[cfg(not(desktop))]
    {
        let _ = app;
        return;
    }
    #[cfg(desktop)]
    {
        let app_version = app.package_info().version.to_string();
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            run_first_open_worker_loop(handle, app_version).await;
        });
    }
}

#[cfg(desktop)]
async fn run_first_open_worker_loop(app: AppHandle, app_version: String) {
    let start = StdInstant::now();
    let max_wait = StdDuration::from_secs(120);
    let mut tick = interval(Duration::from_secs(2));
    let mut failures_since_last_success: u32 = 0;

    loop {
        tick.tick().await;

        if disabled_by_env() {
            return;
        }
        if resolve_posthog_key().is_none() {
            return;
        }
        if user_opted_out(&app) {
            return;
        }
        if first_open_already_sent(&app) {
            return;
        }

        let instant = consent_instant(&app);
        if !instant && start.elapsed() < max_wait {
            continue;
        }

        let done = try_send_first_open_once(&app, &app_version).await;
        if done {
            return;
        }
        failures_since_last_success += 1;
        // ~10 minutes of 2s ticks, then stop retrying (offline / blocked).
        if failures_since_last_success >= 300 {
            warn!(target: "openfang_desktop", "product analytics: giving up after repeated failures");
            return;
        }
    }
}
