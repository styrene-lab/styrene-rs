//! MeshTransport — daemon-internal transport abstraction trait.
//!
//! Thin wrapper over `rns_core::transport::core_transport::Transport`.
//! The delivery pipeline (path request → identity poll → link attempt →
//! opportunistic fallback → receipt tracking) lives in `MessagingService`,
//! not behind this trait.
//!
//! Design: Option C (split levels) — see ownership-matrix.md §MeshTransport.

use rns_core::destination::DestinationDesc;
use rns_core::hash::AddressHash;
use rns_core::identity::Identity;
use rns_core::transport::core_transport::{AnnounceEvent, ReceivedData, SendPacketOutcome};
use rns_core::transport::delivery::LinkSendResult;
use std::time::Duration;
use tokio::sync::broadcast;

/// Transport lifecycle events — services subscribe to react to connectivity changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportLifecycleEvent {
    Connected,
    Disconnected,
    Reconnected,
}

/// Errors from transport operations.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("transport unavailable")]
    Unavailable,
    #[error("send failed: {0}")]
    SendFailed(String),
    #[error("link failed: {0}")]
    LinkFailed(#[from] std::io::Error),
    #[error("shutdown failed: {0}")]
    ShutdownFailed(String),
}

/// Daemon-internal transport abstraction.
///
/// Wraps raw transport operations for testability and future backend flexibility.
/// All consumers are services inside the daemon app crate — this trait is NOT
/// promoted to `styrene-ipc` (frontends have no transport dependency).
///
/// Implementations:
/// - `TokioTransportAdapter` — wraps the real `rns_core::Transport`
/// - `NullTransport` — null object for standalone/test mode
/// - `MockTransport` — deterministic mock for service tests (Package C)
#[async_trait::async_trait]
pub trait MeshTransport: Send + Sync {
    // --- Sending ---

    /// Opportunistic single-packet send (broadcast, no link setup).
    async fn send_raw(
        &self,
        dest: AddressHash,
        data: &[u8],
    ) -> Result<SendPacketOutcome, TransportError>;

    /// Link-based reliable send (with resource fallback for large payloads).
    /// Caller must provide a fully-resolved `DestinationDesc` (includes peer Identity).
    async fn send_via_link(
        &self,
        dest: DestinationDesc,
        data: &[u8],
        timeout: Duration,
    ) -> Result<LinkSendResult, TransportError>;

    // --- Discovery ---

    /// Trigger path request for a destination.
    async fn request_path(&self, dest: &AddressHash);

    /// Look up peer identity from transport's announce table.
    /// Returns `None` if identity not yet known (peer hasn't announced).
    async fn resolve_identity(&self, dest: &AddressHash) -> Option<Identity>;

    // --- Announcing ---

    /// Send announce with optional app_data.
    async fn announce(&self, app_data: Option<&[u8]>);

    // --- Subscriptions (broadcast channels) ---

    /// Subscribe to inbound data events (decoded payloads delivered to our destination).
    fn subscribe_inbound(&self) -> broadcast::Receiver<ReceivedData>;

    /// Subscribe to announce events from other nodes.
    fn subscribe_announces(&self) -> broadcast::Receiver<AnnounceEvent>;

    /// Subscribe to transport lifecycle transitions (connected/disconnected/reconnected).
    fn subscribe_lifecycle(&self) -> broadcast::Receiver<TransportLifecycleEvent>;

    // --- State queries ---

    /// Our identity address hash.
    fn identity_hash(&self) -> AddressHash;

    /// Our delivery destination hash.
    fn destination_hash(&self) -> AddressHash;

    /// Whether transport is currently connected/operational.
    fn is_connected(&self) -> bool;

    // --- Lifecycle ---

    /// Shut down the transport gracefully.
    async fn shutdown(&self) -> Result<(), TransportError>;
}
