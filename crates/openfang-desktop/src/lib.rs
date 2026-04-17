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
#[cfg(target_os = "macos")]
mod macos_app_icon;
mod notification_icon;
mod os_notify;
mod product_analytics;
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
use tracing::{info, warn};

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
                if event.id() == "help_support_email" {
                    let _ = open::that("mailto:ainativelang@gmail.com?subject=ArmaraOS%20Support");
                } else if event.id() == "help_ainl_website" {
                    let _ = open::that("https://ainativelang.com/");
                } else if event.id() == "help_ainl_x" {
                    let _ = open::that("https://x.com/ainativelang");
                } else if event.id() == "help_telegram" {
                    let _ = open::that("https://t.me/AINL_Portal");
                } else if event.id() == "help_ainl_github" {
                    let _ = open::that("https://github.com/sbhooley/ainativelang");
                } else if event.id() == "help_notification_settings" {
                    let _ = crate::commands::open_notification_settings(app.app_handle().clone());
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
                                crate::os_notify::post_from_app(
                                    &handle,
                                    "Diagnostics bundle ready",
                                    path.to_string(),
                                );
                            }
                            Err(e) => {
                                warn!("Diagnostics bundle failed: {e}");
                                crate::os_notify::post_from_app(
                                    &handle,
                                    "Diagnostics bundle failed",
                                    e.clone(),
                                );
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
            commands::copy_diagnostics_to_downloads,
            commands::copy_home_file_to_downloads,
            commands::compose_support_email,
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
            commands::get_dashboard_bookmarks,
            commands::set_dashboard_bookmarks,
            commands::open_external_url,
            commands::open_ainl_library_dir,
            commands::ainl_try_library_file,
            commands::open_notification_settings,
            commands::get_desktop_product_analytics_prefs,
            commands::set_desktop_product_analytics_prefs,
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
            .theme(crate::ui_prefs::window_theme_for_mode(&theme_mode))
            .build()?;

            // macOS 26+: avoid Tahoe’s auto-layered dock icon (silver plate + inset artwork).
            #[cfg(target_os = "macos")]
            crate::macos_app_icon::apply_flat_icon_image();

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
                            crate::os_notify::post_from_app(
                                &app_handle_for_ainl,
                                "AINL setup incomplete",
                                st.detail,
                            );
                        }
                        Err(e) => {
                            warn!("AINL bootstrap failed: {e}");
                            crate::os_notify::post_from_app(
                                &app_handle_for_ainl,
                                "AINL setup failed",
                                e,
                            );
                        }
                    }
                });
            } else {
                info!("Skipping AINL auto-bootstrap (ARMARAOS_AINL_AUTO_BOOTSTRAP=0)");
            }

            // Spawn background task to forward kernel events as native OS notifications.
            // Uses `os_notify` so macOS attributes to this app (not Terminal) in dev builds.
            let app_handle = app.handle().clone();
            let bundle_id = app_handle.config().identifier.clone();
            let mut event_rx = kernel_for_notifications.event_bus.subscribe_all();
            tauri::async_runtime::spawn(async move {
                loop {
                    match event_rx.recv().await {
                        Ok(event) => {
                            let (title, body) = match &event.payload {
                                EventPayload::Lifecycle(LifecycleEvent::Spawned {
                                    agent_id,
                                    name,
                                }) => (
                                    "Agent started".to_string(),
                                    format!("{name} is running ({agent_id})"),
                                ),
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
                                EventPayload::System(SystemEvent::WorkflowRunFinished {
                                    workflow_name,
                                    ok,
                                    summary,
                                    ..
                                }) => {
                                    if *ok {
                                        (
                                            "Workflow finished".to_string(),
                                            format!("{workflow_name}: {}", summary.as_str()),
                                        )
                                    } else {
                                        (
                                            "Workflow failed".to_string(),
                                            format!("{workflow_name}: {}", summary.as_str()),
                                        )
                                    }
                                },
                                // Health check failures are too noisy for OS toasts; use logs / WebUI.
                                EventPayload::System(SystemEvent::HealthCheckFailed { .. }) => {
                                    continue;
                                }
                                // Assistant replies are surfaced in the dashboard bell; OS toasts would be noisy.
                                EventPayload::System(SystemEvent::AgentAssistantReply { .. }) => {
                                    continue;
                                }
                                EventPayload::System(SystemEvent::ApprovalPending {
                                    request_id,
                                    agent_id,
                                    tool_name,
                                    action_summary,
                                }) => (
                                    "Approval needed".to_string(),
                                    format!(
                                        "{tool_name} ({agent_id}) — {action_summary} [id: {request_id}]"
                                    ),
                                ),
                                // Skip everything else (health checks, quota warnings, etc.)
                                _ => continue,
                            };

                            crate::os_notify::post(
                                bundle_id.clone(),
                                title,
                                body,
                                crate::notification_icon::notify_icon_path(),
                            );
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
            #[cfg(desktop)]
            updater::spawn_periodic_update_check(app.handle().clone());

            // PyPI vs venv check: notify when ainativelang has a newer release (desktop only)
            #[cfg(desktop)]
            crate::ainl_version::spawn_ainl_pypi_notify_check(app.handle().clone());

            info!("ArmaraOS Desktop window created");

            #[cfg(desktop)]
            product_analytics::spawn_first_open_worker(app.handle());

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
