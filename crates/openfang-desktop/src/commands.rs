//! Tauri IPC command handlers.

use crate::{KernelState, PortState};
use openfang_kernel::config::openfang_home;
use std::io::Read;
#[cfg(target_os = "macos")]
use std::io::Write;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tauri::Manager;
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_dialog::DialogExt;
use tracing::info;
#[cfg(target_os = "macos")]
use tracing::warn;

/// Return AINL bootstrap status (Option A bundling).
#[tauri::command]
pub fn ainl_status(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let st = crate::ainl::ainl_status(&app)?;
    serde_json::to_value(st).map_err(|e| e.to_string())
}

/// Ensure AINL is installed into the internal app-managed venv (Option A bundling).
#[tauri::command]
pub fn ensure_ainl_installed(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let st = crate::ainl::ensure_ainl_installed(&app)?;
    serde_json::to_value(st).map_err(|e| e.to_string())
}

/// Register bundled `ainl-mcp` + `~/.armaraos/bin/ainl-run` like `ainl install armaraos` (no PyPI step).
#[tauri::command]
pub fn ensure_armaraos_ainl_host(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let st = crate::ainl::ensure_armaraos_ainl_host(&app)?;
    serde_json::to_value(st).map_err(|e| e.to_string())
}

/// PyPI + GitHub `main` SHA comparison for the internal `ainativelang` install.
#[tauri::command]
pub fn ainl_check_versions(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let info = crate::ainl_version::ainl_version_info(&app);
    serde_json::to_value(info).map_err(|e| e.to_string())
}

/// `pip install --upgrade` for the configured AINL spec, then refresh host + library sync.
#[tauri::command]
pub fn upgrade_ainl_pip(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let st = crate::ainl::upgrade_ainl_pip(&app)?;
    serde_json::to_value(st).map_err(|e| e.to_string())
}

/// Get the port the embedded server is listening on.
#[tauri::command]
pub fn get_port(port: tauri::State<'_, PortState>) -> u16 {
    port.0
}

/// Get a status summary of the running kernel.
#[tauri::command]
pub fn get_status(
    port: tauri::State<'_, PortState>,
    kernel_state: tauri::State<'_, KernelState>,
) -> serde_json::Value {
    let agents = kernel_state.kernel.registry.list().len();
    let uptime_secs = kernel_state.started_at.elapsed().as_secs();

    serde_json::json!({
        "status": "running",
        "port": port.0,
        "agents": agents,
        "uptime_secs": uptime_secs,
    })
}

/// Get the number of registered agents.
#[tauri::command]
pub fn get_agent_count(kernel_state: tauri::State<'_, KernelState>) -> usize {
    kernel_state.kernel.registry.list().len()
}

/// Open a native file picker to import an agent TOML manifest.
///
/// Validates the TOML as a valid `AgentManifest`, copies it to
/// `~/.openfang/agents/{name}/agent.toml`, then spawns the agent.
#[tauri::command]
pub fn import_agent_toml(
    app: tauri::AppHandle,
    kernel_state: tauri::State<'_, KernelState>,
) -> Result<String, String> {
    let path = app
        .dialog()
        .file()
        .set_title("Import Agent Manifest")
        .add_filter("TOML files", &["toml"])
        .blocking_pick_file();

    let file_path = match path {
        Some(p) => p,
        None => return Err("No file selected".to_string()),
    };

    let content = std::fs::read_to_string(file_path.as_path().ok_or("Invalid file path")?)
        .map_err(|e| format!("Failed to read file: {e}"))?;

    let manifest: openfang_types::agent::AgentManifest =
        toml::from_str(&content).map_err(|e| format!("Invalid agent manifest: {e}"))?;

    let agent_name = manifest.name.clone();
    let agent_dir = openfang_home().join("agents").join(&agent_name);
    std::fs::create_dir_all(&agent_dir)
        .map_err(|e| format!("Failed to create agent directory: {e}"))?;

    let dest = agent_dir.join("agent.toml");
    std::fs::write(&dest, &content).map_err(|e| format!("Failed to write manifest: {e}"))?;

    kernel_state
        .kernel
        .spawn_agent(manifest)
        .map_err(|e| format!("Failed to spawn agent: {e}"))?;

    info!("Imported and spawned agent \"{agent_name}\"");
    Ok(agent_name)
}

