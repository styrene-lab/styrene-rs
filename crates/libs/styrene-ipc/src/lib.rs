//! Interface boundary traits for the styrene daemon.
//!
//! This crate defines the IPC contract between the styrene daemon and its
//! frontends (TUI, GUI, web bridge). It provides:
//!
//! - **Boundary types** matching what the Python TUI consumes
//! - **Async trait definitions** capturing the full IPC contract
//! - **`StubDaemon`** returning `NotImplemented` for every method
//! - **`IpcError`** with a `NotImplemented` variant for incremental development
//!
//! # Trait hierarchy
//!
//! Seven focused traits combine into one composite:
//!
//! - [`DaemonMessaging`] — chat, conversations, contacts
//! - [`DaemonIdentity`] — local node identity
//! - [`DaemonStatus`] — health, config, device discovery
//! - [`DaemonFleet`] — remote device operations, terminal sessions
//! - [`DaemonEvents`] — event subscriptions via broadcast channels
//! - [`DaemonTunnel`] — VPN tunnel management
//! - [`DaemonPages`] — NomadNet page browsing
//! - [`Daemon`] — composite (auto-implemented for all seven)

pub mod error;
pub mod traits;
pub mod types;

pub use error::IpcError;
pub use traits::{
    Daemon, DaemonEvents, DaemonFleet, DaemonIdentity, DaemonMessaging, DaemonPages, DaemonStatus,
    DaemonTunnel,
};
pub use types::*;

mod stub;
pub use stub::StubDaemon;
