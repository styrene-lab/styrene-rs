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
//! Five focused traits combine into one composite:
//!
//! - [`DaemonMessaging`] — chat, conversations, contacts
//! - [`DaemonIdentity`] — local node identity
//! - [`DaemonStatus`] — health, config, device discovery
//! - [`DaemonFleet`] — remote device operations, terminal sessions
//! - [`DaemonEvents`] — event subscriptions via broadcast channels
//! - [`Daemon`] — composite (auto-implemented for all five)

pub mod error;
pub mod traits;
pub mod types;

pub use error::IpcError;
pub use traits::{
    Daemon, DaemonEvents, DaemonFleet, DaemonIdentity, DaemonMessaging, DaemonStatus,
};
pub use types::*;

mod stub;
pub use stub::StubDaemon;
