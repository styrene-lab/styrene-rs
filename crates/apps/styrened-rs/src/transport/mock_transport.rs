//! MockTransport — deterministic mock for service-level tests.
//!
//! Provides controllable behavior for all MeshTransport methods:
//! - Injectable send results (per-call or default)
//! - Injectable identity resolution results
//! - Programmable inbound/announce/lifecycle event injection
//! - Call recording for assertion
//!
//! Package C — see ownership-matrix.md §MeshTransport.

use super::mesh_transport::{MeshTransport, TransportError, TransportLifecycleEvent};
use rns_core::destination::DestinationDesc;
use rns_core::hash::AddressHash;
use rns_core::identity::Identity;
use rns_core::transport::core_transport::{AnnounceEvent, ReceivedData, SendPacketOutcome};
use rns_core::transport::delivery::LinkSendResult;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::broadcast;

/// Recorded call for assertion.
#[derive(Debug, Clone)]
pub enum MockCall {
    SendRaw {
        dest: AddressHash,
        data: Vec<u8>,
    },
    SendViaLink {
        dest_hash: AddressHash,
        data: Vec<u8>,
        timeout: Duration,
    },
    RequestPath {
        dest: AddressHash,
    },
    ResolveIdentity {
        dest: AddressHash,
    },
    Announce {
        app_data: Option<Vec<u8>>,
    },
    Shutdown,
}

/// Configurable mock transport for testing daemon services.
pub struct MockTransport {
    identity_addr: AddressHash,
    destination_addr: AddressHash,
    connected: Mutex<bool>,

    // Programmable results
    send_raw_results: Mutex<VecDeque<Result<SendPacketOutcome, TransportError>>>,
    send_link_results: Mutex<VecDeque<Result<LinkSendResult, TransportError>>>,
    resolve_results: Mutex<VecDeque<Option<Identity>>>,

    // Default behavior for send_raw when queue is exhausted
    default_send_raw: Mutex<Result<SendPacketOutcome, TransportError>>,

    // Event injection channels
    inbound_tx: broadcast::Sender<ReceivedData>,
    announce_tx: broadcast::Sender<AnnounceEvent>,
    lifecycle_tx: broadcast::Sender<TransportLifecycleEvent>,

    // Call recording
    calls: Mutex<Vec<MockCall>>,
}

impl MockTransport {
    /// Create a new mock with the given identity and destination hashes.
    pub fn new(identity_addr: AddressHash, destination_addr: AddressHash) -> Self {
        let (inbound_tx, _) = broadcast::channel(64);
        let (announce_tx, _) = broadcast::channel(64);
        let (lifecycle_tx, _) = broadcast::channel(16);

        Self {
            identity_addr,
            destination_addr,
            connected: Mutex::new(true),
            send_raw_results: Mutex::new(VecDeque::new()),
            send_link_results: Mutex::new(VecDeque::new()),
            resolve_results: Mutex::new(VecDeque::new()),
            default_send_raw: Mutex::new(Ok(SendPacketOutcome::SentDirect)),
            inbound_tx,
            announce_tx,
            lifecycle_tx,
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Create a mock with zero hashes (convenience for simple tests).
    pub fn new_default() -> Self {
        Self::new(AddressHash::new([0u8; 16]), AddressHash::new([0u8; 16]))
    }

    // --- Configuration ---

    /// Queue a specific result for the next `send_raw` call.
    pub fn queue_send_raw(&self, result: Result<SendPacketOutcome, TransportError>) {
        self.send_raw_results.lock().unwrap().push_back(result);
    }

    /// Queue a specific result for the next `send_via_link` call.
    pub fn queue_send_link(&self, result: Result<LinkSendResult, TransportError>) {
        self.send_link_results.lock().unwrap().push_back(result);
    }

    /// Queue a specific result for the next `resolve_identity` call.
    pub fn queue_resolve(&self, identity: Option<Identity>) {
        self.resolve_results.lock().unwrap().push_back(identity);
    }

    /// Set the connected state.
    pub fn set_connected(&self, connected: bool) {
        *self.connected.lock().unwrap() = connected;
    }

    // --- Event injection ---

    /// Inject an inbound data event (simulates receiving data from mesh).
    pub fn inject_inbound(&self, data: ReceivedData) {
        let _ = self.inbound_tx.send(data);
    }

    /// Inject an announce event (simulates receiving an announce from mesh).
    pub fn inject_announce(&self, event: AnnounceEvent) {
        let _ = self.announce_tx.send(event);
    }

    /// Inject a lifecycle event.
    pub fn inject_lifecycle(&self, event: TransportLifecycleEvent) {
        let _ = self.lifecycle_tx.send(event);
    }

    // --- Inspection ---

    /// Get all recorded calls.
    pub fn calls(&self) -> Vec<MockCall> {
        self.calls.lock().unwrap().clone()
    }

    /// Get the number of recorded calls.
    pub fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    /// Clear recorded calls.
    pub fn clear_calls(&self) {
        self.calls.lock().unwrap().clear();
    }

    fn record(&self, call: MockCall) {
        self.calls.lock().unwrap().push(call);
    }
}

#[async_trait::async_trait]
impl MeshTransport for MockTransport {
    async fn send_raw(
        &self,
        dest: AddressHash,
        data: &[u8],
    ) -> Result<SendPacketOutcome, TransportError> {
        self.record(MockCall::SendRaw {
            dest,
            data: data.to_vec(),
        });
        self.send_raw_results
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| self.default_send_raw.lock().unwrap().clone())
    }

    async fn send_via_link(
        &self,
        dest: DestinationDesc,
        data: &[u8],
        timeout: Duration,
    ) -> Result<LinkSendResult, TransportError> {
        self.record(MockCall::SendViaLink {
            dest_hash: dest.address_hash,
            data: data.to_vec(),
            timeout,
        });
        self.send_link_results
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| {
                // Default: return Unavailable (no queued result)
                Err(TransportError::Unavailable)
            })
    }

