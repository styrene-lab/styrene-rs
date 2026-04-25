//! Styrene Services — domain service abstractions for the Styrene mesh daemon.
//!
//! Provides transport-independent service implementations that can be composed
//! into a daemon runtime via `AppContext`. Each service owns a focused domain
//! and communicates with peers through shared stores and event channels.
//!
//! ## Services
//!
//! | Service | Domain | Status |
//! |---------|--------|--------|
//! | [`conversations`] | Message threading, unread counts | Implemented |
//! | [`node_store`] | Persistent peer registry, announce ingestion | Implemented |
//! | [`protocol_registry`] | Pluggable per-type message handlers | Implemented |
//! | `propagation` | Store-and-forward for offline peers | Planned |
//! | `file_transfer` | Chunked file delivery over links | Planned |
//! | `hub_connection` | Auto-connect to hub transport | Planned |

pub mod conversations;
pub mod node_store;
pub mod protocol_registry;

/// Common error type for service operations.
#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("storage error: {0}")]
    Storage(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
}