/// Open a native file picker to import a skill file.
///
/// Copies the selected file to `~/.openfang/skills/` and triggers a
/// hot-reload of the skill registry.
#[tauri::command]
pub fn import_skill_file(
    app: tauri::AppHandle,
    kernel_state: tauri::State<'_, KernelState>,
) -> Result<String, String> {
    let path = app
        .dialog()
        .file()
        .set_title("Import Skill File")
        .add_filter("Skill files", &["md", "toml", "py", "js", "wasm"])
        .blocking_pick_file();

    let file_path = match path {
        Some(p) => p,
        None => return Err("No file selected".to_string()),
    };

    let src = file_path.as_path().ok_or("Invalid file path")?;
    let file_name = src
        .file_name()
        .ok_or("No filename")?
        .to_string_lossy()
        .to_string();

    let skills_dir = openfang_home().join("skills");
    std::fs::create_dir_all(&skills_dir)
        .map_err(|e| format!("Failed to create skills directory: {e}"))?;

    let dest = skills_dir.join(&file_name);
    std::fs::copy(src, &dest).map_err(|e| format!("Failed to copy skill file: {e}"))?;

    kernel_state.kernel.reload_skills();

    info!("Imported skill file \"{file_name}\" and reloaded registry");
    Ok(file_name)
}

/// Check whether auto-start on login is enabled.
#[tauri::command]
pub fn get_autostart(app: tauri::AppHandle) -> Result<bool, String> {
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

/// Enable or disable auto-start on login.
#[tauri::command]
pub fn set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<bool, String> {
    let manager = app.autolaunch();
    if enabled {
        manager.enable().map_err(|e| e.to_string())?;
    } else {
        manager.disable().map_err(|e| e.to_string())?;
    }
    manager.is_enabled().map_err(|e| e.to_string())
}

/// Perform an on-demand update check.
#[tauri::command]
pub async fn check_for_updates(
    app: tauri::AppHandle,
    kernel_state: tauri::State<'_, KernelState>,
) -> Result<crate::updater::UpdateInfo, String> {
    kernel_state.kernel.audit_log.record(
        openfang_kernel::kernel::shared_memory_agent_id().to_string(),
        openfang_runtime::audit::AuditAction::UpdateCheck,
        "desktop_check_for_updates",
        "started",
    );
    match crate::updater::check_for_update(&app).await {
        Ok(info) => {
            let outcome = if info.available {
                format!(
                    "available v{} (source={}, installable={})",
                    info.version
                        .clone()
                        .unwrap_or_else(|| "unknown".to_string()),
                    info.source,
                    info.installable
                )
            } else {
                "up_to_date".to_string()
            };
            kernel_state.kernel.audit_log.record(
                openfang_kernel::kernel::shared_memory_agent_id().to_string(),
                openfang_runtime::audit::AuditAction::UpdateCheck,
                "desktop_check_for_updates",
                outcome,
            );
            Ok(info)
        }
        Err(e) => {
            kernel_state.kernel.audit_log.record(
                openfang_kernel::kernel::shared_memory_agent_id().to_string(),
                openfang_runtime::audit::AuditAction::UpdateCheck,
                "desktop_check_for_updates",
                openfang_types::truncate_str(&e, 400),
            );
            Err(e)
        }
    }
}

/// Download and install the latest update, then restart the app.
/// Returns Ok(()) which triggers an app restart — the command will not return
/// if the update succeeds (the app restarts). On error, returns Err(message).
#[tauri::command]
pub async fn install_update(
    app: tauri::AppHandle,
    kernel_state: tauri::State<'_, KernelState>,
) -> Result<(), String> {
    kernel_state.kernel.audit_log.record(
        openfang_kernel::kernel::shared_memory_agent_id().to_string(),
        openfang_runtime::audit::AuditAction::UpdateInstall,
        "desktop_install_update",
        "started",
    );
    match crate::updater::download_and_install_update(&app).await {
        Ok(()) => Ok(()),
        Err(e) => {
            kernel_state.kernel.audit_log.record(
                openfang_kernel::kernel::shared_memory_agent_id().to_string(),
                openfang_runtime::audit::AuditAction::UpdateInstall,
                "desktop_install_update",
                openfang_types::truncate_str(&e, 400),
            );
            Err(e)
        }
    }
}

/// POST to the embedded API to build a redacted diagnostics tarball (loopback auth bypass).
pub async fn post_support_bundle(port: u16) -> Result<serde_json::Value, String> {
    let url = format!("http://127.0.0.1:{port}/api/support/diagnostics");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;
    let res = client
        .post(&url)
        .header("Content-Type", "application/json")
        .body("{}")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = res.status();
    let body = res.bytes().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!(
            "Diagnostics failed ({}): {}",
            status,
            String::from_utf8_lossy(&body)
        ));
    }
    serde_json::from_slice(&body).map_err(|e| e.to_string())
}