    async fn request_path(&self, dest: &AddressHash) {
        self.record(MockCall::RequestPath { dest: *dest });
    }

    async fn resolve_identity(&self, dest: &AddressHash) -> Option<Identity> {
        self.record(MockCall::ResolveIdentity { dest: *dest });
        self.resolve_results.lock().unwrap().pop_front().flatten()
    }

    async fn announce(&self, app_data: Option<&[u8]>) {
        self.record(MockCall::Announce {
            app_data: app_data.map(|d| d.to_vec()),
        });
    }

    fn subscribe_inbound(&self) -> broadcast::Receiver<ReceivedData> {
        self.inbound_tx.subscribe()
    }

    fn subscribe_announces(&self) -> broadcast::Receiver<AnnounceEvent> {
        self.announce_tx.subscribe()
    }

    fn subscribe_lifecycle(&self) -> broadcast::Receiver<TransportLifecycleEvent> {
        self.lifecycle_tx.subscribe()
    }

    async fn query_path(&self, _dest: &AddressHash) -> Option<(u8, AddressHash)> {
        None // Mock doesn't track paths
    }

    fn identity_hash(&self) -> AddressHash {
        self.identity_addr
    }

    fn destination_hash(&self) -> AddressHash {
        self.destination_addr
    }

    fn is_connected(&self) -> bool {
        *self.connected.lock().unwrap()
    }

