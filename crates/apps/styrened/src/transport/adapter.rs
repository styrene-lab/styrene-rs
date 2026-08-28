//! TokioTransportAdapter — wraps `rns_core::transport::core_transport::Transport`
//! to implement the `MeshTransport` trait.
//!
//! This is the production implementation. It delegates all operations to the
//! real RNS transport layer.

use super::mesh_transport::{
    LinkRepresentation, MeshTransport, RequestLifecycleEvent, TransportError,
    TransportLifecycleEvent, validate_link_representation,
};
use rns_core::destination::{DestinationDesc, DestinationName, SingleInputDestination};
use rns_core::hash::AddressHash;
use rns_core::identity::Identity;
use rns_core::packet::{
    ContextFlag, DestinationType, Header, HeaderType, IfacFlag, Packet, PacketContext,
    PacketDataBuffer, PacketType, PropagationType,
};
use rns_core::transport::core_transport::{
    AnnounceEvent, ReceivedData, SendPacketOutcome, Transport, path_table::RouteEvent,
};
use rns_core::transport::delivery::LinkSendResult;
use rns_core::transport::destination_ext::link::{LinkEvent, LinkEventData};
use rns_core::transport::iface::InterfaceStatsSnapshot;
use rns_core::transport::resource::ResourceEvent;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Production transport adapter wrapping `rns_core::Transport`.
pub struct TokioTransportAdapter {
    transport: Arc<Transport>,
    identity_addr: AddressHash,
    destination_addr: AddressHash,
    announce_destination: Arc<tokio::sync::Mutex<SingleInputDestination>>,
    announce_app_data: Option<Vec<u8>>,
    lifecycle_tx: broadcast::Sender<TransportLifecycleEvent>,
    /// Cached announce sender — subscribing is sync via sender.subscribe().
    announce_tx: broadcast::Sender<AnnounceEvent>,
    route_tx: broadcast::Sender<RouteEvent>,
    request_tx: broadcast::Sender<RequestLifecycleEvent>,
    packet_receipt_tx: broadcast::Sender<[u8; 32]>,
    forwarder_cancel: CancellationToken,
    interface_cancel: CancellationToken,
    forwarders: tokio::sync::Mutex<Option<Vec<JoinHandle<()>>>>,
    shutdown_started: AtomicBool,
}

impl Drop for TokioTransportAdapter {
    fn drop(&mut self) {
        self.forwarder_cancel.cancel();
        self.interface_cancel.cancel();
        if let Ok(forwarders) = self.forwarders.try_lock()
            && let Some(forwarders) = forwarders.as_ref()
        {
            for forwarder in forwarders {
                forwarder.abort();
            }
        }
        let interface_manager = self.transport.iface_manager();
        if let Ok(manager) = interface_manager.try_lock() {
            manager.shutdown();
            manager.abort_tasks();
        }
        self.transport.abort_manager();
    }
}

