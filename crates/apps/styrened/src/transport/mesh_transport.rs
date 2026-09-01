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
use rns_core::identity::PrivateIdentity;
use rns_core::transport::core_transport::{
    AnnounceEvent, ReceivedData, SendPacketOutcome, SendPacketTrace, path_table::RouteEvent,
};
use rns_core::transport::delivery::LinkSendResult;
use rns_core::transport::destination_ext::link::LinkCloseReason;
use rns_core::transport::iface::InterfaceStatsSnapshot;
use rns_core::transport::resource::ResourceEvent;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

pub fn request_observation_info(
    observation: rns_core::transport::request::RequestObservation,
) -> styrene_ipc::types::RequestObservationInfo {
    request_receipt_info(observation.receipt)
}

pub fn request_receipt_info(
    receipt: rns_core::transport::request::RequestReceipt,
) -> styrene_ipc::types::RequestObservationInfo {
    use rns_core::transport::request::{
        RequestProtocolError as RnsError, RequestStatus as RnsState, ResponseTransfer,
    };
    use styrene_ipc::types::{
        ObservationMetadata, ObservationSource, RequestProtocolError, RequestResponseTransfer,
        RequestState,
    };

    let request_id = hex::encode(receipt.request_id);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    let mut info = styrene_ipc::types::RequestObservationInfo::default();
    info.request_id = request_id.clone();
    info.path_hash = hex::encode(receipt.path_hash);
    info.link_id = hex::encode(receipt.link_id.as_slice());
    info.started_monotonic_ms = receipt.started_at.as_millis().try_into().unwrap_or(u64::MAX);
    info.deadline_monotonic_ms = receipt.deadline.as_millis().try_into().unwrap_or(u64::MAX);
    info.request_size = receipt.request_size.try_into().unwrap_or(u64::MAX);
    info.response_size = receipt.response_size.map(|size| size.try_into().unwrap_or(u64::MAX));
    info.response_transfer_size = receipt.response_transfer_size;
    info.received_bytes = receipt.received_bytes;
    info.total_bytes = receipt.total_bytes;
    info.progress = receipt.progress;
    info.response_transfer = match receipt.response_transfer {
        ResponseTransfer::None => RequestResponseTransfer::None,
        ResponseTransfer::Packet => RequestResponseTransfer::Packet,
        ResponseTransfer::Resource { .. } => RequestResponseTransfer::Resource,
    };
    info.response = receipt.response;
    info.state = match receipt.status {
        RnsState::Pending => RequestState::Pending,
        RnsState::Receiving => RequestState::Receiving,
        RnsState::Succeeded => RequestState::Succeeded,
        RnsState::LinkClosed => RequestState::LinkClosed,
        RnsState::TimedOut => RequestState::TimedOut,
        RnsState::MalformedResponse => RequestState::MalformedResponse,
        RnsState::Cancelled => RequestState::Cancelled,
        RnsState::ResponseTooLarge => RequestState::ResponseTooLarge,
        RnsState::ResourceFailed => RequestState::ResourceFailed,
        RnsState::TransportFailed => RequestState::TransportFailed,
    };
    info.protocol_error = receipt.protocol_error.map(|error| match error {
        RnsError::LinkClosed => RequestProtocolError::LinkClosed,
        RnsError::Timeout => RequestProtocolError::Timeout,
        RnsError::MalformedResponse => RequestProtocolError::MalformedResponse,
        RnsError::Cancelled => RequestProtocolError::Cancelled,
        RnsError::ResponseTooLarge => RequestProtocolError::ResponseTooLarge,
        RnsError::ResourceFailed => RequestProtocolError::ResourceFailed,
        RnsError::TransportFailed => RequestProtocolError::TransportFailed,
    });
    info.completed_monotonic_ms =
        receipt.completed_at.map(|time| time.as_millis().try_into().unwrap_or(u64::MAX));
    info.rtt_ms = receipt.rtt.map(|time| time.as_millis().try_into().unwrap_or(u64::MAX));
    info.request_resource_hash =
        receipt.request_resource_hash.map(|hash| hex::encode(hash.as_slice()));
    info.resource_hash = match receipt.response_transfer {
        ResponseTransfer::Resource { hash } => Some(hex::encode(hash.as_slice())),
        _ => None,
    };
    info.observation =
        ObservationMetadata::at(ObservationSource::TransportRequestState, Some(now), now, 300);
    info.observation.correlation_id = receipt.correlation_id.or(Some(request_id));
    info
}