    async fn shutdown(&self) -> Result<(), TransportError> {
        self.record(MockCall::Shutdown);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rns_core::packet::PacketDataBuffer;
    use rns_core::transport::core_transport::ReceivedPayloadMode;

    #[tokio::test]
    async fn send_raw_records_call_and_returns_default() {
        let mock = MockTransport::new_default();
        let dest = AddressHash::new([1u8; 16]);
        let result = mock.send_raw(dest, b"hello").await;
        assert!(matches!(result, Ok(SendPacketOutcome::SentDirect)));
        assert_eq!(mock.call_count(), 1);
        assert!(matches!(&mock.calls()[0], MockCall::SendRaw { dest: d, data } if *d == dest && data == b"hello"));
    }

    #[tokio::test]
    async fn send_raw_uses_queued_results_then_default() {
        let mock = MockTransport::new_default();
        let dest = AddressHash::new([1u8; 16]);
        mock.queue_send_raw(Err(TransportError::SendFailed("test".into())));
        mock.queue_send_raw(Ok(SendPacketOutcome::SentBroadcast));

        let r1 = mock.send_raw(dest, b"a").await;
        assert!(matches!(r1, Err(TransportError::SendFailed(_))));

        let r2 = mock.send_raw(dest, b"b").await;
        assert!(matches!(r2, Ok(SendPacketOutcome::SentBroadcast)));

        // Queue exhausted — falls back to default
        let r3 = mock.send_raw(dest, b"c").await;
        assert!(matches!(r3, Ok(SendPacketOutcome::SentDirect)));
    }

    #[tokio::test]
    async fn resolve_identity_returns_queued_then_none() {
        let mock = MockTransport::new_default();
        let dest = AddressHash::new([2u8; 16]);
        let id = rns_core::identity::PrivateIdentity::new_from_name("peer");
        mock.queue_resolve(Some(*id.as_identity()));

        let r1 = mock.resolve_identity(&dest).await;
        assert!(r1.is_some());

        let r2 = mock.resolve_identity(&dest).await;
        assert!(r2.is_none());
    }

    #[tokio::test]
    async fn inbound_injection_reaches_subscriber() {
        let mock = MockTransport::new_default();
        let mut rx = mock.subscribe_inbound();

        let data = ReceivedData {
            destination: AddressHash::new([3u8; 16]),
            data: PacketDataBuffer::new_from_slice(b"test payload"),
            payload_mode: ReceivedPayloadMode::FullWire,
            ratchet_used: false,
            context: None,
            request_id: None,
            hops: Some(2),
            interface: None,
        };
        mock.inject_inbound(data);

        let received = rx.recv().await.unwrap();
        assert_eq!(received.destination, AddressHash::new([3u8; 16]));
        assert_eq!(received.hops, Some(2));
    }

    #[tokio::test]
    async fn lifecycle_injection_reaches_subscriber() {
        let mock = MockTransport::new_default();
        let mut rx = mock.subscribe_lifecycle();

        mock.inject_lifecycle(TransportLifecycleEvent::Reconnected);

        let event = rx.recv().await.unwrap();
        assert_eq!(event, TransportLifecycleEvent::Reconnected);
    }

    #[test]
    fn connected_state_is_controllable() {
        let mock = MockTransport::new_default();
        assert!(mock.is_connected());
        mock.set_connected(false);
        assert!(!mock.is_connected());
    }

    #[tokio::test]
    async fn request_path_records_call() {
        let mock = MockTransport::new_default();
        let dest = AddressHash::new([4u8; 16]);
        mock.request_path(&dest).await;
        assert_eq!(mock.call_count(), 1);
        assert!(matches!(&mock.calls()[0], MockCall::RequestPath { dest: d } if *d == dest));
    }

    #[tokio::test]
    async fn announce_records_call_with_data() {
        let mock = MockTransport::new_default();
        mock.announce(Some(b"app-data")).await;
        mock.announce(None).await;
        assert_eq!(mock.call_count(), 2);
        assert!(
            matches!(&mock.calls()[0], MockCall::Announce { app_data: Some(d) } if d == b"app-data")
        );
        assert!(matches!(
            &mock.calls()[1],
            MockCall::Announce { app_data: None }
        ));
    }

    #[tokio::test]
    async fn shutdown_records_call() {
        let mock = MockTransport::new_default();
        assert!(mock.shutdown().await.is_ok());
        assert_eq!(mock.call_count(), 1);
        assert!(matches!(&mock.calls()[0], MockCall::Shutdown));
    }

    #[tokio::test]
    async fn clear_calls_resets_recording() {
        let mock = MockTransport::new_default();
        mock.announce(None).await;
        assert_eq!(mock.call_count(), 1);
        mock.clear_calls();
        assert_eq!(mock.call_count(), 0);
    }

    #[tokio::test]
    async fn mock_as_dyn_mesh_transport() {
        let mock: std::sync::Arc<dyn MeshTransport> =
            std::sync::Arc::new(MockTransport::new_default());
        assert!(mock.is_connected());
        let result = mock.send_raw(AddressHash::new([5u8; 16]), b"test").await;
        assert!(matches!(result, Ok(SendPacketOutcome::SentDirect)));
    }
}