async fn close_unclaimed_native_link(
    transport: &Transport,
    link_id: AddressHash,
) -> Result<(), TransportError> {
    const ATTEMPTS: u8 = 3;
    for attempt in 1..=ATTEMPTS {
        if transport.close_link(&link_id).await {
            return Ok(());
        }
        if transport.find_out_link(&link_id).await.is_none()
            && transport.find_in_link(&link_id).await.is_none()
        {
            return Ok(());
        }
        if attempt < ATTEMPTS {
            log::warn!(
                "unclaimed native link cleanup attempt {attempt} failed for {}",
                hex::encode(link_id.as_slice())
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    Err(TransportError::CleanupFailed(format!(
        "unclaimed native link {} remained registered after {ATTEMPTS} attempts",
        hex::encode(link_id.as_slice())
    )))
}

async fn open_native_link_owned(
    transport: Arc<Transport>,
    dest: DestinationDesc,
    cancellation: tokio_util::sync::CancellationToken,
    timeout: Duration,
) -> Result<super::mesh_transport::LinkOpenResult, TransportError> {
    use super::mesh_transport::LinkOpenResult;
    use rns_core::transport::core_transport::LinkDispatch;
    use rns_core::transport::destination_ext::link::LinkStatus;

    let child_cancellation = tokio_util::sync::CancellationToken::new();
    let dispatch = transport.link_cancellable(dest, child_cancellation.clone());
    tokio::pin!(dispatch);
    let dispatched = tokio::select! {
        biased;
        _ = cancellation.cancelled() => {
            child_cancellation.cancel();
            let _ = dispatch.await;
            return Err(TransportError::Cancelled);
        }
        result = &mut dispatch => result,
    }
    .ok_or_else(|| TransportError::LinkFailed("link dispatch was not accepted".into()))?;
    let (link, owned) = match dispatched {
        LinkDispatch::Created(link) => (link, true),
        LinkDispatch::Reused(link) => (link, false),
    };
    let mut owned_link_id = if owned { Some(*link.lock().await.id()) } else { None };
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if cancellation.is_cancelled() {
            if let Some(link_id) = owned_link_id.take()
                && !transport.cancel_link_open(&link_id).await
            {
                return Err(TransportError::CleanupFailed(
                    "pending native link remained registered".into(),
                ));
            }
            return Err(TransportError::Cancelled);
        }
        let guard = link.lock().await;
        let id = *guard.id();
        match guard.status() {
            LinkStatus::Active => {
                return Ok(if owned {
                    LinkOpenResult::Created(id)
                } else {
                    LinkOpenResult::Reused(id)
                });
            }
            LinkStatus::Closed => {
                drop(guard);
                if let Some(link_id) = owned_link_id.take() {
                    let _ = transport.cancel_link_open(&link_id).await;
                }
                return Err(TransportError::LinkFailed("link closed before activation".into()));
            }
            _ => {}
        }
        drop(guard);
        let timed_out = tokio::time::Instant::now() >= deadline;
        if cancellation.is_cancelled() || timed_out {
            if let Some(link_id) = owned_link_id.take()
                && !transport.cancel_link_open(&link_id).await
            {
                return Err(TransportError::CleanupFailed(
                    "pending native link remained registered".into(),
                ));
            }
            return if timed_out {
                Err(TransportError::TimedOut)
            } else {
                Err(TransportError::Cancelled)
            };
        }
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => {}
            _ = tokio::time::sleep(Duration::from_millis(10)) => {}
        }
    }
}

fn lifecycle_from_link_event(ev: LinkEventData) -> Option<TransportLifecycleEvent> {
    let link_id = hex::encode(ev.id.as_slice());
    let peer_hash = hex::encode(ev.address_hash.as_slice());
    let interface = ev.interface.map(|value| hex::encode(value.as_slice()));
    let rtt_ms = ev.rtt.map(|value| value.as_secs_f64() * 1000.0);
    match ev.event {
        LinkEvent::Activated => Some(TransportLifecycleEvent::LinkActivated {
            link_id,
            peer_hash,
            interface,
            rtt_ms: rtt_ms.unwrap_or(0.0),
        }),
        LinkEvent::Identified => {
            ev.remote_identity.map(|identity| TransportLifecycleEvent::LinkIdentified {
                link_id,
                peer_hash,
                interface,
                rtt_ms,
                remote_identity_hash: hex::encode(identity.as_slice()),
            })
        }
        LinkEvent::Activity => {
            Some(TransportLifecycleEvent::LinkActivity { link_id, peer_hash, interface, rtt_ms })
        }
        LinkEvent::RttUpdated => Some(TransportLifecycleEvent::LinkRttUpdated {
            link_id,
            peer_hash,
            interface,
            rtt_ms: rtt_ms.unwrap_or(0.0),
        }),
        LinkEvent::Closed(reason) => Some(TransportLifecycleEvent::LinkClosed {
            link_id,
            peer_hash,
            interface,
            rtt_ms,
            reason,
        }),
        LinkEvent::Data(_) => None,
    }
}

fn request_lifecycle_from_receive(
    result: Result<rns_core::transport::request::RequestObservation, broadcast::error::RecvError>,
) -> Result<RequestLifecycleEvent, ()> {
    match result {
        Ok(observation) => Ok(RequestLifecycleEvent::Observation(Box::new(
            super::mesh_transport::request_observation_info(observation),
        ))),
        Err(broadcast::error::RecvError::Lagged(dropped)) => {
            Ok(RequestLifecycleEvent::ReconcileRequired { dropped })
        }
        Err(broadcast::error::RecvError::Closed) => Err(()),
    }
}

impl TokioTransportAdapter {
    /// Create a new adapter wrapping the given transport.
    ///
    /// - `transport`: the live RNS transport instance
    /// - `identity_addr`: our identity address hash
    /// - `destination_addr`: our delivery destination hash
    /// - `announce_destination`: the LXMF delivery destination for announcing
    /// - `announce_app_data`: optional app_data bytes for announces
    ///
    /// This is an async constructor because it needs to subscribe to the
    /// transport's announce channel (which requires the handler lock).
    pub async fn new(
        transport: Arc<Transport>,
        identity_addr: AddressHash,
        destination_addr: AddressHash,
        announce_destination: Arc<tokio::sync::Mutex<SingleInputDestination>>,
        announce_app_data: Option<Vec<u8>>,
    ) -> Self {
        let (packet_receipt_tx, _) = broadcast::channel(1);
        Self::new_with_packet_receipts(
            transport,
            identity_addr,
            destination_addr,
            announce_destination,
            announce_app_data,
            packet_receipt_tx,
        )
        .await
    }

    pub async fn new_with_packet_receipts(
        transport: Arc<Transport>,
        identity_addr: AddressHash,
        destination_addr: AddressHash,
        announce_destination: Arc<tokio::sync::Mutex<SingleInputDestination>>,
        announce_app_data: Option<Vec<u8>>,
        packet_receipt_tx: broadcast::Sender<[u8; 32]>,
    ) -> Self {
        let (lifecycle_tx, _) = broadcast::channel(16);
        // Forward transport announces through our own broadcast sender so
        // subscribe_announces() can return synchronously (no async lock needed).
        let (our_announce_tx, _) = broadcast::channel(64);
        let (our_route_tx, _) = broadcast::channel(64);
        let (our_request_tx, _) = broadcast::channel(64);
        let forwarder_cancel = CancellationToken::new();
        let interface_cancel = transport.iface_manager().lock().await.shutdown_token();
        let mut forwarders = Vec::with_capacity(5);
        let fwd_tx = our_announce_tx.clone();
        let mut rx = transport.recv_announces().await;
        let cancel = forwarder_cancel.clone();
        forwarders.push(tokio::spawn(async move {
            loop {
                let result = tokio::select! {
                    _ = cancel.cancelled() => break,
                    result = rx.recv() => result,
                };
                match result {
                    Ok(event) => {
                        let _ = fwd_tx.send(event);
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                }
            }
        }));
        let request_fwd_tx = our_request_tx.clone();
        let mut request_rx = transport.request_events().await;
        let cancel = forwarder_cancel.clone();
        forwarders.push(tokio::spawn(async move {
            loop {
                let result = tokio::select! {
                    _ = cancel.cancelled() => break,
                    result = request_rx.recv() => result,
                };
                let Ok(event) = request_lifecycle_from_receive(result) else { break };
                let _ = request_fwd_tx.send(event);
            }
        }));
        let route_fwd_tx = our_route_tx.clone();
        let mut rx = transport.route_events().await;
        let cancel = forwarder_cancel.clone();
        forwarders.push(tokio::spawn(async move {
            loop {
                let result = tokio::select! {
                    _ = cancel.cancelled() => break,
                    result = rx.recv() => result,
                };
                match result {
                    Ok(event) => {
                        let _ = route_fwd_tx.send(event);
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                }
            }
        }));
        // Forward both link directions without exposing raw transport handles.
        for mut rx in [transport.out_link_events(), transport.in_link_events()] {
            let lifecycle_fwd = lifecycle_tx.clone();
            let cancel = forwarder_cancel.clone();
            forwarders.push(tokio::spawn(async move {
                loop {
                    let result = tokio::select! {
                        _ = cancel.cancelled() => break,
                        result = rx.recv() => result,
                    };
                    match result {
                        Ok(event) => {
                            if let Some(event) = lifecycle_from_link_event(event) {
                                let _ = lifecycle_fwd.send(event);
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            let _ =
                                lifecycle_fwd.send(TransportLifecycleEvent::LinkReconcileRequired);
                        }
                    }
                }
            }));
        }

        Self {
            transport,
            identity_addr,
            destination_addr,
            announce_destination,
            announce_app_data,
            lifecycle_tx,
            announce_tx: our_announce_tx,
            route_tx: our_route_tx,
            request_tx: our_request_tx,
            packet_receipt_tx,
            forwarder_cancel,
            interface_cancel,
            forwarders: tokio::sync::Mutex::new(Some(forwarders)),
            shutdown_started: AtomicBool::new(false),
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
    async fn start_request(
        &self,
        request: styrene_ipc::types::StartRequestInfo,
    ) -> Result<styrene_ipc::types::RequestObservationInfo, TransportError> {
        let link_bytes = hex::decode(&request.link_id)
            .map_err(|_| TransportError::SendFailed("invalid request link ID".into()))?;
        let link_id: [u8; 16] = link_bytes
            .try_into()
            .map_err(|_| TransportError::SendFailed("request link ID must be 16 bytes".into()))?;
        let maximum = usize::try_from(request.max_response_size)
            .map_err(|_| TransportError::SendFailed("maximum response size is too large".into()))?;
        if request.timeout_ms == 0 || maximum == 0 || !request.path.starts_with('/') {
            return Err(TransportError::SendFailed("invalid request limits or path".into()));
        }
        let mut cursor = std::io::Cursor::new(request.data.as_slice());
        if rmpv::decode::read_value(&mut cursor).is_err()
            || usize::try_from(cursor.position()).ok() != Some(request.data.len())
        {
            return Err(TransportError::SendFailed(
                "request data must contain exactly one MessagePack value".into(),
            ));
        }
        let receipt = self
            .transport
            .request_over_link(
                &AddressHash::new(link_id),
                rns_core::destination::request_path_hash(&request.path),
                &request.data,
                Duration::from_millis(request.timeout_ms),
                maximum,
                request.correlation_id.clone(),
            )
            .await
            .map_err(|error| TransportError::SendFailed(format!("{error:?}")))?;
        Ok(super::mesh_transport::request_receipt_info(receipt))
    }

    async fn request_receipt(
        &self,
        request_id: &str,
    ) -> Result<Option<styrene_ipc::types::RequestObservationInfo>, TransportError> {
        let request_id = parse_request_id(request_id)?;
        Ok(self
            .transport
            .request_receipt(&request_id)
            .await
            .map(super::mesh_transport::request_receipt_info))
    }

    async fn request_receipts(
        &self,
    ) -> Result<Vec<styrene_ipc::types::RequestObservationInfo>, TransportError> {
        Ok(self
            .transport
            .request_receipts()
            .await
            .into_iter()
            .map(super::mesh_transport::request_receipt_info)
            .collect())
    }

    async fn cancel_request(
        &self,
        request_id: &str,
    ) -> Result<styrene_ipc::types::RequestObservationInfo, TransportError> {
        let request_id = parse_request_id(request_id)?;
        if !self.transport.cancel_request(request_id).await {
            return Err(TransportError::SendFailed("request is not cancellable".into()));
        }
        self.transport
            .request_receipt(&request_id)
            .await
            .map(super::mesh_transport::request_receipt_info)
            .ok_or_else(|| TransportError::SendFailed("request receipt disappeared".into()))
    }

    async fn cancel_requests_by_correlation(
        &self,
        correlation_id: &str,
    ) -> Result<usize, TransportError> {
        Ok(self.transport.cancel_requests_by_correlation(correlation_id).await)
    }

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

    async fn send_via_link_selected(
        &self,
        dest: DestinationDesc,
        data: &[u8],
        timeout: Duration,
        representation: LinkRepresentation,
    ) -> Result<LinkSendResult, TransportError> {
        validate_link_representation(representation, data.len())?;
        rns_core::transport::delivery::send_via_link(&self.transport, dest, data, timeout)
            .await
            .map_err(|e| TransportError::LinkFailed(e.to_string()))
    }

    async fn send_via_link_selected_cancellable(
        &self,
        dest: DestinationDesc,
        data: &[u8],
        timeout: Duration,
        representation: LinkRepresentation,
        cancellation: tokio_util::sync::CancellationToken,
        dispatch_gate: super::mesh_transport::DispatchGate,
    ) -> Result<LinkSendResult, TransportError> {
        validate_link_representation(representation, data.len())?;
        rns_core::transport::delivery::send_via_link_cancellable(
            &self.transport,
            dest,
            data,
            timeout,
            cancellation,
            move |kind| {
                let actual = match kind {
                    rns_core::transport::delivery::LinkSendKind::Packet => {
                        LinkRepresentation::Packet
                    }
                    rns_core::transport::delivery::LinkSendKind::Resource => {
                        LinkRepresentation::Resource
                    }
                };
                if actual != representation {
                    return Err(std::io::Error::other("selected link representation changed"));
                }
                dispatch_gate(actual).map_err(|error| std::io::Error::other(error.to_string()))
            },
        )
        .await
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::Interrupted {
                TransportError::Cancelled
            } else {
                TransportError::LinkFailed(error.to_string())
            }
        })
    }

    async fn cancel_resource(&self, hash: rns_core::hash::Hash) -> Result<bool, TransportError> {
        self.transport.cancel_resource(hash).await.map_err(|error| {
            TransportError::SendFailed(format!("resource cancellation: {error:?}"))
        })
    }

    async fn request_path(&self, dest: &AddressHash) {
        self.transport.request_path(dest, None, None).await;
    }

    async fn resolve_identity(&self, dest: &AddressHash) -> Option<Identity> {
        self.transport.destination_identity(dest).await
    }

    async fn open_native_nomadnet_link(
        &self,
        dest: DestinationDesc,
        cancellation: tokio_util::sync::CancellationToken,
        timeout: Duration,
    ) -> Result<crate::transport::mesh_transport::LinkOpenResult, TransportError> {
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
        let transport = Arc::clone(&self.transport);
        tokio::spawn(async move {
            let result =
                open_native_link_owned(Arc::clone(&transport), dest, cancellation, timeout).await;
            let created = result.as_ref().ok().and_then(|result| match result {
                super::mesh_transport::LinkOpenResult::Created(link_id) => Some(*link_id),
                super::mesh_transport::LinkOpenResult::Reused(_) => None,
            });
            if (result_tx.send(result).is_err() || ack_rx.await.is_err())
                && let Some(link_id) = created
                && let Err(error) = close_unclaimed_native_link(&transport, link_id).await
            {
                log::error!(
                    "native link ownership handoff cleanup reached terminal error: {error}"
                );
            }
        });
        let result = result_rx
            .await
            .map_err(|_| TransportError::LinkFailed("native link task stopped".into()))?;
        let _ = ack_tx.send(());
        result
    }

    async fn identify_native_nomadnet_link(
        &self,
        link_id: &str,
        identity: &rns_core::identity::PrivateIdentity,
    ) -> Result<(), TransportError> {
        let bytes: [u8; 16] = hex::decode(link_id)
            .map_err(|_| TransportError::LinkFailed("invalid native link ID".into()))?
            .try_into()
            .map_err(|_| TransportError::LinkFailed("native link ID must be 16 bytes".into()))?;
        self.transport
            .identify_link(&AddressHash::new(bytes), identity)
            .await
            .map_err(|error| TransportError::LinkFailed(format!("link identification: {error:?}")))
    }

    async fn open_named_link(
        &self,
        destination: DestinationDesc,
        cancellation: CancellationToken,
        timeout: Duration,
    ) -> Result<crate::transport::mesh_transport::LinkOpenResult, TransportError> {
        self.open_native_nomadnet_link(destination, cancellation, timeout).await
    }

    async fn identify_link(
        &self,
        link_id: &str,
        identity: &rns_core::identity::PrivateIdentity,
    ) -> Result<(), TransportError> {
        self.identify_native_nomadnet_link(link_id, identity).await
    }

    async fn send_on_link(
        &self,
        link_id: &AddressHash,
        data: &[u8],
    ) -> Result<LinkSendResult, TransportError> {
        rns_core::transport::delivery::send_over_link(&self.transport, link_id, data)
            .await
            .map_err(|error| TransportError::LinkFailed(error.to_string()))
    }

    async fn announce(&self, app_data: Option<&[u8]>) {
        let data = app_data.map(|d| d.to_vec()).or_else(|| self.announce_app_data.clone());
        let _ = self.transport.send_announce(&self.announce_destination, data.as_deref()).await;
    }

    async fn dispatch_announce(&self, app_data: Option<&[u8]>) -> Result<(), TransportError> {
        let data = app_data.map(|d| d.to_vec()).or_else(|| self.announce_app_data.clone());
        match self.transport.send_announce(&self.announce_destination, data.as_deref()).await {
            SendPacketOutcome::SentDirect | SendPacketOutcome::SentBroadcast => Ok(()),
            outcome => Err(TransportError::SendFailed(format!(
                "announce was not accepted by an interface: {outcome:?}"
            ))),
        }
    }

    async fn open_link(
        &self,
        dest: &AddressHash,
        cancellation: tokio_util::sync::CancellationToken,
        timeout: Duration,
    ) -> Result<crate::transport::mesh_transport::LinkOpenResult, TransportError> {
        use crate::transport::mesh_transport::LinkOpenResult;
        use rns_core::transport::core_transport::LinkDispatch;

        let identity =
            self.transport.destination_identity(dest).await.ok_or_else(|| {
                TransportError::LinkFailed("destination identity unavailable".into())
            })?;
        let child_cancellation = tokio_util::sync::CancellationToken::new();
        let link = self.transport.link_cancellable(
            DestinationDesc {
                identity,
                address_hash: *dest,
                name: DestinationName::new("lxmf", "delivery"),
            },
            child_cancellation.clone(),
        );
        tokio::pin!(link);
        enum OpenResult {
            Complete(Option<LinkDispatch>),
            Cancelled,
            TimedOut,
        }
        let result = tokio::select! {
            biased;
            _ = cancellation.cancelled() => OpenResult::Cancelled,
            _ = tokio::time::sleep(timeout) => OpenResult::TimedOut,
            result = &mut link => OpenResult::Complete(result),
        };
        let was_cancelled = matches!(&result, OpenResult::Cancelled);
        match result {
            OpenResult::Complete(Some(LinkDispatch::Created(link))) => {
                Ok(LinkOpenResult::Created(*link.lock().await.id()))
            }
            OpenResult::Complete(Some(LinkDispatch::Reused(link))) => {
                Ok(LinkOpenResult::Reused(*link.lock().await.id()))
            }
            OpenResult::Complete(None) => {
                Err(TransportError::LinkFailed("link dispatch was not accepted".into()))
            }
            OpenResult::Cancelled | OpenResult::TimedOut => {
                child_cancellation.cancel();
                let pending = link.await;
                if let Some(LinkDispatch::Created(link)) = pending {
                    let link_id = *link.lock().await.id();
                    if !self.transport.cancel_link_open(&link_id).await {
                        return Err(TransportError::CleanupFailed(
                            "pending link remained registered".into(),
                        ));
                    }
                }
                if was_cancelled {
                    Err(TransportError::Cancelled)
                } else {
                    Err(TransportError::TimedOut)
                }
            }
        }
    }

    async fn cancel_link_open(&self, link_id: &AddressHash) -> Result<(), TransportError> {
        let active = self
            .transport
            .link_lifecycle_snapshot()
            .await
            .active
            .iter()
            .any(|link| link.id == *link_id);
        let cleaned = if active {
            self.transport.close_link(link_id).await
        } else {
            self.transport.cancel_link_open(link_id).await
        };
        cleaned
            .then_some(())
            .ok_or_else(|| TransportError::CleanupFailed("link remained registered".into()))
    }

    async fn probe_link(&self, link_id: &AddressHash) -> Result<(), TransportError> {
        self.transport
            .probe_link(link_id)
            .await
            .then_some(())
            .ok_or_else(|| TransportError::LinkFailed("link is not active or unavailable".into()))
    }

    async fn close_link(&self, link_id: &AddressHash) -> Result<(), TransportError> {
        self.transport
            .close_link(link_id)
            .await
            .then_some(())
            .ok_or_else(|| TransportError::LinkFailed("link not found".into()))
    }

    fn subscribe_inbound(&self) -> broadcast::Receiver<ReceivedData> {
        self.transport.received_data_events()
    }

    fn subscribe_announces(&self) -> broadcast::Receiver<AnnounceEvent> {
        self.announce_tx.subscribe()
    }

    fn subscribe_lifecycle(&self) -> broadcast::Receiver<TransportLifecycleEvent> {
        self.lifecycle_tx.subscribe()
    }

    fn subscribe_resources(&self) -> broadcast::Receiver<ResourceEvent> {
        self.transport.resource_events()
    }

    fn subscribe_packet_receipts(&self) -> broadcast::Receiver<[u8; 32]> {
        self.packet_receipt_tx.subscribe()
    }

    fn subscribe_routes(&self) -> broadcast::Receiver<RouteEvent> {
        self.route_tx.subscribe()
    }

    fn subscribe_request_observations(&self) -> broadcast::Receiver<RequestLifecycleEvent> {
        self.request_tx.subscribe()
    }

    async fn query_path(&self, dest: &AddressHash) -> Option<(u8, AddressHash)> {
        self.transport.path_info(dest).await
    }

    async fn path_table(&self) -> Vec<(AddressHash, u8, AddressHash, AddressHash)> {
        self.transport.path_table_entries().await
    }

    async fn query_path_snapshot(
        &self,
        dest: &AddressHash,
    ) -> Option<rns_core::transport::core_transport::path_table::PathSnapshot> {
        self.transport.path_snapshot(dest).await
    }

    async fn path_snapshots(
        &self,
    ) -> Vec<rns_core::transport::core_transport::path_table::PathSnapshot> {
        self.transport.path_snapshots().await
    }

    async fn link_lifecycle_snapshot(
        &self,
    ) -> rns_core::transport::destination_ext::link::LinkLifecycleSnapshot {
        self.transport.link_lifecycle_snapshot().await
    }

    fn identity_hash(&self) -> AddressHash {
        self.identity_addr
    }

    fn destination_hash(&self) -> AddressHash {
        self.destination_addr
    }

    fn runtime_identity(&self) -> Option<(AddressHash, AddressHash)> {
        Some((self.identity_addr, self.destination_addr))
    }

    fn is_connected(&self) -> bool {
        true // Transport object existence implies connectivity
    }

    async fn shutdown(&self) -> Result<(), TransportError> {
        if self.shutdown_started.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        self.forwarder_cancel.cancel();
        let mut interface_tasks = {
            let interface_manager = self.transport.iface_manager();
            let mut manager = interface_manager.lock().await;
            manager.shutdown();
            manager.take_tasks()
        };
        let manager_error = self.transport.shutdown_manager().await.err();
        let mut interface_timeout = false;
        for interface_task in &mut interface_tasks {
            if tokio::time::timeout(Duration::from_secs(1), &mut *interface_task).await.is_err() {
                interface_task.abort();
                let _ = (&mut *interface_task).await;
                interface_timeout = true;
            }
        }
        let mut forwarder_timeout = false;
        if let Some(forwarders) = self.forwarders.lock().await.take() {
            for mut forwarder in forwarders {
                if tokio::time::timeout(Duration::from_secs(1), &mut forwarder).await.is_err() {
                    forwarder.abort();
                    let _ = forwarder.await;
                    forwarder_timeout = true;
                }
            }
        }
        self.emit_lifecycle(TransportLifecycleEvent::Disconnected);
        match (manager_error, interface_timeout, forwarder_timeout) {
            (Some(error), _, _) => Err(TransportError::ShutdownFailed(error.to_string())),
            (None, true, _) => Err(TransportError::ShutdownFailed(
                "one or more interface tasks did not stop within one second".into(),
            )),
            (None, false, true) => Err(TransportError::ShutdownFailed(
                "one or more transport forwarding tasks did not stop within one second".into(),
            )),
            (None, false, false) => Ok(()),
        }
    }

    async fn interface_stats(&self) -> HashMap<AddressHash, InterfaceStatsSnapshot> {
        self.transport.interface_stats().await
    }

    async fn interface_snapshots(&self) -> Vec<rns_core::transport::iface::InterfaceSnapshot> {
        self.transport.interface_snapshots().await
    }
}

fn parse_request_id(value: &str) -> Result<[u8; 16], TransportError> {
    hex::decode(value)
        .map_err(|_| TransportError::SendFailed("invalid request ID".into()))?
        .try_into()
        .map_err(|_| TransportError::SendFailed("request ID must be 16 bytes".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_core::OsRng;
    use rns_core::identity::PrivateIdentity;
    use rns_core::transport::core_transport::{LinkDispatch, TransportConfig};
    use rns_core::transport::destination_ext::link::LinkCloseReason;
    use rns_core::transport::iface::{Interface, InterfaceContext};

    struct SinkInterface;

    impl Interface for SinkInterface {
        fn mtu() -> usize {
            1_024
        }
    }

    async fn run_sink(mut context: InterfaceContext<SinkInterface>) {
        while context.channel.tx_channel.recv().await.is_some() {}
    }

    async fn run_cancellable_sink(mut context: InterfaceContext<SinkInterface>) {
        tokio::select! {
            _ = context.cancel.cancelled() => {}
            _ = async { while context.channel.tx_channel.recv().await.is_some() {} } => {}
        }
    }

    #[test]
    fn raw_link_event_preserves_interface_rtt_and_terminal_reason() {
        let interface = AddressHash::new([3; 16]);
        let event = LinkEventData {
            id: AddressHash::new([1; 16]),
            address_hash: AddressHash::new([2; 16]),
            interface: Some(interface),
            rtt: Some(Duration::from_millis(25)),
            remote_identity: None,
            observed_at: std::time::SystemTime::now(),
            event: LinkEvent::Closed(LinkCloseReason::StaleTimeout),
        };

        assert!(matches!(
            lifecycle_from_link_event(event),
            Some(TransportLifecycleEvent::LinkClosed {
                interface: Some(value),
                rtt_ms: Some(25.0),
                reason: LinkCloseReason::StaleTimeout,
                ..
            }) if value == hex::encode(interface.as_slice())
        ));
    }

    #[test]
    fn rns_request_forwarding_lag_requires_reconciliation() {
        assert_eq!(
            request_lifecycle_from_receive(Err(broadcast::error::RecvError::Lagged(7))),
            Ok(RequestLifecycleEvent::ReconcileRequired { dropped: 7 })
        );
    }

    #[tokio::test]
    async fn native_open_preserves_real_reused_link_disposition() {
        let local = PrivateIdentity::new_from_rand(OsRng);
        let transport =
            Arc::new(Transport::new(TransportConfig::new("native-reused-link", &local, true)));
        transport.iface_manager().lock().await.spawn(SinkInterface, run_sink);
        let peer = PrivateIdentity::new_from_rand(OsRng);
        let identity = *peer.as_identity();
        let destination = DestinationDesc {
            identity,
            address_hash: identity.address_hash,
            name: DestinationName::new("nomadnetwork", "node"),
        };
        let first = transport
            .link_cancellable(destination, tokio_util::sync::CancellationToken::new())
            .await
            .expect("real link dispatch");
        let LinkDispatch::Created(link) = first else {
            panic!("first real link must be created");
        };
        let link_id = *link.lock().await.id();
        let _ = link.lock().await.prove();
        let announce = Arc::new(tokio::sync::Mutex::new(SingleInputDestination::new(
            PrivateIdentity::new_from_name("native-reused-announce"),
            DestinationName::new("lxmf", "delivery"),
        )));
        let adapter = TokioTransportAdapter::new(
            Arc::clone(&transport),
            *local.address_hash(),
            AddressHash::new([9; 16]),
            announce,
            None,
        )
        .await;

        let reopened = adapter
            .open_native_nomadnet_link(
                destination,
                tokio_util::sync::CancellationToken::new(),
                Duration::from_secs(1),
            )
            .await
            .expect("reuse active native link");

        assert_eq!(reopened, crate::transport::mesh_transport::LinkOpenResult::Reused(link_id));
        assert!(
            transport
                .link_lifecycle_snapshot()
                .await
                .active
                .iter()
                .any(|snapshot| snapshot.id == link_id)
        );
    }

    #[tokio::test]
    async fn dropped_native_open_waiter_still_cleans_real_pending_link() {
        let local = PrivateIdentity::new_from_rand(OsRng);
        let transport =
            Arc::new(Transport::new(TransportConfig::new("native-dropped-waiter", &local, true)));
        transport.iface_manager().lock().await.spawn(SinkInterface, run_sink);
        let announce = Arc::new(tokio::sync::Mutex::new(SingleInputDestination::new(
            PrivateIdentity::new_from_name("native-dropped-announce"),
            DestinationName::new("lxmf", "delivery"),
        )));
        let adapter = Arc::new(
            TokioTransportAdapter::new(
                Arc::clone(&transport),
                *local.address_hash(),
                AddressHash::new([8; 16]),
                announce,
                None,
            )
            .await,
        );
        let peer = PrivateIdentity::new_from_rand(OsRng);
        let identity = *peer.as_identity();
        let destination = DestinationDesc {
            identity,
            address_hash: identity.address_hash,
            name: DestinationName::new("nomadnetwork", "node"),
        };
        let opening = {
            let adapter = Arc::clone(&adapter);
            tokio::spawn(async move {
                adapter
                    .open_native_nomadnet_link(
                        destination,
                        tokio_util::sync::CancellationToken::new(),
                        Duration::from_millis(30),
                    )
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(5)).await;
        opening.abort();

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if transport.link_lifecycle_snapshot().await.history.iter().any(|link| {
                    link.address_hash == destination.address_hash
                        && link.close_reason == Some(LinkCloseReason::Teardown)
                }) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("detached native open cleanup");
    }

    #[tokio::test]
    async fn shutdown_joins_manager_and_all_forwarders_idempotently() {
        let identity = PrivateIdentity::new_from_name("adapter-shutdown");
        let transport =
            Arc::new(Transport::new(TransportConfig::new("adapter-shutdown", &identity, true)));
        transport.iface_manager().lock().await.spawn(SinkInterface, run_cancellable_sink);
        let destination = Arc::new(tokio::sync::Mutex::new(SingleInputDestination::new(
            identity.clone(),
            DestinationName::new("lxmf", "delivery"),
        )));
        let weak_destination = Arc::downgrade(&destination);
        let destination_address = destination.lock().await.desc.address_hash;
        let adapter = TokioTransportAdapter::new(
            Arc::clone(&transport),
            *identity.address_hash(),
            destination_address,
            destination,
            None,
        )
        .await;

        tokio::time::timeout(Duration::from_secs(2), adapter.shutdown())
            .await
            .expect("bounded shutdown")
            .expect("first shutdown");
        adapter.shutdown().await.expect("idempotent shutdown");
        assert!(transport.manager_task_finished());
        assert!(adapter.forwarders.lock().await.is_none());
        drop(adapter);
        drop(transport);
        assert!(weak_destination.upgrade().is_none());
    }

    #[tokio::test]
    async fn drop_cancels_interface_while_manager_lock_is_contended() {
        let identity = PrivateIdentity::new_from_name("adapter-contended-drop");
        let transport = Arc::new(Transport::new(TransportConfig::new(
            "adapter-contended-drop",
            &identity,
            true,
        )));
        let (stopped_tx, stopped_rx) = tokio::sync::oneshot::channel();
        transport.iface_manager().lock().await.spawn(SinkInterface, move |context| async move {
            context.cancel.cancelled().await;
            let _ = stopped_tx.send(());
        });
        let destination = Arc::new(tokio::sync::Mutex::new(SingleInputDestination::new(
            identity.clone(),
            DestinationName::new("lxmf", "delivery"),
        )));
        let destination_address = destination.lock().await.desc.address_hash;
        let adapter = TokioTransportAdapter::new(
            Arc::clone(&transport),
            *identity.address_hash(),
            destination_address,
            destination,
            None,
        )
        .await;

        let interface_manager = transport.iface_manager();
        let mut manager = interface_manager.lock().await;
        drop(adapter);
        tokio::time::timeout(Duration::from_secs(1), stopped_rx)
            .await
            .expect("interface cancellation deadline")
            .expect("interface cancellation observation");
        for task in manager.take_tasks() {
            task.await.expect("cancelled interface task");
        }
    }
}
