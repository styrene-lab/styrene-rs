//! NullTransport — null object pattern for standalone/test mode.
//!
//! All sends return `TransportError::Unavailable`. Subscriptions return
//! receivers that never produce events. `is_connected()` returns `false`.
//! Eliminates `Option<Arc<dyn MeshTransport>>` throughout services.

use super::mesh_transport::{MeshTransport, TransportError, TransportLifecycleEvent};
use rns_core::destination::DestinationDesc;
use rns_core::hash::AddressHash;
use rns_core::identity::Identity;
use rns_core::transport::core_transport::{AnnounceEvent, ReceivedData, SendPacketOutcome};
use rns_core::transport::delivery::LinkSendResult;
use std::time::Duration;
use tokio::sync::broadcast;

/// A transport that does nothing — for standalone and test configurations.
pub struct NullTransport {
    /// Lifecycle channel kept alive so `subscribe_lifecycle()` returns a valid receiver.
    _lifecycle_tx: broadcast::Sender<TransportLifecycleEvent>,
}

impl NullTransport {
    pub fn new() -> Self {
        let (_lifecycle_tx, _) = broadcast::channel(1);
        Self { _lifecycle_tx }
    }
}

impl Default for NullTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl MeshTransport for NullTransport {
    async fn send_raw(
        &self,
        _dest: AddressHash,
        _data: &[u8],
    ) -> Result<SendPacketOutcome, TransportError> {
        Err(TransportError::Unavailable)
    }

    async fn send_via_link(
        &self,
        _dest: DestinationDesc,
        _data: &[u8],
        _timeout: Duration,
    ) -> Result<LinkSendResult, TransportError> {
        Err(TransportError::Unavailable)
    }

    async fn request_path(&self, _dest: &AddressHash) {
        // no-op
    }

    async fn resolve_identity(&self, _dest: &AddressHash) -> Option<Identity> {
        None
    }

    async fn announce(&self, _app_data: Option<&[u8]>) {
        // no-op
    }

    fn subscribe_inbound(&self) -> broadcast::Receiver<ReceivedData> {
        // Create a channel and immediately drop the sender — receiver will
        // return RecvError::Closed on any recv attempt.
        let (tx, rx) = broadcast::channel(1);
        drop(tx);
        rx
    }

    fn subscribe_announces(&self) -> broadcast::Receiver<AnnounceEvent> {
        let (tx, rx) = broadcast::channel(1);
        drop(tx);
        rx
    }

    fn subscribe_lifecycle(&self) -> broadcast::Receiver<TransportLifecycleEvent> {
        self._lifecycle_tx.subscribe()
    }

    fn identity_hash(&self) -> AddressHash {
        AddressHash::new([0u8; 16])
    }

    fn destination_hash(&self) -> AddressHash {
        AddressHash::new([0u8; 16])
    }

    fn is_connected(&self) -> bool {
        false
    }

    async fn shutdown(&self) -> Result<(), TransportError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn send_raw_returns_unavailable() {
        let transport = NullTransport::new();
        let dest = AddressHash::new([1u8; 16]);
        let result = transport.send_raw(dest, b"hello").await;
        assert!(matches!(result, Err(TransportError::Unavailable)));
    }

    #[tokio::test]
    async fn send_via_link_returns_unavailable() {
        let transport = NullTransport::new();
        let identity = rns_core::identity::PrivateIdentity::new_from_name("test");
        let desc = DestinationDesc {
            identity: *identity.as_identity(),
            address_hash: AddressHash::new([2u8; 16]),
            name: rns_core::destination::DestinationName::new("test", "app"),
        };
        let result = transport
            .send_via_link(desc, b"hello", Duration::from_secs(5))
            .await;
        assert!(matches!(result, Err(TransportError::Unavailable)));
    }

    #[tokio::test]
    async fn resolve_identity_returns_none() {
        let transport = NullTransport::new();
        let dest = AddressHash::new([3u8; 16]);
        assert!(transport.resolve_identity(&dest).await.is_none());
    }

    #[test]
    fn is_connected_returns_false() {
        let transport = NullTransport::new();
        assert!(!transport.is_connected());
    }

    #[test]
    fn identity_hash_is_zero() {
        let transport = NullTransport::new();
        assert_eq!(transport.identity_hash(), AddressHash::new([0u8; 16]));
    }

    #[test]
    fn destination_hash_is_zero() {
        let transport = NullTransport::new();
        assert_eq!(transport.destination_hash(), AddressHash::new([0u8; 16]));
    }

    #[tokio::test]
    async fn shutdown_succeeds() {
        let transport = NullTransport::new();
        assert!(transport.shutdown().await.is_ok());
    }

    #[test]
    fn subscribe_inbound_channel_is_closed() {
        let transport = NullTransport::new();
        let mut rx = transport.subscribe_inbound();
        // Channel sender was dropped, so try_recv returns Closed
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn subscribe_announces_channel_is_closed() {
        let transport = NullTransport::new();
        let mut rx = transport.subscribe_announces();
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn subscribe_lifecycle_returns_valid_receiver() {
        let transport = NullTransport::new();
        let mut rx = transport.subscribe_lifecycle();
        // Receiver is valid but empty (no events sent)
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn default_is_same_as_new() {
        let t1 = NullTransport::new();
        let t2 = NullTransport::default();
        assert_eq!(t1.is_connected(), t2.is_connected());
        assert_eq!(t1.identity_hash(), t2.identity_hash());
    }
}