/// Generate support bundle via local API (desktop shell menu + dashboard invoke).
#[tauri::command]
pub async fn generate_support_bundle(
    port: tauri::State<'_, PortState>,
) -> Result<serde_json::Value, String> {
    post_support_bundle(port.0).await
}

/// Desktop updater prefs (channel, feed URLs, last check / last error).
#[tauri::command]
pub fn get_desktop_updater_prefs(app: tauri::AppHandle) -> serde_json::Value {
    crate::ui_prefs::updater_prefs_snapshot(&app)
}

/// Set stable vs beta updater channel (writes `desktop_ui_prefs.json`).
#[tauri::command]
pub fn set_release_channel(app: tauri::AppHandle, channel: String) -> Result<(), String> {
    crate::ui_prefs::save_release_channel(&app, &channel)
}

/// Persist last daemon vs-GitHub check (desktop shell).
#[tauri::command]
pub fn report_daemon_update_check(app: tauri::AppHandle, error: Option<String>) {
    crate::ui_prefs::record_daemon_update_check(&app, error.as_deref());
}

/// Open the ArmaraOS config directory (`~/.armaraos/`, or legacy `~/.openfang/`) in the OS file manager.
#[tauri::command]
pub fn open_config_dir() -> Result<(), String> {
    let dir = openfang_home();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create config dir: {e}"))?;
    open::that(&dir).map_err(|e| format!("Failed to open directory: {e}"))
}

/// Open the ArmaraOS logs directory (`~/.armaraos/logs/`, or legacy `~/.openfang/logs/`) in the OS file manager.
#[tauri::command]
pub fn open_logs_dir() -> Result<(), String> {
    let dir = openfang_home().join("logs");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create logs dir: {e}"))?;
    open::that(&dir).map_err(|e| format!("Failed to open directory: {e}"))
}

/// Open the OS UI where notification permissions are configured (macOS / Windows).
#[tauri::command]
pub fn open_notification_settings(_app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.notifications")
            .status()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "ms-settings:notifications"])
            .status()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Err("Open your system settings and enable notifications for this application.".to_string())
    }
}

/// Open `~/.armaraos/ainl-library/` (mirrored AINL demo/examples/intelligence from upstream).
#[tauri::command]
pub fn open_ainl_library_dir() -> Result<(), String> {
    let dir = openfang_home().join("ainl-library");
    if !dir.is_dir() {
        return Err(
            "AINL library folder not found yet. Run AINL bootstrap with network access once."
                .to_string(),
        );
    }
    open::that(&dir).map_err(|e| format!("Failed to open directory: {e}"))
}

const AINL_TRY_TIMEOUT_TOKEN: &str = "__armaraos_ainl_timeout__";

fn run_ainl_subprocess_with_timeout(
    mut cmd: Command,
    timeout: Duration,
) -> Result<std::process::Output, String> {
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn ainl: {e}"))?;
    let mut stdout_h = child.stdout.take().unwrap();
    let mut stderr_h = child.stderr.take().unwrap();
    let t1 = thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout_h.read_to_string(&mut s);
        s
    });
    let t2 = thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr_h.read_to_string(&mut s);
        s
    });
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let o = t1.join().unwrap_or_default();
                let e = t2.join().unwrap_or_default();
                return Ok(std::process::Output {
                    status,
                    stdout: o.into_bytes(),
                    stderr: e.into_bytes(),
                });
            }
            Ok(None) => {
                if Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = t1.join();
                    let _ = t2.join();
                    return Err(AINL_TRY_TIMEOUT_TOKEN.to_string());
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(format!("wait: {e}")),
        }
    }
}

