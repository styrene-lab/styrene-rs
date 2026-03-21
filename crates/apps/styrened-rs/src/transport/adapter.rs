//! TokioTransportAdapter — wraps `rns_core::transport::core_transport::Transport`
//! to implement the `MeshTransport` trait.
//!
//! This is the production implementation. It delegates all operations to the
//! real RNS transport layer.

use super::mesh_transport::{MeshTransport, TransportError, TransportLifecycleEvent};
use rns_core::destination::{DestinationDesc, SingleInputDestination};
use rns_core::hash::AddressHash;
use rns_core::identity::Identity;
use rns_core::packet::{
    ContextFlag, DestinationType, Header, HeaderType, IfacFlag, Packet, PacketContext,
    PacketDataBuffer, PacketType, PropagationType,
};
use rns_core::transport::core_transport::{
    AnnounceEvent, ReceivedData, SendPacketOutcome, Transport,
};
use rns_core::transport::delivery::LinkSendResult;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

/// Production transport adapter wrapping `rns_core::Transport`.
pub struct TokioTransportAdapter {
    transport: Arc<Transport>,
    identity_addr: AddressHash,
    destination_addr: AddressHash,
    announce_destination: Arc<tokio::sync::Mutex<SingleInputDestination>>,
    announce_app_data: Option<Vec<u8>>,
    lifecycle_tx: broadcast::Sender<TransportLifecycleEvent>,
}

impl TokioTransportAdapter {
    /// Create a new adapter wrapping the given transport.
    ///
    /// - `transport`: the live RNS transport instance
    /// - `identity_addr`: our identity address hash
    /// - `destination_addr`: our delivery destination hash
    /// - `announce_destination`: the LXMF delivery destination for announcing
    /// - `announce_app_data`: optional app_data bytes for announces
    pub fn new(
        transport: Arc<Transport>,
        identity_addr: AddressHash,
        destination_addr: AddressHash,
        announce_destination: Arc<tokio::sync::Mutex<SingleInputDestination>>,
        announce_app_data: Option<Vec<u8>>,
    ) -> Self {
        let (lifecycle_tx, _) = broadcast::channel(16);
        Self {
            transport,
            identity_addr,
            destination_addr,
            announce_destination,
            announce_app_data,
            lifecycle_tx,
        }
    }

    /// Emit a lifecycle event to all subscribers.
    pub fn emit_lifecycle(&self, event: TransportLifecycleEvent) {
        // Ignore send errors (no subscribers is fine)
        let _ = self.lifecycle_tx.send(event);
    }
}

#[async_trait::async_trait]
impl MeshTransport for TokioTransportAdapter {
    async fn send_raw(
        &self,
        dest: AddressHash,
        data: &[u8],
    ) -> Result<SendPacketOutcome, TransportError> {
        let mut packet_data = PacketDataBuffer::new();
        packet_data
            .write(data)
            .map_err(|e| TransportError::SendFailed(format!("payload too large: {e:?}")))?;

        let packet = Packet {
            header: Header {
                ifac_flag: IfacFlag::Open,
                header_type: HeaderType::Type1,
                context_flag: ContextFlag::Unset,
                propagation_type: PropagationType::Broadcast,
                destination_type: DestinationType::Single,
                packet_type: PacketType::Data,
                hops: 0,
            },
            ifac: None,
            destination: dest,
            transport: None,
            context: PacketContext::None,
            data: packet_data,
        };

        let outcome = self.transport.send_packet_with_outcome(packet).await;
        Ok(outcome)
    }

    async fn send_via_link(
        &self,
        dest: DestinationDesc,
        data: &[u8],
        timeout: Duration,
    ) -> Result<LinkSendResult, TransportError> {
        rns_core::transport::delivery::send_via_link(&self.transport, dest, data, timeout)
            .await
            .map_err(|e| TransportError::LinkFailed(e.to_string()))
    }

    async fn request_path(&self, dest: &AddressHash) {
        self.transport.request_path(dest, None, None).await;
    }

    async fn resolve_identity(&self, dest: &AddressHash) -> Option<Identity> {
        self.transport.destination_identity(dest).await
    }

    async fn announce(&self, app_data: Option<&[u8]>) {
        let data = app_data
            .map(|d| d.to_vec())
            .or_else(|| self.announce_app_data.clone());
        self.transport
            .send_announce(&self.announce_destination, data.as_deref())
            .await;
    }

    fn subscribe_inbound(&self) -> broadcast::Receiver<ReceivedData> {
        self.transport.received_data_events()
    }

    fn subscribe_announces(&self) -> broadcast::Receiver<AnnounceEvent> {
        // Note: Transport::recv_announces() is async, but we need to return
        // synchronously. We use a blocking approach via tokio::task::block_in_place
        // which is acceptable since this is called during initialization, not in
        // hot paths.
        //
        // Alternative: store the receiver during construction. For now, we create
        // a new subscription each time (broadcast supports multiple subscribers).
        let transport = self.transport.clone();
        // This works because broadcast::Sender::subscribe() is sync under the hood;
        // recv_announces() wraps it in an async fn. We can replicate the subscription
        // directly via the transport's internal sender.
        //
        // SAFETY: This relies on the announce channel being already initialized,
        // which is guaranteed after Transport::new().
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(transport.recv_announces())
        })
    }

    fn subscribe_lifecycle(&self) -> broadcast::Receiver<TransportLifecycleEvent> {
        self.lifecycle_tx.subscribe()
    }

    fn identity_hash(&self) -> AddressHash {
        self.identity_addr
    }

    fn destination_hash(&self) -> AddressHash {
        self.destination_addr
    }

    fn is_connected(&self) -> bool {
        true // Transport object existence implies connectivity
    }

    async fn shutdown(&self) -> Result<(), TransportError> {
        // Graceful shutdown will be refined in later packages.
        // For now, emit the lifecycle event.
        self.emit_lifecycle(TransportLifecycleEvent::Disconnected);
        Ok(())
    }
}
