//! ArmaraOS logo path for OS notifications (`notify_rust` expects a PNG path).

use std::path::PathBuf;
use std::sync::OnceLock;

use tauri::Runtime;
use tauri_plugin_notification::NotificationBuilder;

static ICON_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

fn cached_icon_path() -> Option<String> {
    ICON_PATH
        .get_or_init(|| {
            let p = std::env::temp_dir().join("armaraos-notification-icon.png");
            match std::fs::write(&p, include_bytes!("../icons/armaraos-logo.png")) {
                Ok(()) => Some(p),
                Err(_) => None,
            }
        })
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
}

/// Attach the bundled ArmaraOS logo so notifications match app branding.
pub fn apply_notification_icon<R: Runtime>(
    builder: NotificationBuilder<R>,
) -> NotificationBuilder<R> {
    if let Some(p) = cached_icon_path() {
        builder.icon(p)
    } else {
        builder
    }
}