/// Run `ainl validate --strict` or `ainl run` on a file under `~/.armaraos/ainl-library/` (path must resolve inside the mirror).
///
/// `timeout_secs` optional; defaults 120 (validate) / 300 (run), clamped 5–600. On timeout returns JSON with `timed_out: true` and `suggested_command`.
#[tauri::command]
pub fn ainl_try_library_file(
    app: tauri::AppHandle,
    relative_path: String,
    mode: Option<String>,
    timeout_secs: Option<u64>,
    strict: Option<bool>,
) -> Result<serde_json::Value, String> {
    let home = openfang_home();
    let abs = openfang_kernel::ainl_library::resolve_program_under_ainl_library(
        &home,
        relative_path.trim(),
    )?;
    let venv = crate::ainl::venv_dir(&app)?;
    let ainl = crate::ainl::venv_bin(&venv, "ainl");
    if !ainl.exists() {
        return Err(
            "AINL CLI is missing from the app virtualenv. Open Settings → AINL and run Bootstrap."
                .to_string(),
        );
    }
    let cwd = home.join("ainl-library");
    let mode = mode.as_deref().unwrap_or("validate").trim().to_lowercase();
    let strict = strict.unwrap_or(true);
    let default_secs = if mode == "run" { 300u64 } else { 120u64 };
    let timeout_secs = timeout_secs.unwrap_or(default_secs).clamp(5, 600);
    let timeout = Duration::from_secs(timeout_secs);

    let mut cmd = Command::new(&ainl);
    cmd.current_dir(&cwd);
    cmd.env("PATH", crate::ainl::subprocess_path_with_venv_bin(&venv));
    cmd.env("AINL_ALLOW_IR_DECLARED_ADAPTERS", "1");
    match mode.as_str() {
        "run" => {
            cmd.arg("run");
            cmd.arg("--enable-adapter");
            cmd.arg("http");
            cmd.arg(&abs);
        }
        _ => {
            cmd.arg("validate");
            if strict {
                cmd.arg("--strict");
            }
            cmd.arg(&abs);
        }
    }

    let suggested_command = format!(
        "cd \"{}\" && \"{}\" {} \"{}\"",
        cwd.display(),
        ainl.display(),
        if mode == "run" {
            "run --enable-adapter http"
        } else {
            if strict {
                "validate --strict"
            } else {
                "validate"
            }
        },
        abs.display()
    );

    match run_ainl_subprocess_with_timeout(cmd, timeout) {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            Ok(serde_json::json!({
                "ok": out.status.success(),
                "exit_code": out.status.code(),
                "mode": mode,
                "strict": strict,
                "path": relative_path.trim(),
                "stdout": stdout,
                "stderr": stderr,
                "timed_out": false,
                "timeout_secs": timeout_secs,
                "suggested_command": suggested_command,
            }))
        }
        Err(e) if e == AINL_TRY_TIMEOUT_TOKEN => Ok(serde_json::json!({
            "ok": false,
            "timed_out": true,
            "timeout_secs": timeout_secs,
            "mode": mode,
            "strict": strict,
            "path": relative_path.trim(),
            "stdout": "",
            "stderr": "",
            "suggested_command": suggested_command,
        })),
        Err(e) => Err(e),
    }
}

/// Persist dashboard theme mode (`light` | `dark` | `system`) for the next app launch.
#[tauri::command]
pub fn set_dashboard_theme_mode(app: tauri::AppHandle, mode: String) -> Result<(), String> {
    crate::ui_prefs::save_theme_mode(&app, &mode)?;
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.set_theme(crate::ui_prefs::window_theme_for_mode(&mode));
    }
    Ok(())
}

/// Load chat bookmarks JSON saved on disk (desktop shell; survives random localhost port / WebView storage).
#[tauri::command]
pub fn get_dashboard_bookmarks(app: tauri::AppHandle) -> Result<Option<String>, String> {
    crate::ui_prefs::load_dashboard_bookmarks_json(&app)
}

