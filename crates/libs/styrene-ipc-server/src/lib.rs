//! Unix socket IPC server for the Styrene daemon.
//!
//! Exposes an [`Arc<dyn Daemon>`](styrene_ipc::traits::Daemon) over a framed
//! msgpack wire protocol on a Unix domain socket. Wire-compatible with the
//! Python `styrened.ipc` protocol, enabling the Python TUI to connect to
//! `styrened-rs` as a drop-in daemon replacement.
//!
//! # Quick Start
//!
//! ```rust,no_run
//! use std::sync::Arc;
//! use styrene_ipc_server::{IpcServer, IpcServerConfig};
//!
//! # async fn run(daemon: Arc<dyn styrene_ipc::traits::Daemon>) -> std::io::Result<()> {
//! let config = IpcServerConfig::default();
//! let mut server = IpcServer::new(daemon, config);
//! server.start().await?;
//! // ... server runs until stopped
//! server.stop().await;
//! # Ok(())
//! # }
//! ```

pub mod wire;
pub mod connection;
pub mod dispatch;
pub mod server;

pub use server::{IpcServer, IpcServerConfig, default_socket_path};
