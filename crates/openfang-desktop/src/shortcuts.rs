//! System-wide keyboard shortcuts for the ArmaraOS desktop app.

use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, Modifiers, ShortcutState};
use tracing::warn;

/// Build the global shortcut plugin with 3 system-wide shortcuts:
///
/// - `Ctrl+Alt+Space` — Show/focus the ArmaraOS window
/// - `Ctrl+Alt+A` — Show window + navigate to agents page
/// - `Ctrl+Alt+H` — Show window + navigate to chat page
///
/// Changed from Ctrl+Shift+{O,N,C} to avoid conflicts with browser shortcuts
/// (Ctrl+Shift+N = incognito, Ctrl+Shift+C = inspect element).
///
/// Returns `Result` so `lib.rs` can handle registration failure gracefully.
pub fn build_shortcut_plugin<R: tauri::Runtime>(
) -> Result<tauri::plugin::TauriPlugin<R>, tauri_plugin_global_shortcut::Error> {
    let plugin = tauri_plugin_global_shortcut::Builder::new()
        .with_shortcuts(["ctrl+alt+space", "ctrl+alt+a", "ctrl+alt+h"])?
        .with_handler(|app, shortcut, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }

            // All shortcuts show/focus the window first
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }

            if shortcut.matches(Modifiers::ALT | Modifiers::CONTROL, Code::KeyA) {
                if let Err(e) = app.emit("navigate", "agents") {
                    warn!("Failed to emit navigate event: {e}");
                }
            } else if shortcut.matches(Modifiers::ALT | Modifiers::CONTROL, Code::KeyH) {
                if let Err(e) = app.emit("navigate", "chat") {
                    warn!("Failed to emit navigate event: {e}");
                }
            }
            // Ctrl+Alt+Space just shows the window (already done above)
        })
        .build();

    Ok(plugin)
}