#[derive(Debug, Clone, PartialEq)]
pub enum RequestLifecycleEvent {
    Observation(Box<styrene_ipc::types::RequestObservationInfo>),
    ReconcileRequired { dropped: u64 },
}

/// Transport lifecycle events — services subscribe to react to connectivity changes.
#[derive(Debug, Clone, PartialEq)]
pub enum TransportLifecycleEvent {
    Connected,
    Disconnected,
    Reconnected,
    /// An interface changed state; consumers must query authoritative interface snapshots.
    InterfaceChanged,
    /// The bounded interface-state stream lagged; consumers must query authoritative state.
    InterfaceReconcileRequired,
    /// A lower-level lifecycle receiver lagged; consumers must query authoritative state.
    LinkReconcileRequired,
    /// An outbound link became active (proof received, RTT measured).
    LinkActivated {
        /// Short hex link ID (16 chars).
        link_id: String,
        /// Destination peer hash (32 chars).
        peer_hash: String,
        interface: Option<String>,
        /// RTT in milliseconds if already measured, else 0.0.
        rtt_ms: f64,
    },
    LinkIdentified {
        link_id: String,
        peer_hash: String,
        interface: Option<String>,
        rtt_ms: Option<f64>,
        remote_identity_hash: String,
    },
    LinkActivity {
        link_id: String,
        peer_hash: String,
        interface: Option<String>,
        rtt_ms: Option<f64>,
    },
    /// A link closed (stale timeout or explicit close).
    LinkClosed {
        link_id: String,
        peer_hash: String,
        interface: Option<String>,
        rtt_ms: Option<f64>,
        reason: LinkCloseReason,
    },
    /// Link RTT updated (from an RTT probe response).
    LinkRttUpdated {
        link_id: String,
        peer_hash: String,
        interface: Option<String>,
        rtt_ms: f64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkOpenResult {
    Created(AddressHash),
    Reused(AddressHash),
}

impl LinkOpenResult {
    pub fn link_id(self) -> AddressHash {
        match self {
            Self::Created(link_id) | Self::Reused(link_id) => link_id,
        }
    }

    pub fn is_created(self) -> bool {
        matches!(self, Self::Created(_))
    }
}

/// Errors from transport operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum TransportError {
    #[error("transport unavailable")]
    Unavailable,
    #[error("send failed: {0}")]
    SendFailed(String),
    #[error("link failed: {0}")]
    LinkFailed(String),
    #[error("operation cancelled after transport cleanup")]
    Cancelled,
    #[error("operation deadline expired after transport cleanup")]
    TimedOut,
    #[error("transport cleanup failed: {0}")]
    CleanupFailed(String),
    #[error("shutdown failed: {0}")]
    ShutdownFailed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkRepresentation {
    Packet,
    Resource,
}

pub type DispatchGate = Arc<dyn Fn(LinkRepresentation) -> Result<(), TransportError> + Send + Sync>;

pub fn validate_link_representation(
    representation: LinkRepresentation,
    encoded_size: usize,
) -> Result<(), TransportError> {
    let packet_sized = encoded_size <= rns_core::transport::resource::LINK_PACKET_MDU;
    if matches!(
        (representation, packet_sized),
        (LinkRepresentation::Packet, true) | (LinkRepresentation::Resource, false)
    ) {
        Ok(())
    } else {
        Err(TransportError::SendFailed(format!(
            "selected {representation:?} representation is invalid for {encoded_size} encoded bytes"
        )))
    }
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
    ///
    /// Sends raw bytes as a SINGLE packet to the destination. The caller is
    /// responsible for LXMF wire format details (e.g., stripping the
    /// destination prefix for opportunistic delivery). This is a transport
    /// primitive, not a message delivery method.
    async fn send_raw(
        &self,
        dest: AddressHash,
        data: &[u8],
    ) -> Result<SendPacketOutcome, TransportError>;

    /// Opportunistic send that also reports the transmitted packet hash.
    ///
    /// Reticulum delivery proofs identify the proved packet by this hash, so
    /// callers that want delivery evidence must correlate on it. The default
    /// preserves `send_raw` behavior without a hash.
    async fn send_raw_traced(
        &self,
        dest: AddressHash,
        data: &[u8],
    ) -> Result<SendPacketTrace, TransportError> {
        let outcome = self.send_raw(dest, data).await?;
        Ok(SendPacketTrace {
            outcome,
            direct_iface: None,
            broadcast: false,
            dispatch: rns_core::transport::iface::TxDispatchTrace::default(),
            packet_hash: None,
        })
    }

    /// Link-based reliable send (with resource fallback for large payloads).
    /// Caller must provide a fully-resolved `DestinationDesc` (includes peer Identity).
    async fn send_via_link(
        &self,
        dest: DestinationDesc,
        data: &[u8],
        timeout: Duration,
    ) -> Result<LinkSendResult, TransportError>;

    /// Link send with a representation selected by the authoritative caller.
    /// Validation occurs before the underlying transport is invoked.
    async fn send_via_link_selected(
        &self,
        dest: DestinationDesc,
        data: &[u8],
        timeout: Duration,
        representation: LinkRepresentation,
    ) -> Result<LinkSendResult, TransportError> {
        validate_link_representation(representation, data.len())?;
        self.send_via_link(dest, data, timeout).await
    }

    /// Link send whose caller owns cancellation until `dispatch_gate` succeeds.
    async fn send_via_link_selected_cancellable(
        &self,
        dest: DestinationDesc,
        data: &[u8],
        timeout: Duration,
        representation: LinkRepresentation,
        cancellation: CancellationToken,
        dispatch_gate: DispatchGate,
    ) -> Result<LinkSendResult, TransportError> {
        validate_link_representation(representation, data.len())?;
        if cancellation.is_cancelled() {
            return Err(TransportError::Cancelled);
        }
        dispatch_gate(representation)?;
        self.send_via_link_selected(dest, data, timeout, representation).await
    }

    /// Cancel an outbound resource when the backend still owns it.
    async fn cancel_resource(&self, _hash: rns_core::hash::Hash) -> Result<bool, TransportError> {
        Ok(false)
    }

    async fn start_request(
        &self,
        _request: styrene_ipc::types::StartRequestInfo,
    ) -> Result<styrene_ipc::types::RequestObservationInfo, TransportError> {
        Err(TransportError::Unavailable)
    }

    async fn request_receipt(
        &self,
        _request_id: &str,
    ) -> Result<Option<styrene_ipc::types::RequestObservationInfo>, TransportError> {
        Err(TransportError::Unavailable)
    }

    async fn request_receipts(
        &self,
    ) -> Result<Vec<styrene_ipc::types::RequestObservationInfo>, TransportError> {
        Err(TransportError::Unavailable)
    }

    async fn cancel_request(
        &self,
        _request_id: &str,
    ) -> Result<styrene_ipc::types::RequestObservationInfo, TransportError> {
        Err(TransportError::Unavailable)
    }

    async fn cancel_requests_by_correlation(
        &self,
        _correlation_id: &str,
    ) -> Result<usize, TransportError> {
        Err(TransportError::Unavailable)
    }

    // --- Discovery ---

    /// Trigger path request for a destination.
    async fn request_path(&self, dest: &AddressHash);

    /// Look up peer identity from transport's announce table.
    /// Returns `None` if identity not yet known (peer hasn't announced).
    async fn resolve_identity(&self, dest: &AddressHash) -> Option<Identity>;

    /// Establish an active native RNS link and return its link ID.
    async fn open_native_nomadnet_link(
        &self,
        _dest: DestinationDesc,
        _cancellation: tokio_util::sync::CancellationToken,
        _timeout: Duration,
    ) -> Result<LinkOpenResult, TransportError> {
        Err(TransportError::Unavailable)
    }

    async fn identify_native_nomadnet_link(
        &self,
        _link_id: &str,
        _identity: &PrivateIdentity,
    ) -> Result<(), TransportError> {
        Err(TransportError::Unavailable)
    }

    /// Establish a link to an exact named destination. Protocol coordinators
    /// retain ownership information through `LinkOpenResult`.
    async fn open_named_link(
        &self,
        destination: DestinationDesc,
        cancellation: CancellationToken,
        timeout: Duration,
    ) -> Result<LinkOpenResult, TransportError> {
        self.open_native_nomadnet_link(destination, cancellation, timeout).await
    }

    /// Identify the local private identity on an existing active link.
    async fn identify_link(
        &self,
        link_id: &str,
        identity: &PrivateIdentity,
    ) -> Result<(), TransportError> {
        self.identify_native_nomadnet_link(link_id, identity).await
    }

    /// Send packet/resource data on the exact already-authenticated link.
    async fn send_on_link(
        &self,
        _link_id: &AddressHash,
        _data: &[u8],
    ) -> Result<rns_core::transport::delivery::LinkSendResult, TransportError> {
        Err(TransportError::Unavailable)
    }

    // --- Announcing ---

    /// Send announce with optional app_data.
    async fn announce(&self, app_data: Option<&[u8]>);

    /// Dispatch an announce and report only local transport acceptance.
    async fn dispatch_announce(&self, _app_data: Option<&[u8]>) -> Result<(), TransportError> {
        Err(TransportError::Unavailable)
    }

    /// Open an RNS link to an announced destination and return its stable link ID.
    async fn open_link(
        &self,
        _dest: &AddressHash,
        _cancellation: tokio_util::sync::CancellationToken,
        _timeout: Duration,
    ) -> Result<LinkOpenResult, TransportError> {
        Err(TransportError::Unavailable)
    }

    /// Abort local pending establishment after the link ID is known.
    async fn cancel_link_open(&self, _link_id: &AddressHash) -> Result<(), TransportError> {
        Err(TransportError::Unavailable)
    }

    /// Emit the existing RNS RTT signal on an active link.
    async fn probe_link(&self, _link_id: &AddressHash) -> Result<(), TransportError> {
        Err(TransportError::Unavailable)
    }

    /// Close an active local RNS link.
    async fn close_link(&self, _link_id: &AddressHash) -> Result<(), TransportError> {
        Err(TransportError::Unavailable)
    }

    // --- Subscriptions (broadcast channels) ---

    /// Subscribe to inbound data events (decoded payloads delivered to our destination).
    fn subscribe_inbound(&self) -> broadcast::Receiver<ReceivedData>;

    /// Subscribe to announce events from other nodes.
    fn subscribe_announces(&self) -> broadcast::Receiver<AnnounceEvent>;

    /// Subscribe to transport lifecycle transitions (connected/disconnected/reconnected).
    fn subscribe_lifecycle(&self) -> broadcast::Receiver<TransportLifecycleEvent>;

    /// Subscribe to resource transfer events (completed resource reassembly).
    /// Large payloads (> LINK_PACKET_MDU) are sent as resources — these events
    /// carry the reassembled data after all chunks arrive.
    fn subscribe_resources(&self) -> broadcast::Receiver<ResourceEvent>;

    /// Subscribe to authenticated delivery proofs keyed by exact packet hash.
    fn subscribe_packet_receipts(&self) -> broadcast::Receiver<[u8; 32]>;

    /// Subscribe to authoritative route discovery, loss, and rediscovery transitions.
    fn subscribe_routes(&self) -> broadcast::Receiver<RouteEvent>;

    fn subscribe_request_observations(&self) -> broadcast::Receiver<RequestLifecycleEvent>;

    // --- State queries ---

    /// Query path table for hop count and next-hop interface.
    /// Returns `None` if no path is known for the destination.
    async fn query_path(&self, dest: &AddressHash) -> Option<(u8, AddressHash)>;

    /// Dump the entire path table: (destination, hops, received_from, interface).
    async fn path_table(&self) -> Vec<(AddressHash, u8, AddressHash, AddressHash)> {
        Vec::new()
    }

    async fn query_path_snapshot(
        &self,
        _dest: &AddressHash,
    ) -> Option<rns_core::transport::core_transport::path_table::PathSnapshot> {
        None
    }

    async fn path_snapshots(
        &self,
    ) -> Vec<rns_core::transport::core_transport::path_table::PathSnapshot> {
        Vec::new()
    }

    /// Authoritative active link state from the transport's link tables.
    async fn link_lifecycle_snapshot(
        &self,
    ) -> rns_core::transport::destination_ext::link::LinkLifecycleSnapshot {
        Default::default()
    }

    /// Our identity address hash.
    fn identity_hash(&self) -> AddressHash;

    /// Our delivery destination hash.
    fn destination_hash(&self) -> AddressHash;

    /// Runtime-owned identity and delivery destination, when this backend has one.
    fn runtime_identity(&self) -> Option<(AddressHash, AddressHash)>;

    /// Whether transport is currently connected/operational.
    fn is_connected(&self) -> bool;

    // --- Lifecycle ---

    /// Shut down the transport gracefully.
    async fn shutdown(&self) -> Result<(), TransportError>;

    /// Per-interface byte counter snapshots (tx_bytes, rx_bytes).
    /// Keys are interface address hashes. Returns an empty map when the
    /// transport backend does not track per-interface stats.
    async fn interface_stats(&self) -> HashMap<AddressHash, InterfaceStatsSnapshot> {
        HashMap::new()
    }

    /// Authoritative runtime interface observations.
    async fn interface_snapshots(&self) -> Vec<rns_core::transport::iface::InterfaceSnapshot>;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rns_core::transport::request::{RequestClock, RequestTracker};

    use super::*;

    struct FixedClock;

    impl RequestClock for FixedClock {
        fn now(&self) -> Duration {
            Duration::from_millis(25)
        }
    }

    #[test]
    fn request_projection_preserves_correlation_sizes_and_monotonic_deadline() {
        let mut tracker = RequestTracker::new(1, Arc::new(FixedClock));
        let mut events = tracker.subscribe();
        tracker
            .start_correlated(
                [1; 16],
                [2; 16],
                AddressHash::new([3; 16]),
                31,
                rns_core::transport::request::RequestOptions {
                    timeout: Duration::from_secs(5),
                    max_response_size: 64,
                    correlation_id: Some("page-operation".into()),
                },
            )
            .expect("request receipt");
        let projected = request_observation_info(events.try_recv().expect("request observation"));

        assert_eq!(projected.request_id, "01".repeat(16));
        assert_eq!(projected.path_hash, "02".repeat(16));
        assert_eq!(projected.link_id, "03".repeat(16));
        assert_eq!(projected.started_monotonic_ms, 25);
        assert_eq!(projected.deadline_monotonic_ms, 5_025);
        assert_eq!(projected.request_size, 31);
        assert_eq!(projected.observation.correlation_id.as_deref(), Some("page-operation"));
    }

    #[test]
    fn request_projection_preserves_outbound_resource_hash() {
        let mut tracker = RequestTracker::new(1, Arc::new(FixedClock));
        tracker
            .start([1; 16], [2; 16], AddressHash::new([3; 16]), 512, Duration::from_secs(5), 64)
            .expect("request receipt");
        let hash = rns_core::hash::Hash::new([4; 32]);
        assert!(tracker.set_request_resource([1; 16], hash));

        let projected = request_receipt_info(tracker.get(&[1; 16]).expect("receipt").clone());

        assert_eq!(projected.request_resource_hash, Some("04".repeat(32)));
        assert_eq!(projected.resource_hash, None);
    }
}