/// Persist chat bookmarks JSON (same schema as dashboard `armaraos-bookmarks-v1`).
#[tauri::command]
pub fn set_dashboard_bookmarks(app: tauri::AppHandle, json: String) -> Result<(), String> {
    crate::ui_prefs::save_dashboard_bookmarks_json(&app, &json)
}

/// Open a whitelisted HTTPS URL in the system default browser (Tauri webview `target=_blank` is unreliable).
#[tauri::command]
pub fn open_external_url(url: String) -> Result<(), String> {
    if url.len() > 2048 {
        return Err("Invalid URL".to_string());
    }
    if url.starts_with("mailto:") && url.contains('@') && url.len() <= 4096 {
        return open::that(&url).map_err(|e| e.to_string());
    }
    let ok = url == "https://ainativelang.com"
        || url.starts_with("https://ainativelang.com/")
        || url == "https://github.com/sbhooley/armaraos/releases"
        || url.starts_with("https://github.com/sbhooley/armaraos/releases/")
        || url.starts_with("https://github.com/sbhooley/ainativelang")
        || url.starts_with("https://x.com/ainativelang")
        || url.starts_with("https://t.me/")
        || url.starts_with("https://pypi.org/")
        || url.starts_with("https://www.python.org/")
        || url.starts_with("https://python.org/");
    if !ok {
        return Err("Invalid URL".to_string());
    }
    open::that(&url).map_err(|e| e.to_string())
}

/// Return a destination path that does not already exist by appending `(N)` before the extension.
///
/// `dir/file.zip` → `dir/file (2).zip` → `dir/file (3).zip` …
fn unique_dest(dir: &std::path::Path, fname: &str) -> std::path::PathBuf {
    let candidate = dir.join(fname);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match fname.rfind('.') {
        Some(dot) => (&fname[..dot], &fname[dot..]),
        None => (fname, ""),
    };
    for n in 2u32.. {
        let name = format!("{stem} ({n}){ext}");
        let p = dir.join(&name);
        if !p.exists() {
            return p;
        }
    }
    // Unreachable in practice
    dir.join(fname)
}

/// Same rules as `openfang_api::routes::is_allowed_diagnostics_zip_name` (no path segments).
fn diagnostics_zip_filename_ok(name: &str) -> bool {
    const PREFIX: &str = "armaraos-diagnostics-";
    const SUFFIX: &str = ".zip";
    if !name.starts_with(PREFIX) || !name.ends_with(SUFFIX) {
        return false;
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return false;
    }
    let inner = &name[PREFIX.len()..name.len() - SUFFIX.len()];
    let parts: Vec<&str> = inner.splitn(2, '-').collect();
    if parts.len() != 2 {
        return false;
    }
    if parts[0].len() != 8 || parts[1].len() != 6 {
        return false;
    }
    parts[0].chars().all(|c| c.is_ascii_digit()) && parts[1].chars().all(|c| c.is_ascii_digit())
}

fn resolve_support_bundle_for_copy(
    bundle_path: Option<String>,
    bundle_filename: Option<String>,
) -> Result<std::path::PathBuf, String> {
    let support = openfang_home().join("support");
    if let Some(name) = bundle_filename
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        if diagnostics_zip_filename_ok(name) {
            let p = support.join(name);
            if p.is_file() {
                return Ok(p);
            }
        }
    }
    if let Some(path) = bundle_path
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        return validate_support_bundle_path(path);
    }
    if let Some(name) = bundle_filename
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        if diagnostics_zip_filename_ok(name) {
            return Err(format!(
                "Diagnostics bundle not found in support folder: {name}"
            ));
        }
    }
    Err("Provide bundle_filename or bundle_path".to_string())
}

