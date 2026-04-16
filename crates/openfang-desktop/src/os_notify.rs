//! Post OS notifications via `notify-rust` with the correct macOS bundle id.
//!
//! `tauri-plugin-notification` sets `notify_rust::set_application` to `com.apple.Terminal` when
//! `tauri::is_dev()` is true, so dev builds attribute notifications to Terminal and they are easy
//! to miss. We always use the app identifier from [`tauri::Config`] (e.g. `ai.armaraos.desktop`).
//!
//! **macOS:** `notify-rust` does not apply [`Notification::icon`](notify_rust::Notification::icon);
//! Notification Center shows the **app bundle icon** (`icon.icns` from the Tauri build). Regenerate
//! bundle + web icons from `public/assets/armaraos-logo.png` via `crates/openfang-desktop/scripts/regen_icons_from_logo.py`.

use tauri::{AppHandle, Runtime};

/// Same as [`post`], using the app bundle id and notification icon from `app`.
pub fn post_from_app<R: Runtime>(
    app: &AppHandle<R>,
    title: impl Into<String>,
    body: impl Into<String>,
) {
    post(
        app.config().identifier.clone(),
        title.into(),
        body.into(),
        crate::notification_icon::notify_icon_path(),
    );
}

/// Show a native notification (async on the Tauri runtime). Logs failures at `warn`.
pub fn post(bundle_identifier: String, title: String, body: String, icon_path: Option<String>) {
    tauri::async_runtime::spawn(async move {
        #[cfg(target_os = "macos")]
        let bundle_id = bundle_identifier;
        #[cfg(not(target_os = "macos"))]
        drop(bundle_identifier);

        let mut n = notify_rust::Notification::new();
        n.summary(&title);
        n.body(&body);
        #[cfg(target_os = "macos")]
        drop(icon_path);
        #[cfg(not(target_os = "macos"))]
        {
            if let Some(ref p) = icon_path {
                n.icon(p);
            } else {
                n.auto_icon();
            }
        }
        #[cfg(target_os = "macos")]
        {
            let _ = notify_rust::set_application(&bundle_id);
        }
        match n.show() {
            Ok(_) => tracing::debug!(%title, "Desktop notification posted"),
            Err(e) => tracing::warn!("Desktop notification failed: {e}"),
        }
    });
}
