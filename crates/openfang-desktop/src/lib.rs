//! ArmaraOS Desktop — Native Tauri 2.0 wrapper for the ArmaraOS Agent OS.
//!
//! Boots the kernel + embedded API server, then opens a native window pointing
//! at the WebUI. Includes system tray, single-instance enforcement, native OS
//! notifications, global shortcuts, auto-start, and update checking.

mod ainl;
mod ainl_upstream;
mod ainl_version;
#[cfg(desktop)]
mod app_menu;
mod commands;
mod notification_icon;
mod server;
mod shortcuts;
mod tray;
mod ui_prefs;
mod updater;

use openfang_kernel::OpenFangKernel;
use openfang_types::event::{EventPayload, LifecycleEvent, SystemEvent};
use std::sync::Arc;
use std::time::Instant;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_notification::NotificationExt;
use tracing::{info, warn};

use crate::notification_icon::apply_notification_icon;

/// Managed state: the port the embedded server listens on.
pub struct PortState(pub u16);

/// Managed state: the kernel instance and startup time.
pub struct KernelState {
    pub kernel: Arc<OpenFangKernel>,
    pub started_at: Instant,
}

/// Entry point for the Tauri application.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Init tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "openfang=info,tauri=info".into()),
        )
        .init();

    info!("Starting ArmaraOS Desktop...");

    // Boot kernel + embedded server (blocks until port is known)
    let server_handle = server::start_server().expect("Failed to start ArmaraOS server");
    let port = server_handle.port;
    let kernel_for_notifications = server_handle.kernel.clone();

    info!("ArmaraOS server running on port {port}");

    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init());

    #[cfg(desktop)]
    {
        builder = builder
            .menu(crate::app_menu::build)
            .on_menu_event(|app, event| {
                if event.id() == "help_ainl_website" {
                    let _ = open::that("https://ainativelang.com/");
                } else if event.id() == "help_ainl_x" {
                    let _ = open::that("https://x.com/ainativelang");
                } else if event.id() == "help_generate_diagnostics" {
                    let handle = app.app_handle().clone();
                    let port = app.state::<PortState>().0;
                    tauri::async_runtime::spawn(async move {
                        match crate::commands::post_support_bundle(port).await {
                            Ok(v) => {
                                let path = v
                                    .get("bundle_path")
                                    .and_then(|x| x.as_str())
                                    .unwrap_or("(see ~/.armaraos/support/)");
                                let _ = crate::notification_icon::apply_notification_icon(
                                    handle
                                        .notification()
                                        .builder()
                                        .title("Diagnostics bundle ready")
                                        .body(path.to_string()),
                                )
                                .show();
                            }
                            Err(e) => {
                                warn!("Diagnostics bundle failed: {e}");
                                let _ = crate::notification_icon::apply_notification_icon(
                                    handle
                                        .notification()
                                        .builder()
                                        .title("Diagnostics bundle failed")
                                        .body(e.clone()),
                                )
                                .show();
                            }
                        }
                    });
                }
            });
    }

    // Desktop-only plugins
    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // Another instance tried to launch — focus the existing window
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }));

        builder = builder.plugin(
            tauri_plugin_autostart::Builder::new()
                .args(["--minimized"])
                .build(),
        );

        builder = builder.plugin(tauri_plugin_updater::Builder::new().build());

        // Global shortcuts — non-fatal on registration failure
        match shortcuts::build_shortcut_plugin() {
            Ok(plugin) => {
                builder = builder.plugin(plugin);
            }
            Err(e) => {
                warn!("Failed to register global shortcuts: {e}");
            }
        }
    }

    builder
        .manage(PortState(port))
        .manage(KernelState {
            kernel: server_handle.kernel.clone(),
            started_at: Instant::now(),
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_port,
            commands::get_status,
            commands::get_agent_count,
            commands::import_agent_toml,
            commands::import_skill_file,
            commands::get_autostart,
            commands::set_autostart,
            commands::check_for_updates,
            commands::install_update,
            commands::generate_support_bundle,
            commands::get_desktop_updater_prefs,
            commands::set_release_channel,
            commands::report_daemon_update_check,
            commands::open_config_dir,
            commands::open_logs_dir,
            commands::ainl_status,
            commands::ensure_ainl_installed,
            commands::ensure_armaraos_ainl_host,
            commands::ainl_check_versions,
            commands::upgrade_ainl_pip,
            commands::set_dashboard_theme_mode,
            commands::open_external_url,
            commands::open_ainl_library_dir,
            commands::ainl_try_library_file,
        ])
        .setup(move |app| {
            // Create the main window pointing directly at the embedded HTTP server.
            // We do NOT define windows in tauri.conf.json because Tauri would try to
            // load index.html from embedded assets (which don't exist), causing a race
            // condition where AssetNotFound overwrites the navigated page.
            let theme_mode = crate::ui_prefs::load_theme_mode(app.handle());
            let url = format!("http://127.0.0.1:{port}/?armaraos_theme={theme_mode}");
            let _window = WebviewWindowBuilder::new(
                app,
                "main",
                WebviewUrl::External(url.parse().expect("Invalid server URL")),
            )
            .title("ArmaraOS")
            .inner_size(1280.0, 800.0)
            .min_inner_size(800.0, 600.0)
            .center()
            .visible(true)
            .build()?;

            // Set up system tray (desktop only)
            #[cfg(desktop)]
            tray::setup_tray(app)?;

            // Auto-bootstrap AINL on startup: venv + pip/wheel + MCP host + library sync.
            // Runs on a dedicated OS thread (blocking I/O + subprocess) so the async runtime is not wedged.
            // Set ARMARAOS_AINL_AUTO_BOOTSTRAP=0 to skip (e.g. CI or air-gapped debugging).
            let skip_ainl = std::env::var("ARMARAOS_AINL_AUTO_BOOTSTRAP")
                .map(|v| v == "0" || v.eq_ignore_ascii_case("false"))
                .unwrap_or(false);
            if !skip_ainl {
                let app_handle_for_ainl = app.handle().clone();
                std::thread::spawn(move || {
                    match crate::ainl::ensure_ainl_installed(&app_handle_for_ainl) {
                        Ok(st) if st.ok => {
                            info!("AINL bootstrap OK: {}", st.detail);
                            if st.armaraos_host_ok == Some(false) {
                                warn!(
                                    "ArmaraOS AINL host integration failed: {:?}",
                                    st.armaraos_host_detail
                                );
                            }
                        }
                        Ok(st) => {
                            warn!("AINL bootstrap incomplete: {}", st.detail);
                            let _ = apply_notification_icon(
                                app_handle_for_ainl
                                    .notification()
                                    .builder()
                                    .title("AINL setup incomplete")
                                    .body(st.detail),
                            )
                            .show();
                        }
                        Err(e) => {
                            warn!("AINL bootstrap failed: {e}");
                            let _ = apply_notification_icon(
                                app_handle_for_ainl
                                    .notification()
                                    .builder()
                                    .title("AINL setup failed")
                                    .body(e),
                            )
                            .show();
                        }
                    }
                });
            } else {
                info!("Skipping AINL auto-bootstrap (ARMARAOS_AINL_AUTO_BOOTSTRAP=0)");
            }

            // Spawn background task to forward critical kernel events as native
            // OS notifications. Only truly critical events — crashes, hard quota
            // limits, and kernel shutdown. Health checks and quota warnings are
            // too noisy for desktop notifications.
            let app_handle = app.handle().clone();
            let mut event_rx = kernel_for_notifications.event_bus.subscribe_all();
            tauri::async_runtime::spawn(async move {
                loop {
                    match event_rx.recv().await {
                        Ok(event) => {
                            let (title, body) = match &event.payload {
                                EventPayload::Lifecycle(LifecycleEvent::Crashed {
                                    agent_id,
                                    error,
                                }) => (
                                    "Agent Crashed".to_string(),
                                    format!("Agent {agent_id} crashed: {error}"),
                                ),
                                EventPayload::System(SystemEvent::KernelStopping) => (
                                    "Kernel Stopping".to_string(),
                                    "ArmaraOS kernel is shutting down".to_string(),
                                ),
                                EventPayload::System(SystemEvent::QuotaEnforced {
                                    agent_id,
                                    spent,
                                    limit,
                                }) => (
                                    "Quota Enforced".to_string(),
                                    format!(
                                        "Agent {agent_id} quota hit: ${spent:.4} / ${limit:.4}"
                                    ),
                                ),
                                EventPayload::System(SystemEvent::CronJobCompleted {
                                    job_name,
                                    output_preview,
                                    ..
                                }) => (
                                    "Scheduled job finished".to_string(),
                                    format!("{job_name}: {output_preview}"),
                                ),
                                EventPayload::System(SystemEvent::CronJobFailed {
                                    job_name,
                                    error,
                                    ..
                                }) => ("Scheduled job failed".to_string(), format!("{job_name}: {error}")),
                                // Skip everything else (health checks, spawns, suspends, etc.)
                                _ => continue,
                            };

                            if let Err(e) = apply_notification_icon(
                                app_handle
                                    .notification()
                                    .builder()
                                    .title(&title)
                                    .body(&body),
                            )
                            .show()
                            {
                                warn!("Failed to send desktop notification: {e}");
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            warn!("Notification listener lagged, skipped {n} events");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            info!("Event bus closed, stopping notification listener");
                            break;
                        }
                    }
                }
            });

            // Spawn startup update check (desktop only, after event forwarding is set up)
            #[cfg(desktop)]
            updater::spawn_startup_check(app.handle().clone());

            // PyPI vs venv check: notify when ainativelang has a newer release (desktop only)
            #[cfg(desktop)]
            crate::ainl_version::spawn_ainl_pypi_notify_check(app.handle().clone());

            info!("ArmaraOS Desktop window created");
            Ok(())
        })
        .on_window_event(|window, event| {
            // Hide to tray on close instead of quitting (desktop)
            #[cfg(desktop)]
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .build(tauri::generate_context!())
        .expect("Failed to build Tauri application")
        .run(|_app, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                info!("Tauri app exit requested");
            }
        });

    // App event loop has ended — shut down the embedded server + kernel
    info!("Tauri app closed, shutting down embedded server...");
    server_handle.shutdown();
}