/// Ensure `user_path` is a real `.zip` under `openfang_home()/support/`.
fn validate_support_bundle_path(user_path: &str) -> Result<std::path::PathBuf, String> {
    let p = std::path::Path::new(user_path);
    let meta = std::fs::canonicalize(p).map_err(|e| format!("Invalid bundle path: {e}"))?;
    let support = openfang_home().join("support");
    std::fs::create_dir_all(&support).map_err(|e| format!("support dir: {e}"))?;
    let support_canon = std::fs::canonicalize(&support).unwrap_or(support);
    if !meta.starts_with(&support_canon) {
        return Err("Path must be under the ArmaraOS support directory".to_string());
    }
    if !meta.is_file() {
        return Err("Not a file".to_string());
    }
    let lossy = meta.to_string_lossy();
    if !lossy.ends_with(".zip") {
        return Err("Expected a .zip diagnostics bundle".to_string());
    }
    Ok(meta)
}

/// Copy a generated diagnostics zip into the user Downloads folder (same filename).
/// Single `bundle_path` string matches Tauri IPC schema (`bundlePath` from JS).
#[tauri::command]
pub fn copy_diagnostics_to_downloads(bundle_path: String) -> Result<serde_json::Value, String> {
    let trimmed = bundle_path.trim().to_string();
    if trimmed.is_empty() {
        return Err("bundlePath is required".to_string());
    }
    let name_only = std::path::Path::new(&trimmed)
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| diagnostics_zip_filename_ok(n))
        .map(String::from);
    let canon = resolve_support_bundle_for_copy(Some(trimmed), name_only)?;
    let fname = canon
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| "Bad filename".to_string())?;
    let dest_dir =
        dirs::download_dir().ok_or_else(|| "Could not locate Downloads folder".to_string())?;
    let dest = unique_dest(&dest_dir, fname);
    std::fs::copy(&canon, &dest).map_err(|e| format!("Copy to Downloads failed: {e}"))?;
    Ok(serde_json::json!({
        "downloads_path": dest.display().to_string(),
        "bundle_path": canon.display().to_string(),
    }))
}

fn validate_home_relative_file(rel: &str) -> Result<std::path::PathBuf, String> {
    let trimmed = rel.trim().trim_start_matches(['/', '\\']);
    if trimmed.is_empty() || trimmed.contains("..") {
        return Err("Invalid path".to_string());
    }
    let home = openfang_home();
    let full = home.join(trimmed);
    let meta = std::fs::canonicalize(&full).map_err(|e| format!("Invalid path: {e}"))?;
    let home_canon = std::fs::canonicalize(&home).unwrap_or(home);
    if !meta.starts_with(&home_canon) {
        return Err("Path must be under the ArmaraOS home directory".to_string());
    }
    if !meta.is_file() {
        return Err("Not a file".to_string());
    }
    Ok(meta)
}

/// Copy any file under ArmaraOS home (relative path like `support/foo.zip`) to Downloads.
#[tauri::command]
pub fn copy_home_file_to_downloads(relative_path: String) -> Result<serde_json::Value, String> {
    let canon = validate_home_relative_file(&relative_path)?;
    let fname = canon
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| "Bad filename".to_string())?;
    let dest_dir =
        dirs::download_dir().ok_or_else(|| "Could not locate Downloads folder".to_string())?;
    let dest = unique_dest(&dest_dir, fname);
    std::fs::copy(&canon, &dest).map_err(|e| format!("Copy to Downloads failed: {e}"))?;
    Ok(serde_json::json!({
        "downloads_path": dest.display().to_string(),
        "source_path": canon.display().to_string(),
    }))
}

