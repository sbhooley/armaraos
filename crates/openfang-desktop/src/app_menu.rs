//! Application menu bar — Help submenu (support email, links, diagnostics).

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, HELP_SUBMENU_ID};
use tauri::{AppHandle, Runtime};

/// Extends the default Help menu with ArmaraOS / AI Native Lang links (same targets as the web sidebar footer).
pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let menu = Menu::default(app)?;
    if let Some(kind) = menu.get(HELP_SUBMENU_ID) {
        if let Some(help) = kind.as_submenu() {
            let support_email = MenuItem::with_id(
                app,
                "help_support_email",
                "Get Help (Email)…",
                true,
                None::<&str>,
            )?;
            let notif = MenuItem::with_id(
                app,
                "help_notification_settings",
                "Notification Settings…",
                true,
                None::<&str>,
            )?;
            let sep1 = PredefinedMenuItem::separator(app)?;
            let website = MenuItem::with_id(
                app,
                "help_ainl_website",
                "Website — ainativelang.com",
                true,
                None::<&str>,
            )?;
            let x_profile =
                MenuItem::with_id(app, "help_ainl_x", "X — @ainativelang", true, None::<&str>)?;
            let telegram = MenuItem::with_id(
                app,
                "help_telegram",
                "Telegram — AINL Portal",
                true,
                None::<&str>,
            )?;
            let github = MenuItem::with_id(
                app,
                "help_ainl_github",
                "AINL Compiler on GitHub…",
                true,
                None::<&str>,
            )?;
            let sep2 = PredefinedMenuItem::separator(app)?;
            let diagnostics = MenuItem::with_id(
                app,
                "help_generate_diagnostics",
                "Generate Diagnostics Bundle…",
                true,
                None::<&str>,
            )?;
            help.append(&support_email)?;
            help.append(&notif)?;
            help.append(&sep1)?;
            help.append(&website)?;
            help.append(&x_profile)?;
            help.append(&telegram)?;
            help.append(&github)?;
            help.append(&sep2)?;
            help.append(&diagnostics)?;
        }
    }
    Ok(menu)
}
