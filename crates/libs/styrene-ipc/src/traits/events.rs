use async_trait::async_trait;
use tokio::sync::broadcast;

use crate::error::IpcError;
use crate::types::DaemonEvent;

/// Event subscriptions via `tokio::sync::broadcast`.
#[async_trait]
pub trait DaemonEvents: Send + Sync {
    /// Subscribe to message events, optionally filtered to specific peers.
    /// An empty slice subscribes to all message events.
    async fn subscribe_messages(
        &self,
        peer_hashes: &[String],
    ) -> Result<broadcast::Receiver<DaemonEvent>, IpcError>;

    /// Subscribe to device discovery/status events.
    async fn subscribe_devices(&self) -> Result<broadcast::Receiver<DaemonEvent>, IpcError>;
}
