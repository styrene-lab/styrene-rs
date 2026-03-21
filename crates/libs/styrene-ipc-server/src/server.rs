//! Unix socket IPC server.
//!
//! Listens on a Unix domain socket, accepts client connections, and dispatches
//! IPC requests to an [`Arc<dyn Daemon>`] implementation.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::net::UnixListener;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use styrene_ipc::traits::Daemon;
use styrene_ipc::types::DaemonEvent;

use crate::connection;

/// Configuration for the IPC server.
#[derive(Debug, Clone)]
pub struct IpcServerConfig {
    /// Path to the Unix socket file.
    pub socket_path: PathBuf,

    /// Event broadcast channel capacity.
    pub event_capacity: usize,
}

impl Default for IpcServerConfig {
    fn default() -> Self {
        Self {
            socket_path: default_socket_path(),
            event_capacity: 256,
        }
    }
}

/// The IPC server. Call [`start`] to begin accepting connections.
pub struct IpcServer {
    config: IpcServerConfig,
    daemon: Arc<dyn Daemon>,
    event_tx: broadcast::Sender<DaemonEvent>,
    accept_handle: Option<JoinHandle<()>>,
}

impl IpcServer {
    /// Create a new IPC server.
    pub fn new(daemon: Arc<dyn Daemon>, config: IpcServerConfig) -> Self {
        let (event_tx, _) = broadcast::channel(config.event_capacity);
        Self {
            config,
            daemon,
            event_tx,
            accept_handle: None,
        }
    }

    /// Get a sender for pushing events to all subscribed clients.
    pub fn event_sender(&self) -> broadcast::Sender<DaemonEvent> {
        self.event_tx.clone()
    }

    /// Start the IPC server (non-blocking — spawns accept loop).
    pub async fn start(&mut self) -> std::io::Result<()> {
        // Remove stale socket file
        let _ = std::fs::remove_file(&self.config.socket_path);

        // Ensure parent directory exists
        if let Some(parent) = self.config.socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let listener = UnixListener::bind(&self.config.socket_path)?;
        log::info!(
            "IPC server listening on {}",
            self.config.socket_path.display()
        );

        // Set socket permissions (owner-only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(&self.config.socket_path, perms)?;
        }

        let daemon = self.daemon.clone();
        let event_tx = self.event_tx.clone();

        let handle = tokio::spawn(async move {
            accept_loop(listener, daemon, event_tx).await;
        });

        self.accept_handle = Some(handle);
        Ok(())
    }

    /// Stop the IPC server, cleaning up the socket file.
    pub async fn stop(&mut self) {
        if let Some(handle) = self.accept_handle.take() {
            handle.abort();
            let _ = handle.await;
        }
        // Remove socket file
        let _ = std::fs::remove_file(&self.config.socket_path);
        log::info!("IPC server stopped");
    }

    /// Returns the socket path for client connections.
    pub fn socket_path(&self) -> &Path {
        &self.config.socket_path
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        if let Some(handle) = self.accept_handle.take() {
            handle.abort();
        }
        let _ = std::fs::remove_file(&self.config.socket_path);
    }
}

/// Accept loop — runs until aborted.
async fn accept_loop(
    listener: UnixListener,
    daemon: Arc<dyn Daemon>,
    event_tx: broadcast::Sender<DaemonEvent>,
) {
    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let d = daemon.clone();
                let rx = event_tx.subscribe();
                let (read_half, write_half) = stream.into_split();
                tokio::spawn(async move {
                    connection::handle_client(d, read_half, write_half, rx).await;
                });
            }
            Err(e) => {
                log::error!("IPC accept error: {e}");
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
        }
    }
}

/// Determine the default socket path.
///
/// Respects `STYRENED_SOCKET` env var, then defaults to `~/.styrene/styrened.sock`.
pub fn default_socket_path() -> PathBuf {
    if let Ok(path) = std::env::var("STYRENED_SOCKET") {
        return PathBuf::from(path);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".styrene").join("styrened.sock")
}