#[cfg(target_os = "macos")]
fn try_compose_mail_macos_attachment(bundle: &std::path::Path) -> Result<(), String> {
    let path_str = bundle
        .to_str()
        .ok_or_else(|| "Invalid path encoding".to_string())?;
    // Script on stdin: argv[1] is the zip path. Attachment must be created on the outgoing
    // message (not inside a silent try); Mail.app ignores failed attachments otherwise.
    // NSAppleEventsUsageDescription is required for Automation permission on modern macOS.
    const SCRIPT: &[u8] = br#"on run argv
	set bundlePath to item 1 of argv
	set theFile to POSIX file bundlePath
	tell application "Mail"
		activate
		set newMessage to make new outgoing message with properties {visible:true, subject:"ArmaraOS Support - Bug Report", content:"Describe the bug here:" & return & return & "Thanks!"}
		tell newMessage
			make new to recipient at end of to recipients with properties {address:"ainativelang@gmail.com"}
			make new attachment with properties {file name:theFile}
		end tell
	end tell
end run
"#;
    // Pass the zip path as the sole script argument (not after `--`, or argv[1] becomes `--`).
    let mut child = Command::new("osascript")
        .arg("-")
        .arg(path_str)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not start osascript: {e}"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "osascript stdin".to_string())?;
    stdin
        .write_all(SCRIPT)
        .map_err(|e| format!("osascript stdin: {e}"))?;
    drop(stdin);
    let output = child
        .wait_with_output()
        .map_err(|e| format!("osascript wait: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        warn!(target: "openfang_desktop", "Mail AppleScript failed: {}", stderr.trim());
        Err(format!(
            "Could not compose in Mail (install Mail, grant Automation in Privacy & Security, or attach the zip manually). {}",
            stderr.trim()
        ))
    }
}

#[cfg(target_os = "linux")]
fn try_compose_mail_linux_attachment(bundle: &std::path::Path) -> Result<(), String> {
    let p = bundle.to_str().ok_or_else(|| "Invalid path".to_string())?;
    let status = Command::new("xdg-email")
        .args([
            "--subject",
            "ArmaraOS Support — Bug Report",
            "--body",
            "Describe the bug here. The diagnostics .zip is attached.\n\nThanks!",
            "--attach",
            p,
            "ainativelang@gmail.com",
        ])
        .status()
        .map_err(|e| format!("xdg-email: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("xdg-email failed (install xdg-utils or use Get Help without attachment)".to_string())
    }
}

/// Open the default mail client. On macOS/Linux with a valid bundle path, tries to attach the zip (Mail / xdg-email).
#[tauri::command]
pub fn compose_support_email(bundle_path: Option<String>) -> Result<serde_json::Value, String> {
    let body_txt = "Describe the bug here:\n\nIf no file is attached, please attach the armaraos-diagnostics-*.zip from your Downloads folder or from Home folder → support.\n\nThanks!";
    let mailto = format!(
        "mailto:ainativelang@gmail.com?subject={}&body={}",
        urlencoding::encode("ArmaraOS Support — Bug Report"),
        urlencoding::encode(body_txt)
    );
    if let Some(ref raw) = bundle_path {
        let t = raw.trim();
        if !t.is_empty() {
            if let Ok(canon) = validate_support_bundle_path(t) {
                #[cfg(target_os = "macos")]
                match try_compose_mail_macos_attachment(&canon) {
                    Ok(()) => return Ok(serde_json::json!({"mode": "apple_mail"})),
                    Err(_) => {
                        open::that(&mailto).map_err(|e| e.to_string())?;
                        return Ok(serde_json::json!({
                            "mode": "mailto",
                            "attach_failed": true,
                        }));
                    }
                }
                #[cfg(target_os = "linux")]
                match try_compose_mail_linux_attachment(&canon) {
                    Ok(()) => return Ok(serde_json::json!({"mode": "xdg_email"})),
                    Err(_) => {
                        open::that(&mailto).map_err(|e| e.to_string())?;
                        return Ok(serde_json::json!({
                            "mode": "mailto",
                            "attach_failed": true,
                        }));
                    }
                }
                #[cfg(not(any(target_os = "macos", target_os = "linux")))]
                let _ = canon;
            }
        }
    }
    open::that(&mailto).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({"mode": "mailto"}))
}

/// Desktop telemetry prefs for the Setup Wizard (opt-out before first PostHog ping).
#[tauri::command]
pub fn get_desktop_product_analytics_prefs(
    app: tauri::AppHandle,
) -> Result<serde_json::Value, String> {
    Ok(crate::product_analytics::prefs_json(&app))
}

/// `opt_out`: user disabled anonymous install ping. `from_wizard_continue`: true when leaving wizard step 1.
#[tauri::command]
pub fn set_desktop_product_analytics_prefs(
    opt_out: bool,
    from_wizard_continue: bool,
    app: tauri::AppHandle,
) -> Result<(), String> {
    crate::product_analytics::save_prefs_merged(opt_out, from_wizard_continue, &app)
}
