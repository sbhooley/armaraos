//! ArmaraOS logo path for OS notifications (`notify_rust` expects a PNG path).

use std::path::PathBuf;
use std::sync::OnceLock;

static ICON_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

/// PNG path for `notify-rust` / OS notifications.
pub fn notify_icon_path() -> Option<String> {
    cached_icon_path()
}

fn cached_icon_path() -> Option<String> {
    ICON_PATH
        .get_or_init(|| {
            let p = std::env::temp_dir().join("armaraos-notification-icon.png");
            // Same asset as `public/assets/armaraos-logo.png` (repo root); avoid duplicating bytes under `icons/`.
            match std::fs::write(
                &p,
                include_bytes!("../../../public/assets/armaraos-logo.png"),
            ) {
                Ok(()) => Some(p),
                Err(_) => None,
            }
        })
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
}
