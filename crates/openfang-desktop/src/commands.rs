//! Tauri IPC command handlers.

use crate::{KernelState, PortState};
use openfang_kernel::config::openfang_home;
use std::io::Read;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_dialog::DialogExt;
use tracing::info;

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
                    info.version.clone().unwrap_or_else(|| "unknown".to_string()),
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
pub async fn generate_support_bundle(port: tauri::State<'_, PortState>) -> Result<serde_json::Value, String> {
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
    match mode.as_str() {
        "run" => {
            cmd.arg("run");
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
            "run"
        } else {
            if strict { "validate --strict" } else { "validate" }
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
    crate::ui_prefs::save_theme_mode(&app, &mode)
}

/// Open a whitelisted HTTPS URL in the system default browser (Tauri webview `target=_blank` is unreliable).
#[tauri::command]
pub fn open_external_url(url: String) -> Result<(), String> {
    if url.len() > 2048 {
        return Err("Invalid URL".to_string());
    }
    let ok = url == "https://ainativelang.com"
        || url.starts_with("https://ainativelang.com/")
        || url == "https://github.com/sbhooley/armaraos/releases"
        || url.starts_with("https://github.com/sbhooley/armaraos/releases/")
        || url.starts_with("https://www.python.org/")
        || url.starts_with("https://python.org/");
    if !ok {
        return Err("Invalid URL".to_string());
    }
    open::that(&url).map_err(|e| e.to_string())
}
