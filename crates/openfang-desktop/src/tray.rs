//! System tray setup for the ArmaraOS desktop app.

use std::sync::Arc;
use std::time::Duration;

use openfang_kernel::config::openfang_home;
use openfang_kernel::openclaw_workspace;
use openfang_kernel::OpenFangKernel;
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager,
};
use tauri_plugin_autostart::ManagerExt;
use tracing::{info, warn};

use crate::os_notify;

/// Format seconds into a human-readable uptime string.
fn format_uptime(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        let m = secs / 60;
        let s = secs % 60;
        format!("{m}m {s}s")
    } else {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        format!("{h}h {m}m")
    }
}

fn tooltip_for_kernel(kernel: &OpenFangKernel) -> String {
    let cfg = &kernel.config.openclaw_workspace;
    if !cfg.enabled || !cfg.show_pending_in_tray {
        return "ArmaraOS Agent OS".to_string();
    }
    let root = openclaw_workspace::resolve_openclaw_workspace_root(&kernel.config);
    let total = openclaw_workspace::read_pipeline_pending_total(&root).unwrap_or(0);
    if total > 0 {
        format!("ArmaraOS — {total} learnings pending (skills workspace)")
    } else {
        "ArmaraOS Agent OS".to_string()
    }
}

/// Build and register the system tray icon with enhanced menu.
pub fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    // Action items
    let show = MenuItem::with_id(app, "show", "Show Window", true, None::<&str>)?;
    let browser = MenuItem::with_id(app, "browser", "Open in Browser", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;

    // Informational items (disabled — display only)
    let agent_count = if let Some(ks) = app.try_state::<crate::KernelState>() {
        ks.kernel.registry.list().len()
    } else {
        0
    };
    let uptime = if let Some(ks) = app.try_state::<crate::KernelState>() {
        format_uptime(ks.started_at.elapsed().as_secs())
    } else {
        "0s".to_string()
    };
    let agents_info = MenuItem::with_id(
        app,
        "agents_info",
        format!("Agents: {agent_count} running"),
        false,
        None::<&str>,
    )?;
    let status_info = MenuItem::with_id(
        app,
        "status_info",
        format!("Status: Running ({uptime})"),
        false,
        None::<&str>,
    )?;

    let sep_oc_pre = PredefinedMenuItem::separator(app)?;

    let (oc_total, oc_enabled) = app
        .try_state::<crate::KernelState>()
        .map(|ks| {
            let en = ks.kernel.config.openclaw_workspace.enabled;
            if !en {
                return (0u32, false);
            }
            let root = openclaw_workspace::resolve_openclaw_workspace_root(&ks.kernel.config);
            let t = openclaw_workspace::read_pipeline_pending_total(&root).unwrap_or(0);
            (t, true)
        })
        .unwrap_or((0, false));

    let oc_info = MenuItem::with_id(
        app,
        "oc_info",
        if oc_enabled {
            format!("Skills workspace: {oc_total} pending")
        } else {
            "Skills workspace: off (see config)".to_string()
        },
        false,
        None::<&str>,
    )?;
    let open_oc_learnings = MenuItem::with_id(
        app,
        "open_oc_learnings",
        "Open .learnings folder (skills workspace)",
        oc_enabled,
        None::<&str>,
    )?;
    let run_oc_digest = MenuItem::with_id(
        app,
        "run_oc_digest",
        "Run learnings digest now",
        oc_enabled,
        None::<&str>,
    )?;

    let sep2 = PredefinedMenuItem::separator(app)?;

    // Settings items
    let autostart_enabled = app.autolaunch().is_enabled().unwrap_or(false);
    let launch_at_login = CheckMenuItem::with_id(
        app,
        "launch_at_login",
        "Launch at Login",
        true,
        autostart_enabled,
        None::<&str>,
    )?;
    let check_updates = MenuItem::with_id(
        app,
        "check_updates",
        "Check for Updates...",
        true,
        None::<&str>,
    )?;
    let open_config = MenuItem::with_id(
        app,
        "open_config",
        "Open Config Directory",
        true,
        None::<&str>,
    )?;
    let sep3 = PredefinedMenuItem::separator(app)?;

    let quit = MenuItem::with_id(app, "quit", "Quit ArmaraOS", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &show,
            &browser,
            &sep1,
            &agents_info,
            &status_info,
            &sep_oc_pre,
            &oc_info,
            &open_oc_learnings,
            &run_oc_digest,
            &sep2,
            &launch_at_login,
            &check_updates,
            &open_config,
            &sep3,
            &quit,
        ],
    )?;

    // Load the tray icon from embedded PNG bytes
    let tray_image = tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png"))
        .expect("Failed to decode tray icon PNG");

    let initial_tip = app
        .try_state::<crate::KernelState>()
        .map(|ks| tooltip_for_kernel(&ks.kernel))
        .unwrap_or_else(|| "ArmaraOS Agent OS".to_string());

    let tray = TrayIconBuilder::new()
        .icon(tray_image)
        .menu(&menu)
        .tooltip(&initial_tip)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "show" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.unminimize();
                    let _ = w.set_focus();
                }
            }
            "browser" => {
                if let Some(port) = app.try_state::<crate::PortState>() {
                    let url = format!("http://127.0.0.1:{}", port.0);
                    let _ = open::that(&url);
                }
            }
            "open_oc_learnings" => {
                if let Some(ks) = app.try_state::<crate::KernelState>() {
                    let root =
                        openclaw_workspace::resolve_openclaw_workspace_root(&ks.kernel.config);
                    let p = root.join(".learnings");
                    if let Err(e) = std::fs::create_dir_all(&p) {
                        warn!("create .learnings: {e}");
                    }
                    if let Err(e) = open::that(&p) {
                        warn!("open .learnings: {e}");
                    }
                }
            }
            "run_oc_digest" => {
                if let Some(ks) = app.try_state::<crate::KernelState>() {
                    if !ks.kernel.config.openclaw_workspace.enabled {
                        return;
                    }
                    let root =
                        openclaw_workspace::resolve_openclaw_workspace_root(&ks.kernel.config);
                    let app_h = app.clone();
                    std::thread::spawn(move || {
                        match openclaw_workspace::export_learnings_digest(&root) {
                            Ok(s) => {
                                os_notify::post_from_app(
                                    &app_h,
                                    "Skills workspace digest updated",
                                    s.daily_path.display().to_string(),
                                );
                            }
                            Err(e) => {
                                warn!("Skills workspace digest: {e}");
                                os_notify::post_from_app(
                                    &app_h,
                                    "Skills workspace digest failed",
                                    e,
                                );
                            }
                        }
                    });
                }
            }
            "launch_at_login" => {
                let manager = app.autolaunch();
                let currently_enabled = manager.is_enabled().unwrap_or(false);
                if currently_enabled {
                    if let Err(e) = manager.disable() {
                        warn!("Failed to disable autostart: {e}");
                    }
                } else if let Err(e) = manager.enable() {
                    warn!("Failed to enable autostart: {e}");
                }
                info!(
                    "Autostart toggled: {}",
                    manager.is_enabled().unwrap_or(false)
                );
            }
            "check_updates" => {
                let app_handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    // First check what's available
                    match crate::updater::check_for_update(&app_handle).await {
                        Ok(info) if info.available => {
                            let version = info.version.as_deref().unwrap_or("unknown");
                            os_notify::post_from_app(
                                &app_handle,
                                "Installing Update...",
                                format!(
                                    "Downloading ArmaraOS v{version}. App will restart shortly."
                                ),
                            );
                            // Perform install
                            if let Err(e) =
                                crate::updater::download_and_install_update(&app_handle).await
                            {
                                warn!("Manual update install failed: {e}");
                                os_notify::post_from_app(
                                    &app_handle,
                                    "Update Failed",
                                    format!("Could not install update: {e}"),
                                );
                            }
                            // If we reach here, install failed (success causes restart)
                        }
                        Ok(_) => {
                            os_notify::post_from_app(
                                &app_handle,
                                "Up to Date",
                                "You're running the latest version of ArmaraOS.",
                            );
                        }
                        Err(e) => {
                            warn!("Tray update check failed: {e}");
                            os_notify::post_from_app(
                                &app_handle,
                                "Update Check Failed",
                                "Could not check for updates. Try again later.",
                            );
                        }
                    }
                });
            }
            "open_config" => {
                let dir = openfang_home();
                let _ = std::fs::create_dir_all(&dir);
                if let Err(e) = open::that(&dir) {
                    warn!("Failed to open config dir: {e}");
                }
            }
            "quit" => {
                info!("Quit requested from system tray");
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.unminimize();
                    let _ = w.set_focus();
                }
            }
        })
        .build(app)?;

    if let Some(ks) = app.try_state::<crate::KernelState>() {
        let tray_bg = tray.clone();
        let kernel_bg: Arc<OpenFangKernel> = Arc::clone(&ks.kernel);
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(90));
            let tip = tooltip_for_kernel(kernel_bg.as_ref());
            let _ = tray_bg.set_tooltip(Some(tip.as_str()));
        });
    }

    Ok(())
}
