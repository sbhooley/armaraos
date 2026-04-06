# ArmaraOS Desktop App

The ArmaraOS Desktop App is a native desktop wrapper built with [Tauri 2.0](https://v2.tauri.app/) that packages the ArmaraOS Agent OS into a single, installable application. Instead of running a CLI daemon and opening a browser, users get a native window with system tray integration, OS notifications, and single-instance enforcement -- all powered by the same kernel and API server that the headless deployment uses.

**Crate:** `openfang-desktop`
**Identifier:** `ai.armaraos.desktop`
**Product name:** ArmaraOS

---

## Architecture

The desktop app follows a straightforward embedded-server pattern:

```
+-------------------------------------------+
|  Tauri 2.0 Process                        |
|                                           |
|  +-----------+    +--------------------+  |
|  |  Main     |    | Background Thread  |  |
|  |  Thread   |    | ("openfang-server")|  |
|  |           |    |                    |  |
|  | WebView   |    | tokio runtime      |  |
|  | Window    |--->| axum API server    |  |
|  | (main)    |    | channel bridges    |  |
|  |           |    | background agents  |  |
|  | System    |    |                    |  |
|  | Tray      |    | OpenFang Kernel    |  |
|  +-----------+    +--------------------+  |
|       |                    |              |
|       |   http://127.0.0.1:{port}        |
|       +------------------------------------
+-------------------------------------------+
```

### Startup Sequence

1. **Tracing init** -- `tracing_subscriber` is configured with `RUST_LOG` env, defaulting to `openfang=info,tauri=info`.
2. **Dotenv + AINL defaults** -- `load_dotenv_files()` merges **`~/.armaraos/.env`** and **`secrets.env`** into the process (existing OS env wins). If **`AINL_ALLOW_IR_DECLARED_ADAPTERS`** is still unset, it is set to **`1`** so AINL graphs can use IR-declared adapters without manual shell configuration. Scheduled **`ainl run`** children also receive this flag from the kernel; see **[scheduled-ainl.md](scheduled-ainl.md)**.
3. **Kernel boot** -- `OpenFangKernel::boot(None)` loads the default configuration (from `config.toml` or defaults), wrapped in `Arc`. `set_self_handle()` is called to enable self-referencing kernel operations.
4. **Port binding** -- A `std::net::TcpListener` binds to `127.0.0.1:0` on the main thread, which lets the OS assign a random free port. This ensures the port number is known before any window is created.
5. **Server thread** -- A dedicated OS thread named `"openfang-server"` is spawned. It creates its own `tokio::runtime::Builder::new_multi_thread()` runtime and runs:
   - `kernel.start_background_agents()` -- heartbeat monitor, autonomous agents, etc.
   - `run_embedded_server()` -- builds the axum router via `openfang_api::server::build_router()`, converts the `std::net::TcpListener` to a `tokio::net::TcpListener`, and serves with graceful shutdown.
6. **Tauri app** -- The Tauri builder is assembled with plugins, managed state, IPC commands, system tray, and a WebView window pointing at `http://127.0.0.1:{port}`.
7. **Event loop** -- Tauri runs its native event loop. On exit, `server_handle.shutdown()` is called to stop the embedded server and kernel.

### ServerHandle

The `ServerHandle` struct (defined in `src/server.rs`) manages the embedded server lifecycle:

```rust
pub struct ServerHandle {
    pub port: u16,
    pub kernel: Arc<OpenFangKernel>,
    shutdown_tx: watch::Sender<bool>,
    server_thread: Option<std::thread::JoinHandle<()>>,
}
```

- **`port`** -- The port the embedded server is listening on.
- **`kernel`** -- Shared reference to the kernel, also used by the Tauri app for IPC commands and notifications.
- **`shutdown_tx`** -- A `tokio::sync::watch` channel. Sending `true` triggers graceful shutdown of the axum server.
- **`server_thread`** -- Join handle for the background thread. `shutdown()` joins it to ensure clean termination.

Calling `shutdown()` sends the shutdown signal, joins the background thread, and calls `kernel.shutdown()`. The `Drop` implementation sends the shutdown signal as a best-effort fallback but does not block on the thread join.

### Graceful Shutdown

The axum server uses `with_graceful_shutdown()` wired to the watch channel:

```rust
let server = axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
    .with_graceful_shutdown(async move {
        let _ = shutdown_rx.wait_for(|v| *v).await;
    });
```

After the server shuts down, channel bridges (Telegram, Slack, etc.) are stopped via `bridge.stop().await`.

---

## Features

### System Tray

The system tray (defined in `src/tray.rs`) provides quick access without bringing up the main window:

| Menu Item | Behavior |
|-----------|----------|
| **Show Window** | Calls `show()`, `unminimize()`, and `set_focus()` on the main WebView window |
| **Open in Browser** | Reads the port from managed `PortState` and opens `http://127.0.0.1:{port}` in the default browser |
| **Agents: N running** | Disabled (info only) — shows current agent count |
| **Status: Running (uptime)** | Disabled (info only) — shows uptime in human-readable format |
| **Launch at Login** | Checkbox — toggles OS-level auto-start via `tauri-plugin-autostart` |
| **Check for Updates...** | Checks for updates, downloads, installs, and restarts if available. Shows notifications for progress/success/failure |
| **Open Config Directory** | Opens `~/.armaraos/` in the OS file manager |
| **Quit OpenFang** | Logs the quit event and calls `app.exit(0)` |

The tray tooltip reads **"OpenFang Agent OS"**.

**Left-click on tray icon** shows the main window (same as "Show Window" menu item). This is implemented via `on_tray_icon_event` listening for `MouseButton::Left` with `MouseButtonState::Up`.

### Single-Instance Enforcement

On desktop platforms, `tauri-plugin-single-instance` prevents multiple copies of OpenFang from running simultaneously. When a second instance attempts to launch, the existing instance's main window is shown, unminimized, and focused:

```rust
#[cfg(desktop)]
{
    builder = builder.plugin(tauri_plugin_single_instance::init(
        |app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        },
    ));
}
```

### Hide-to-Tray on Close

Closing the window does not quit the application. Instead, the window is hidden and the close event is suppressed:

```rust
.on_window_event(|window, event| {
    #[cfg(desktop)]
    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
        let _ = window.hide();
        api.prevent_close();
    }
})
```

To actually quit, use the **"Quit OpenFang"** option in the system tray menu.

### Native OS Notifications

The app subscribes to the kernel's event bus and forwards selected events as native desktop notifications. Posts go through **`notify-rust`** with the app bundle id from Tauri config (see **`crates/openfang-desktop/src/os_notify.rs`**) so notifications attribute to **ArmaraOS** in release builds instead of Terminal in dev. On **macOS**, Notification Center uses the **app bundle icon** (`icon.icns`); regenerate tray/app icons from `public/assets/armaraos-logo.png` with **`crates/openfang-desktop/scripts/regen_icons_from_logo.py`** if you change the logo.

| Event | Notification Title | Body |
|-------|-------------------|------|
| `LifecycleEvent::Crashed` | "Agent Crashed" | `Agent {id} crashed: {error}` |
| `LifecycleEvent::Spawned` | "Agent Started" | `Agent "{name}" is now running` |
| `SystemEvent::QuotaEnforced` | (spend / limit summary) | Per-agent quota message |
| `SystemEvent::CronJobCompleted` / `CronJobFailed` | Scheduled job status | Job name + preview or error |
| `SystemEvent::ApprovalPending` | "Approval needed" | Tool / agent / summary |

**Not toasted:** `SystemEvent::HealthCheckFailed` is intentionally **skipped** (too noisy during recovery); use logs and the Web UI for health issues. Most other bus events are also skipped.

The notification listener runs as an async task spawned via `tauri::async_runtime::spawn` and handles broadcast lag gracefully (logs a warning and continues).

---

## IPC Commands

Tauri IPC commands are registered in `crates/openfang-desktop/src/lib.rs` (`invoke_handler!`) and callable from the WebView via `invoke()` (camelCase argument names). The list below is not exhaustive — see the crate for the full set (AINL bootstrap, bookmarks, updater, etc.).

### `get_port`

Returns the port number (`u16`) the embedded server is listening on.

```typescript
// Frontend usage
const port: number = await invoke("get_port");
```

### `get_status`

Returns a JSON object with runtime status:

```json
{
  "status": "running",
  "port": 8042,
  "agents": 5,
  "uptime_secs": 3600
}
```

- `agents` -- count of registered agents from `kernel.registry.list()`.
- `uptime_secs` -- seconds since the kernel state was initialized (via `Instant::now()` at startup).

### `get_agent_count`

Returns the number of registered agents (`usize`) as a simple integer.

```typescript
const count: number = await invoke("get_agent_count");
```

### `import_agent_toml`

Opens a native file picker for `.toml` files. Validates the selected file as an `AgentManifest`, copies it to `~/.armaraos/agents/{name}/agent.toml`, and spawns the agent. Returns the agent name on success.

### `import_skill_file`

Opens a native file picker for skill files (`.md`, `.toml`, `.py`, `.js`, `.wasm`). Copies the file to `~/.armaraos/skills/` and triggers a hot-reload of the skill registry.

### `get_autostart` / `set_autostart`

Check or toggle whether OpenFang launches at OS login. Uses `tauri-plugin-autostart` (launchd on macOS, registry on Windows, systemd on Linux).

### `check_for_updates`

Checks for available updates without installing. Returns an `UpdateInfo` object:

```json
{ "available": true, "version": "0.2.0", "body": "Release notes..." }
```

### `install_update`

Downloads and installs the latest update, then restarts the app. The command does not return on success (the app restarts). Returns an error string on failure.

```typescript
await invoke("install_update"); // App restarts if update succeeds
```

### `open_config_dir` / `open_logs_dir`

Opens `~/.armaraos/` or `~/.armaraos/logs/` in the OS file manager.

### `generate_support_bundle`

Calls the embedded API **`POST /api/support/diagnostics`** on loopback (no separate auth). Returns the same JSON as the HTTP route (`bundle_path`, `bundle_filename`, `relative_path`, …). Used from the Help menu and from the dashboard when generating a bundle from the shell.

### `copy_diagnostics_to_downloads`

Copies a diagnostics `.zip` into the user’s **Downloads** folder (same filename).

**Arguments (object):** **`bundlePath`** (string, required) — use the absolute `bundle_path` from `generate_support_bundle` / `POST /api/support/diagnostics`. Tauri’s schema expects this **camelCase** key; do not pass a nested shape the codegen does not recognize.

The implementation resolves `support/<filename>` when needed so copy succeeds even if path canonicalization is finicky. On failure, the dashboard may fall back to **`GET /api/support/diagnostics/download`** (fetch + save).

### `copy_home_file_to_downloads`

Copies **any file under the ArmaraOS home directory** into Downloads.

**Arguments:** **`relativePath`** — path relative to home, e.g. `support/armaraos-diagnostics-20260405-120000.zip`. Used by **Home folder** → row **Download** on desktop (browser clients use **`GET /api/armaraos-home/download`** instead).

Permissions for dashboard-invoked commands are listed in `crates/openfang-desktop/permissions/dashboard.toml` and mirrored in generated ACL manifests.

### `compose_support_email`

Opens the system mail client with support addressing; on macOS/Linux may attach a validated diagnostics zip path when provided.

**Further reading:** [Dashboard testing — Support diagnostics bundle](dashboard-testing.md#support-diagnostics-bundle-create-download-desktop), [Home folder browser](dashboard-home-folder.md).

---

## Window Configuration

The main window is created programmatically in the `setup` closure (not via `tauri.conf.json`, which declares an empty `windows: []` array):

| Property | Value |
|----------|-------|
| Window label | `"main"` |
| Title | `"OpenFang"` |
| URL | `http://127.0.0.1:{port}` (external) |
| Inner size | 1280 x 800 |
| Minimum inner size | 800 x 600 |
| Position | Centered |

The window uses `WebviewUrl::External(...)` rather than a bundled frontend, because the WebView renders the axum-served UI.

### Auto-Updater

The app checks for updates 10 seconds after startup. If an update is available, it is downloaded, installed, and the app restarts automatically. Users can also trigger a manual check via the system tray.

**Flow:**
1. Startup check (10s delay) → `check_for_update()` → if available → notify user → `download_and_install_update()` → app restarts
2. Tray "Check for Updates" → same flow, with failure notification if install fails

**Configuration** (in `tauri.conf.json`):
- `plugins.updater.pubkey` — Ed25519 public key (must match the signing private key)
- `plugins.updater.endpoints` — URL to `latest.json` (hosted on GitHub Releases)
- `plugins.updater.windows.installMode` — `"passive"` (install without full UI)

**Website-hosted feeds:** Production builds also support updater JSON mirrored to the marketing site: stable **`https://ainativelang.com/downloads/armaraos/latest.json`**, beta channel **`.../beta.json`** (see `crates/openfang-desktop/src/ui_prefs.rs` — `STABLE_FEED_URL` / `BETA_FEED_URL`). CI copies manifests there on each tag; prerelease tags update `beta.json` without overwriting stable `latest.json` (see `docs/release-desktop.md`). The app checks that feed first for **signed** auto-install; it also compares your version to **GitHub’s latest release** when the feed says “up to date” (stale mirror) or when the feed request fails, so you still get a notification and release-page link even if ainativelang.com lags.

**Marketing site (browser downloads):** **[ainativelang.com](https://ainativelang.com)** and **`/download`** surface the same installer set when `latest.json` is present under **`/downloads/armaraos/`**; otherwise they fall back to GitHub release assets (tag pinned in **ainativelangweb** `config/site.ts`). Details in **`docs/release-desktop.md`** (Marketing site installers).

**Signing:** Every release bundle is signed with `TAURI_SIGNING_PRIVATE_KEY` (GitHub Secret). The `tauri-action` generates `latest.json` containing download URLs and signatures for each platform.

See [Production Checklist](production-checklist.md) for key generation and setup instructions.

### CSP

The `tauri.conf.json` configures a Content Security Policy that allows connections to the local embedded server:

```
default-src 'self' http://127.0.0.1:* ws://127.0.0.1:*;
img-src 'self' data: http://127.0.0.1:*;
style-src 'self' 'unsafe-inline';
script-src 'self' 'unsafe-inline'
```

This permits the WebView to load content from the localhost API server while blocking external resource loading. The axum API server provides additional security headers middleware.

---

## Building

### Prerequisites

- **Rust** (stable toolchain)
- **Tauri CLI v2**: `cargo install tauri-cli --version "^2"`
- **Platform-specific dependencies**:
  - **Windows**: WebView2 (included in Windows 10/11), Visual Studio Build Tools
  - **macOS**: Xcode Command Line Tools
  - **Linux**: `libwebkit2gtk-4.1-dev`, `libappindicator3-dev`, `librsvg2-dev`, `libssl-dev`, `build-essential`

### Development

```bash
cd crates/openfang-desktop
cargo tauri dev
```

This launches the app with hot-reload support. The console window is visible in debug builds for tracing output.

### Production Build

```bash
cd crates/openfang-desktop
cargo tauri build
```

This produces platform-specific installers:
- **Windows**: `.msi` and `.exe` (NSIS) installers
- **macOS**: `.dmg` and `.app` bundle
- **Linux**: `.deb`, `.rpm`, and `.AppImage`

The release binary suppresses the console window on Windows via:

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
```

### Bundle Configuration

From `tauri.conf.json`:

```json
{
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": [
      "icons/icon.png",
      "icons/32x32.png",
      "icons/128x128.png",
      "icons/128x128@2x.png"
    ]
  }
}
```

The `"targets": "all"` setting generates every available package format for the current platform. Icons are provided at multiple resolutions, plus an `icon.ico` for Windows.

---

## Plugins

| Plugin | Version | Purpose |
|--------|---------|---------|
| `tauri-plugin-notification` | 2 | Native OS notifications for kernel events and update progress |
| `tauri-plugin-shell` | 2 | Shell/process access from the WebView |
| `tauri-plugin-dialog` | 2 | Native file picker for agent/skill import |
| `tauri-plugin-single-instance` | 2 | Prevents multiple instances (desktop only) |
| `tauri-plugin-autostart` | 2 | Launch at OS login (desktop only) |
| `tauri-plugin-updater` | 2 | Signed auto-updates from GitHub Releases (desktop only) |
| `tauri-plugin-global-shortcut` | 2 | Ctrl+Shift+O/N/C shortcuts (desktop only) |

### Capabilities

The default capability set (defined in `capabilities/default.json`) grants:

```json
{
  "identifier": "default",
  "windows": ["main"],
  "permissions": [
    "core:default",
    "notification:default",
    "shell:default",
    "dialog:default",
    "global-shortcut:allow-register",
    "global-shortcut:allow-unregister",
    "global-shortcut:allow-is-registered",
    "autostart:default",
    "updater:default"
  ]
}
```

Only the `"main"` window receives these permissions.

---

## Mobile Ready

The codebase includes conditional compilation guards for mobile platform support:

- **Entry point**: The `run()` function is annotated with `#[cfg_attr(mobile, tauri::mobile_entry_point)]`, allowing Tauri to use it as the mobile entry point.
- **Desktop-only features**: System tray setup, single-instance enforcement, and hide-to-tray on close are all gated behind `#[cfg(desktop)]` so they compile out on mobile targets.
- **Mobile targets**: iOS and Android builds are structurally supported by the Tauri 2.0 framework, though the kernel and API server would still boot in-process on the device.

---

## File Structure

```
crates/openfang-desktop/
  build.rs                 # tauri_build::build()
  Cargo.toml               # Crate dependencies and metadata
  tauri.conf.json           # Tauri app configuration
  capabilities/
    default.json            # Permission grants for the main window
  gen/
    schemas/                # Auto-generated Tauri schemas
  icons/
    icon.png                # Source icon (327 KB)
    icon.ico                # Windows icon
    32x32.png               # Small icon
    128x128.png             # Standard icon
    128x128@2x.png          # HiDPI icon
  src/
    main.rs                 # Binary entry point (calls lib::run())
    lib.rs                  # Tauri app builder, state types, event listener
    commands.rs             # IPC command handlers (get_port, get_status, get_agent_count)
    server.rs               # ServerHandle, kernel boot, embedded axum server
    tray.rs                 # System tray menu and event handlers
```

---

## Environment Variables

| Variable | Effect |
|----------|--------|
| `RUST_LOG` | Controls tracing verbosity. Defaults to `openfang=info,tauri=info` if unset. |

All other OpenFang environment variables (API keys, configuration) apply as normal since the desktop app boots the same kernel as the headless daemon.
