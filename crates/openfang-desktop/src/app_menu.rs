//! Application menu bar — Help links to AI Native Lang.

use tauri::menu::{Menu, MenuItem, HELP_SUBMENU_ID};
use tauri::{AppHandle, Runtime};

/// Builds the default Tauri menu and adds Help items for the website and X profile.
pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let menu = Menu::default(app)?;
    if let Some(kind) = menu.get(HELP_SUBMENU_ID) {
        if let Some(help) = kind.as_submenu() {
            let notif = MenuItem::with_id(
                app,
                "help_notification_settings",
                "Notification Settings…",
                true,
                None::<&str>,
            )?;
            let website = MenuItem::with_id(
                app,
                "help_ainl_website",
                "AI Native Lang Website…",
                true,
                None::<&str>,
            )?;
            let x_profile =
                MenuItem::with_id(app, "help_ainl_x", "X — @ainativelang", true, None::<&str>)?;
            let diagnostics = MenuItem::with_id(
                app,
                "help_generate_diagnostics",
                "Generate Diagnostics Bundle…",
                true,
                None::<&str>,
            )?;
            help.append(&notif)?;
            help.append(&website)?;
            help.append(&x_profile)?;
            help.append(&diagnostics)?;
        }
    }
    Ok(menu)
}
