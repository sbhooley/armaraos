//! Kernel lifecycle management for the desktop app.
//!
//! Boots the ArmaraOS kernel (legacy OpenFang naming in crate IDs), binds to a random localhost port, and runs the
//! API server on a background thread with its own tokio runtime.

use openfang_api::server::build_router;
use openfang_kernel::config::{
    apply_desktop_bundled_llm_defaults, default_config_path, load_config, openfang_home,
};
use openfang_kernel::OpenFangKernel;
use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::watch;
use tracing::{error, info};

/// Handle to the running embedded server. Drop or call `shutdown()` to stop.
pub struct ServerHandle {
    /// The port the server is listening on.
    pub port: u16,
    /// The kernel instance (shared with the server).
    pub kernel: Arc<OpenFangKernel>,
    /// Send `true` to trigger graceful shutdown.
    shutdown_tx: watch::Sender<bool>,
    /// Join handle for the background server thread.
    server_thread: Option<std::thread::JoinHandle<()>>,
    /// Track whether shutdown has already been initiated to prevent double shutdown.
    shutdown_initiated: Arc<AtomicBool>,
}

impl ServerHandle {
    /// Signal the server to shut down and wait for the background thread.
    pub fn shutdown(mut self) {
        // Only proceed if shutdown hasn't been initiated yet
        if self
            .shutdown_initiated
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::Relaxed)
            .is_ok()
        {
            let _ = self.shutdown_tx.send(true);
            if let Some(handle) = self.server_thread.take() {
                let _ = handle.join();
            }
            self.kernel.shutdown();
            info!("ArmaraOS embedded server stopped");
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        // Only send shutdown signal if it hasn't been initiated yet
        if self
            .shutdown_initiated
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::Relaxed)
            .is_ok()
        {
            let _ = self.shutdown_tx.send(true);
            // Best-effort: don't block in drop, the thread will exit on its own.
        }
    }
}

/// Boot the kernel and start the embedded API server on a background thread.
///
/// Binds to `127.0.0.1:0` on the calling thread so the port is known before
/// any Tauri window is created. The actual axum server runs on a dedicated
/// thread with its own tokio runtime.
pub fn start_server() -> Result<ServerHandle, Box<dyn std::error::Error>> {
    // Load .env and secrets.env into process environment (same as CLI).
    // Without this, API keys stored in ~/.armaraos/.env (or legacy ~/.openfang/.env) are invisible to
    // the kernel's provider detection and credential resolver.
    load_dotenv_files();

    // AINL: if unset, allow IR-declared adapters without requiring users to export host env.
    // `~/.armaraos/.env` or the system environment can still set this explicitly (load_dotenv does not override).
    if std::env::var("AINL_ALLOW_IR_DECLARED_ADAPTERS").is_err() {
        std::env::set_var("AINL_ALLOW_IR_DECLARED_ADAPTERS", "1");
    }

    // Bind first so `kernel.config.api_listen` matches the real socket. Scheduled AINL programs
    // receive `ARMARAOS_DAEMON_BASE_URL` from `scheduled_ainl_api_base_url(api_listen)`; if we
    // booted the kernel with the on-disk `api_listen` while serving on a random port, HTTP graphs
    // would call the wrong host:port (connection refused).
    let std_listener = TcpListener::bind("127.0.0.1:0")?;
    let listen_addr: SocketAddr = std_listener.local_addr()?;
    let port = listen_addr.port();

    let mut config = load_config(None);
    if !default_config_path().exists() {
        apply_desktop_bundled_llm_defaults(&mut config);
    }
    config.api_listen = listen_addr.to_string();
    // `boot_with_config` overrides `api_listen` from `OPENFANG_LISTEN` if set; clear for this boot
    // so our bound address wins, then restore so user/CI env is unchanged.
    let saved_openfang_listen = std::env::var("OPENFANG_LISTEN").ok();
    std::env::remove_var("OPENFANG_LISTEN");
    let kernel = OpenFangKernel::boot_with_config(config)?;
    if let Some(v) = saved_openfang_listen {
        std::env::set_var("OPENFANG_LISTEN", v);
    }

    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    info!("ArmaraOS embedded server bound to http://127.0.0.1:{port}");

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let kernel_clone = kernel.clone();
    let shutdown_initiated = Arc::new(AtomicBool::new(false));

    let server_thread = std::thread::Builder::new()
        .name("armaraos-server".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("Failed to create tokio runtime for embedded server");

            rt.block_on(async move {
                // start_background_agents() uses tokio::spawn, so it must
                // run inside a tokio runtime context.
                kernel_clone.start_background_agents();
                run_embedded_server(kernel_clone, std_listener, listen_addr, shutdown_rx).await;
            });
        })?;

    Ok(ServerHandle {
        port,
        kernel,
        shutdown_tx,
        server_thread: Some(server_thread),
        shutdown_initiated,
    })
}

/// Run the axum server inside a tokio runtime, shut down when the watch
/// channel fires.
async fn run_embedded_server(
    kernel: Arc<OpenFangKernel>,
    std_listener: TcpListener,
    listen_addr: SocketAddr,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let (app, state) = build_router(kernel, listen_addr).await;

    // Convert std TcpListener → tokio TcpListener
    std_listener
        .set_nonblocking(true)
        .expect("Failed to set listener to non-blocking");
    let listener = tokio::net::TcpListener::from_std(std_listener)
        .expect("Failed to convert std TcpListener to tokio");

    info!("ArmaraOS embedded server listening on http://{listen_addr}");

    let server = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        let _ = shutdown_rx.wait_for(|v| *v).await;
        info!("Embedded server received shutdown signal");
    });

    if let Err(e) = server.await {
        error!("Embedded server error: {e}");
    }

    // Clean up channel bridges
    {
        let mut guard = state.bridge_manager.lock().await;
        if let Some(ref mut b) = *guard {
            b.stop().await;
        }
    }
}

/// Load ~/.armaraos/.env (or legacy ~/.openfang/.env) and secrets.env into the process environment.
/// Runs after [`openfang_home`] so migration/create matches the kernel config path.
/// System env vars take priority — existing vars are NOT overridden.
fn load_dotenv_files() {
    let home = openfang_home();

    for filename in &[".env", "secrets.env"] {
        let path = home.join(filename);
        if let Ok(content) = std::fs::read_to_string(&path) {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                if let Some((key, value)) = trimmed.split_once('=') {
                    let key = key.trim();
                    let mut value = value.trim().to_string();
                    if ((value.starts_with('"') && value.ends_with('"'))
                        || (value.starts_with('\'') && value.ends_with('\'')))
                        && value.len() >= 2
                    {
                        value = value[1..value.len() - 1].to_string();
                    }
                    if !key.is_empty() && std::env::var(key).is_err() {
                        std::env::set_var(key, &value);
                    }
                }
            }
        }
    }
}
