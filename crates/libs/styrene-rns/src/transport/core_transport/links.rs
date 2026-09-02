use super::*;
use crate::transport::channel::{
    ChannelError, Envelope as ChannelEnvelope, HandlerId, MessageState as ChannelMessageState,
    TypedMessage, validate_typed_message_type,
};
use crate::transport::destination_ext::link::{
    LinkCloseReason, LinkLifecycleSnapshot, LinkStateSnapshot,
};

const TERMINAL_LINK_HISTORY_CAPACITY: usize = 200;

impl TransportHandler {
    pub(super) fn record_terminal_link(&mut self, snapshot: LinkStateSnapshot) {
        if snapshot.status != LinkStatus::Closed
            || self.terminal_link_history.iter().any(|existing| {
                existing.id == snapshot.id && existing.observed_at == snapshot.observed_at
            })
        {
            return;
        }
        self.request_tracker.link_closed(snapshot.id);
        self.resource_manager.cancel_link(snapshot.id);
        if self.terminal_link_history.len() >= TERMINAL_LINK_HISTORY_CAPACITY {
            self.terminal_link_history.pop_front();
        }
        self.terminal_link_history.push_back(snapshot);
    }
}

enum PreparedOutboundLink {
    Existing(Arc<Mutex<Link>>),
    New { link: Arc<Mutex<Link>>, message: Option<Box<TxMessage>> },
}

/// Outcome of an outbound link dispatch, preserving registration ownership.
pub enum LinkDispatch {
    Created(Arc<Mutex<Link>>),
    Reused(Arc<Mutex<Link>>),
}

pub(super) fn terminalize_request_setup_failure(
    tracker: &mut crate::transport::request::RequestTracker,
    request_id: RequestId,
) -> Option<crate::transport::request::RequestReceipt> {
    tracker.transport_failed(request_id);
    tracker.get(&request_id).cloned()
}

impl Transport {
    #[cfg(feature = "testing")]
    pub async fn intermediate_link_count_for_test(&self) -> usize {
        self.handler.lock().await.link_table.len()
    }

    pub async fn identify_link(
        &self,
        link_id: &AddressHash,
        identity: &PrivateIdentity,
    ) -> Result<(), RnsError> {
        let link = self.find_out_link(link_id).await.ok_or(RnsError::InvalidArgument)?;
        let (iface, packet) = {
            let link = link.lock().await;
            let iface = link.ingress_iface().ok_or(RnsError::InvalidArgument)?;
            (iface, link.identify_packet(identity)?)
        };
        let outcome = self
            .handler
            .lock()
            .await
            .send(TxMessage { tx_type: TxMessageType::Direct(iface), packet })
            .await;
        if outcome.sent_ifaces == 0 {
            return Err(RnsError::InvalidArgument);
        }
        Ok(())
    }

    pub async fn request_over_link(
        &self,
        link_id: &AddressHash,
        path_hash: RequestPathHash,
        request_data: &[u8],
        timeout: Duration,
        max_response_size: usize,
        correlation_id: Option<String>,
    ) -> Result<crate::transport::request::RequestReceipt, RnsError> {
        let link = if let Some(link) = self.find_out_link(link_id).await {
            link
        } else {
            self.find_in_link(link_id).await.ok_or(RnsError::InvalidArgument)?
        };
        let requested_at = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        let envelope = crate::transport::request::encode_request_envelope(
            requested_at,
            path_hash,
            request_data,
        )
        .ok_or(RnsError::InvalidArgument)?;

        let (iface, packet) = {
            let link = link.lock().await;
            if link.status() != LinkStatus::Active {
                return Err(RnsError::InvalidArgument);
            }
            let iface = link.ingress_iface().ok_or(RnsError::InvalidArgument)?;
            let packet = if envelope.len() <= link.packet_mdu() {
                let mut packet = link.data_packet(&envelope)?;
                packet.context = PacketContext::Request;
                Some(packet)
            } else {
                None
            };
            (iface, packet)
        };
        let request_id = if let Some(packet) = packet.as_ref() {
            let hash = packet.hash().to_bytes();
            let mut request_id = [0u8; crate::hash::ADDRESS_HASH_SIZE];
            request_id.copy_from_slice(&hash[..crate::hash::ADDRESS_HASH_SIZE]);
            request_id
        } else {
            crate::transport::request::canonical_request_id(&envelope)
        };

        let mut handler = self.handler.lock().await;
        handler
            .request_tracker
            .start_correlated(
                request_id,
                path_hash,
                *link_id,
                envelope.len(),
                crate::transport::request::RequestOptions {
                    timeout,
                    max_response_size,
                    correlation_id,
                },
            )
            .map_err(|_| RnsError::InvalidArgument)?;
        let sent = if let Some(packet) = packet {
            handler
                .send(TxMessage { tx_type: TxMessageType::Direct(iface), packet })
                .await
                .sent_ifaces
                > 0
        } else {
            let link_guard = link.lock().await;
            let resource =
                handler.resource_manager.start_request(&link_guard, envelope, request_id);
            drop(link_guard);
            let (resource_hash, advertisement) = match resource {
                Ok(resource) => resource,
                Err(error) => {
                    if let Some(receipt) =
                        terminalize_request_setup_failure(&mut handler.request_tracker, request_id)
                    {
                        return Ok(receipt);
                    }
                    return Err(error);
                }
            };
            handler.request_tracker.set_request_resource(request_id, resource_hash);
            let sent = handler
                .send(TxMessage { tx_type: TxMessageType::Direct(iface), packet: advertisement })
                .await
                .sent_ifaces
                > 0;
            handler.resource_manager.confirm_outbound_dispatch(resource_hash, sent);
            sent
        };
        if !sent {
            handler.request_tracker.transport_failed(request_id);
        }
        handler.request_tracker.get(&request_id).cloned().ok_or(RnsError::InvalidArgument)
    }

    pub async fn request_receipt(
        &self,
        request_id: &RequestId,
    ) -> Option<crate::transport::request::RequestReceipt> {
        self.handler.lock().await.request_tracker.get(request_id).cloned()
    }

    pub async fn request_receipts(&self) -> Vec<crate::transport::request::RequestReceipt> {
        self.handler.lock().await.request_tracker.snapshot()
    }

    pub async fn cancel_request(&self, request_id: RequestId) -> bool {
        let mut handler = self.handler.lock().await;
        if !handler.request_tracker.cancel(request_id) {
            return false;
        }
        cancel_correlated_request_resources(&mut handler, request_id).await;
        true
    }

    pub async fn cancel_requests_by_correlation(&self, correlation_id: &str) -> usize {
        let mut handler = self.handler.lock().await;
        let ids = handler.request_tracker.request_ids_by_correlation(correlation_id);
        for request_id in &ids {
            handler.request_tracker.cancel(*request_id);
            cancel_correlated_request_resources(&mut handler, *request_id).await;
        }
        ids.len()
    }

    pub async fn poll_request_timeouts(&self) -> usize {
        let mut handler = self.handler.lock().await;
        let ids = handler.request_tracker.timeout_due_ids();
        for request_id in &ids {
            handler.request_tracker.timeout(*request_id);
            cancel_correlated_request_resources(&mut handler, *request_id).await;
        }
        ids.len()
    }

    pub async fn request_events(
        &self,
    ) -> broadcast::Receiver<crate::transport::request::RequestObservation> {
        self.handler.lock().await.request_tracker.subscribe()
    }

    #[allow(dead_code)] // Used by transport-adjacent test fixtures.
    pub(crate) async fn register_pending_outbound_link(
        &self,
        destination: DestinationDesc,
    ) -> (Arc<Mutex<Link>>, Packet) {
        match self.prepare_outbound_link(destination).await {
            PreparedOutboundLink::New { link, .. } | PreparedOutboundLink::Existing(link) => {
                let request = link.lock().await.request();
                (link, request)
            }
        }
    }

    async fn prepare_outbound_link(&self, destination: DestinationDesc) -> PreparedOutboundLink {
        let mut handler = self.handler.lock().await;
        if let Some(existing) = handler.out_links.get(&destination.address_hash).cloned() {
            if existing.lock().await.status() != LinkStatus::Closed {
                return PreparedOutboundLink::Existing(existing);
            }
            if handler
                .out_links
                .get(&destination.address_hash)
                .is_some_and(|registered| Arc::ptr_eq(registered, &existing))
            {
                handler.out_links.remove(&destination.address_hash);
            }
        }

        let mut raw_link = Link::new(destination, self.link_out_event_tx.clone());
        let route_iface =
            handler.path_table.next_hop_full(&destination.address_hash).map(|(_, iface)| iface);
        let request_mtu = if handler.config.link_mtu_discovery {
            if let Some(iface) = route_iface {
                handler
                    .iface_manager
                    .lock()
                    .await
                    .online_link_mtu(&iface)
                    .unwrap_or(crate::packet::MTU)
            } else {
                crate::packet::MTU
            }
        } else {
            crate::packet::MTU
        };
        raw_link.set_request_mtu(Some(request_mtu));
        let request = raw_link.request();
        let (packet, direct_iface) = handler.path_table.handle_packet(&request);
        let tx_type = if let Some(iface) = direct_iface {
            raw_link.set_ingress_iface(iface);
            Some(TxMessageType::Direct(iface))
        } else if handler.config.broadcast {
            Some(TxMessageType::Broadcast(None))
        } else {
            None
        };
        let link = Arc::new(Mutex::new(raw_link));
        handler.out_links.insert(destination.address_hash, link.clone());
        let message = tx_type.map(|tx_type| Box::new(TxMessage { tx_type, packet }));
        if let Some(message) = message.as_deref() {
            handler.packet_cache.lock().await.update(&message.packet);
        }
        PreparedOutboundLink::New { link, message }
    }

    async fn dispatch_outbound_link_request(
        &self,
        message: Option<TxMessage>,
    ) -> SendPacketOutcome {
        let Some(message) = message else {
            return SendPacketOutcome::DroppedNoRoute;
        };
        let tx_type = message.tx_type;
        let dispatch = self.iface_manager.lock().await.send(message).await;
        if dispatch.sent_ifaces == 0 {
            SendPacketOutcome::DroppedNoRoute
        } else if matches!(tx_type, TxMessageType::Direct(_)) {
            SendPacketOutcome::SentDirect
        } else {
            SendPacketOutcome::SentBroadcast
        }
    }

    pub(crate) async fn link_with_dispatch<F, Fut>(
        &self,
        destination: DestinationDesc,
        dispatch: F,
    ) -> Arc<Mutex<Link>>
    where
        F: FnOnce(Option<TxMessage>) -> Fut,
        Fut: std::future::Future<Output = SendPacketOutcome>,
    {
        let (link, message) = match self.prepare_outbound_link(destination).await {
            PreparedOutboundLink::Existing(link) => return link,
            PreparedOutboundLink::New { link, message } => (link, message.map(|message| *message)),
        };

        let outcome = dispatch(message).await;
        if !matches!(outcome, SendPacketOutcome::SentDirect | SendPacketOutcome::SentBroadcast) {
            {
                let mut handler = self.handler.lock().await;
                if handler
                    .out_links
                    .get(&destination.address_hash)
                    .is_some_and(|registered| Arc::ptr_eq(registered, &link))
                {
                    handler.out_links.remove(&destination.address_hash);
                }
            }
            let snapshot = {
                let mut original = link.lock().await;
                original.close_with_reason(LinkCloseReason::SendFailure);
                original.state_snapshot()
            };
            self.handler.lock().await.record_terminal_link(snapshot);
        }
        link
    }

    pub(crate) async fn link_with_dispatch_cancellable<F, Fut>(
        &self,
        destination: DestinationDesc,
        cancellation: CancellationToken,
        dispatch: F,
    ) -> Option<LinkDispatch>
    where
        F: FnOnce(Option<TxMessage>) -> Fut,
        Fut: std::future::Future<Output = SendPacketOutcome>,
    {
        let (link, message) = match self.prepare_outbound_link(destination).await {
            PreparedOutboundLink::Existing(link) => return Some(LinkDispatch::Reused(link)),
            PreparedOutboundLink::New { link, message } => (link, message.map(|message| *message)),
        };
        let outcome = tokio::select! {
            biased;
            _ = cancellation.cancelled() => None,
            outcome = dispatch(message) => Some(outcome),
        };
        if outcome.is_some_and(|outcome| {
            matches!(outcome, SendPacketOutcome::SentDirect | SendPacketOutcome::SentBroadcast)
        }) {
            return Some(LinkDispatch::Created(link));
        }
        self.abort_registered_link(destination.address_hash, &link).await;
        None
    }

    async fn abort_registered_link(&self, destination: AddressHash, link: &Arc<Mutex<Link>>) {
        {
            let mut handler = self.handler.lock().await;
            if handler
                .out_links
                .get(&destination)
                .is_some_and(|registered| Arc::ptr_eq(registered, link))
            {
                handler.out_links.remove(&destination);
            }
        }
        let snapshot = {
            let mut link = link.lock().await;
            link.close();
            link.state_snapshot()
        };
        self.handler.lock().await.record_terminal_link(snapshot);
    }

    /// Build one Link-context packet per active Link in `links`, optionally
    /// restricted to one destination, paired with that Link's bound interface.
    ///
    /// An established Link always carries the interface its proof or request
    /// arrived on, and the destination path table never learns ephemeral Link
    /// IDs, so a Link without a bound interface is skipped rather than routed.
    async fn collect_bound_link_packets<F>(
        links: &HashMap<AddressHash, Arc<Mutex<Link>>>,
        destination: Option<&AddressHash>,
        build: F,
    ) -> Vec<(AddressHash, Packet)>
    where
        F: Fn(&Link) -> Result<Packet, RnsError>,
    {
        let mut packets = Vec::new();
        for link in links.values() {
            let link = link.lock().await;
            if link.status() != LinkStatus::Active {
                continue;
            }
            if let Some(destination) = destination
                && link.destination().address_hash != *destination
            {
                continue;
            }
            let Some(iface) = link.ingress_iface() else {
                log::trace!("tp: active link {} has no bound interface", link.id());
                continue;
            };
            if let Ok(packet) = build(&link) {
                packets.push((iface, packet));
            }
        }
        packets
    }

    /// Enqueue already-built Link packets directly on their bound interfaces.
    ///
    /// The transport handler is held only to record the packets in the
    /// duplicate cache; interface dispatch runs without it so a slow interface
    /// queue cannot stall protocol processing. Returns the number of packets
    /// accepted by an interface.
    async fn dispatch_bound_link_packets(&self, packets: Vec<(AddressHash, Packet)>) -> usize {
        if packets.is_empty() {
            return 0;
        }
        {
            let handler = self.handler.lock().await;
            let mut cache = handler.packet_cache.lock().await;
            for (_, packet) in &packets {
                cache.update(packet);
            }
        }
        let mut sent = 0usize;
        for (iface, packet) in packets {
            let dispatch = self
                .iface_manager
                .lock()
                .await
                .send(TxMessage { tx_type: TxMessageType::Direct(iface), packet })
                .await;
            if dispatch.sent_ifaces > 0 {
                sent += 1;
            }
        }
        sent
    }

    pub async fn send_channel_to_all_out_links(&self, payload: &[u8]) {
        let packets = {
            let handler = self.handler.lock().await;
            Self::collect_bound_link_packets(&handler.out_links, None, |link| {
                link.channel_packet(payload)
            })
            .await
        };
        self.dispatch_bound_link_packets(packets).await;
    }

    pub async fn send_to_all_out_links(&self, payload: &[u8]) {
        let packets = {
            let handler = self.handler.lock().await;
            Self::collect_bound_link_packets(&handler.out_links, None, |link| {
                link.data_packet(payload)
            })
            .await
        };
        self.dispatch_bound_link_packets(packets).await;
    }

    pub async fn send_to_out_links(&self, destination: &AddressHash, payload: &[u8]) {
        let packets = {
            let handler = self.handler.lock().await;
            Self::collect_bound_link_packets(&handler.out_links, Some(destination), |link| {
                link.data_packet(payload)
            })
            .await
        };
        if self.dispatch_bound_link_packets(packets).await == 0 {
            log::trace!("tp({}): no output links for {} destination", self.name, destination);
        }
    }

    pub async fn send_to_in_links(&self, destination: &AddressHash, payload: &[u8]) {
        let packets = {
            let handler = self.handler.lock().await;
            Self::collect_bound_link_packets(&handler.in_links, Some(destination), |link| {
                link.data_packet(payload)
            })
            .await
        };
        if self.dispatch_bound_link_packets(packets).await == 0 {
            log::trace!("tp({}): no input links for {} destination", self.name, destination);
        }
    }

    pub async fn link_destination(&self, link_id: &AddressHash) -> Option<AddressHash> {
        if let Some(link) = self.find_in_link(link_id).await {
            return Some(link.lock().await.destination().address_hash);
        }
        let link = self.find_out_link(link_id).await?;
        let destination = link.lock().await.destination().address_hash;
        Some(destination)
    }

    pub async fn respond_to_link_request(
        &self,
        link_id: &AddressHash,
        request_id: [u8; crate::hash::ADDRESS_HASH_SIZE],
        response: &[u8],
    ) -> Result<(), RnsError> {
        let link = if let Some(link) = self.find_in_link(link_id).await {
            link
        } else {
            self.find_out_link(link_id).await.ok_or(RnsError::InvalidArgument)?
        };

        let envelope = crate::transport::request::encode_response_envelope(request_id, response)
            .ok_or(RnsError::InvalidArgument)?;

        let (iface, packet) = {
            let link = link.lock().await;
            if link.status() != LinkStatus::Active {
                return Err(RnsError::InvalidArgument);
            }
            let iface = link.ingress_iface().ok_or(RnsError::InvalidArgument)?;
            (iface, link.response_packet(&envelope)?)
        };
        let dispatch = self
            .handler
            .lock()
            .await
            .send(TxMessage { tx_type: TxMessageType::Direct(iface), packet })
            .await;
        if dispatch.sent_ifaces == 0 {
            return Err(RnsError::InvalidArgument);
        }
        Ok(())
    }

    pub async fn respond_to_link_request_resource(
        &self,
        link_id: &AddressHash,
        request_id: [u8; crate::hash::ADDRESS_HASH_SIZE],
        response: Vec<u8>,
    ) -> Result<Hash, RnsError> {
        let link = if let Some(link) = self.find_in_link(link_id).await {
            link
        } else {
            self.find_out_link(link_id).await.ok_or(RnsError::InvalidArgument)?
        };

        self.respond_to_link_request_resource_with_dispatch(
            link,
            request_id,
            response,
            |message| async move {
                self.iface_manager.lock().await.send(message).await.sent_ifaces > 0
            },
        )
        .await
    }

    pub(super) async fn respond_to_link_request_resource_with_dispatch<F, Fut>(
        &self,
        link: Arc<Mutex<Link>>,
        request_id: [u8; crate::hash::ADDRESS_HASH_SIZE],
        response: Vec<u8>,
        dispatch: F,
    ) -> Result<Hash, RnsError>
    where
        F: FnOnce(TxMessage) -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        let response = crate::transport::request::encode_response_envelope(request_id, &response)
            .ok_or(RnsError::InvalidArgument)?;
        let mut handler = self.handler.lock().await;
        let link_guard = link.lock().await;
        if link_guard.status() != LinkStatus::Active {
            return Err(RnsError::InvalidArgument);
        }
        let iface = link_guard.ingress_iface().ok_or(RnsError::InvalidArgument)?;
        let (resource_hash, packet) =
            handler.resource_manager.start_response(&link_guard, response, request_id)?;
        drop(link_guard);
        handler.packet_cache.lock().await.update(&packet);
        drop(handler);
        let sent = dispatch(TxMessage { tx_type: TxMessageType::Direct(iface), packet }).await;
        let confirmed = self
            .handler
            .lock()
            .await
            .resource_manager
            .confirm_outbound_dispatch(resource_hash, sent);
        if !sent || !confirmed {
            return Err(RnsError::InvalidArgument);
        }
        Ok(resource_hash)
    }

    pub async fn send_resource(
        &self,
        link_id: &AddressHash,
        data: Vec<u8>,
        metadata: Option<Vec<u8>>,
    ) -> Result<Hash, RnsError> {
        let (out_links, in_link) = {
            let handler = self.handler.lock().await;
            (
                handler.out_links.values().cloned().collect::<Vec<_>>(),
                handler.in_links.get(link_id).cloned(),
            )
        };

        let link = if let Some(link) = in_link {
            Some(link)
        } else {
            let mut found = None;
            for link in out_links {
                if *link.lock().await.id() == *link_id {
                    found = Some(link);
                    break;
                }
            }
            found
        };

        let link = link.ok_or(RnsError::InvalidArgument)?;
        self.send_resource_with_dispatch(link, data, metadata, |message| async move {
            self.iface_manager.lock().await.send(message).await.sent_ifaces > 0
        })
        .await
    }

    pub(super) async fn send_resource_with_dispatch<F, Fut>(
        &self,
        link: Arc<Mutex<Link>>,
        data: Vec<u8>,
        metadata: Option<Vec<u8>>,
        dispatch: F,
    ) -> Result<Hash, RnsError>
    where
        F: FnOnce(TxMessage) -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        let mut handler = self.handler.lock().await;
        let link_guard = link.lock().await;
        if link_guard.status() != LinkStatus::Active {
            return Err(RnsError::InvalidArgument);
        }
        let iface = link_guard.ingress_iface().ok_or(RnsError::InvalidArgument)?;
        let (resource_hash, packet) =
            handler.resource_manager.start_send(&link_guard, data, metadata)?;
        drop(link_guard);
        handler.packet_cache.lock().await.update(&packet);
        drop(handler);
        let sent = dispatch(TxMessage { tx_type: TxMessageType::Direct(iface), packet }).await;
        let confirmed = self
            .handler
            .lock()
            .await
            .resource_manager
            .confirm_outbound_dispatch(resource_hash, sent);
        if !sent || !confirmed {
            return Err(RnsError::InvalidArgument);
        }
        Ok(resource_hash)
    }

    pub async fn cancel_resource(&self, hash: Hash) -> Result<bool, RnsError> {
        let mut handler = self.handler.lock().await;
        let Some(cancellation) = handler.resource_manager.cancel_local(hash) else {
            return Ok(false);
        };
        let link = find_link_in_handler(&handler, cancellation.link_id).await;
        if let Some(link) = link {
            let link = link.lock().await;
            if let Some(iface) = link.ingress_iface() {
                let packet =
                    build_resource_cancel_packet(&link, cancellation.hash, cancellation.context)?;
                handler.send(TxMessage { tx_type: TxMessageType::Direct(iface), packet }).await;
            }
        }
        for event in handler.resource_manager.drain_events() {
            let _ = handler.resource_events_tx.send(event);
        }
        Ok(true)
    }

    pub async fn resource_state_counts(&self) -> ResourceStateCounts {
        self.handler.lock().await.resource_manager.state_counts()
    }

    pub async fn find_out_link(&self, link_id: &AddressHash) -> Option<Arc<Mutex<Link>>> {
        let links = {
            let handler = self.handler.lock().await;
            handler.out_links.values().cloned().collect::<Vec<_>>()
        };
        for link in links {
            if *link.lock().await.id() == *link_id {
                return Some(link);
            }
        }
        None
    }

    pub async fn find_in_link(&self, link_id: &AddressHash) -> Option<Arc<Mutex<Link>>> {
        self.handler.lock().await.in_links.get(link_id).cloned()
    }

    /// Authoritative active and bounded terminal link state owned by transport.
    pub async fn link_lifecycle_snapshot(&self) -> LinkLifecycleSnapshot {
        let (links, mut history) = {
            let handler = self.handler.lock().await;
            (
                handler
                    .out_links
                    .values()
                    .chain(handler.in_links.values())
                    .cloned()
                    .collect::<Vec<_>>(),
                handler.terminal_link_history.iter().copied().collect::<Vec<_>>(),
            )
        };
        let mut active = Vec::new();
        for link in links {
            let snapshot = link.lock().await.state_snapshot();
            if matches!(snapshot.status, LinkStatus::Active | LinkStatus::Stale)
                && !active.iter().any(|existing: &LinkStateSnapshot| existing.id == snapshot.id)
            {
                active.push(snapshot);
            } else if snapshot.status == LinkStatus::Closed
                && !history.iter().any(|existing| {
                    existing.id == snapshot.id && existing.observed_at == snapshot.observed_at
                })
            {
                if history.len() >= TERMINAL_LINK_HISTORY_CAPACITY {
                    history.remove(0);
                }
                history.push(snapshot);
            }
        }
        active.sort_by(|left, right| left.id.as_slice().cmp(right.id.as_slice()));
        LinkLifecycleSnapshot { active, history }
    }

    /// Start a fresh RTT measurement using a canonical keepalive exchange.
    pub async fn probe_link(&self, link_id: &AddressHash) -> bool {
        let link = if let Some(link) = self.find_out_link(link_id).await {
            link
        } else if let Some(link) = self.find_in_link(link_id).await {
            link
        } else {
            return false;
        };
        let (iface, packet) = {
            let mut link = link.lock().await;
            let Some(iface) = link.ingress_iface() else { return false };
            let Ok(packet) = link.probe_packet() else { return false };
            (iface, packet)
        };
        self.handler
            .lock()
            .await
            .send(TxMessage { tx_type: TxMessageType::Direct(iface), packet })
            .await
            .sent_ifaces
            > 0
    }

    /// Send canonical LINKCLOSE, then close local state after dispatch acceptance.
    pub async fn close_link(&self, link_id: &AddressHash) -> bool {
        let link = if let Some(link) = self.find_out_link(link_id).await {
            link
        } else if let Some(link) = self.find_in_link(link_id).await {
            link
        } else {
            return false;
        };
        let (iface, packet) = {
            let link = link.lock().await;
            let Some(iface) = link.ingress_iface() else { return false };
            let Ok(packet) = link.teardown_packet() else { return false };
            (iface, packet)
        };
        let mut handler = self.handler.lock().await;
        let dispatch =
            handler.send(TxMessage { tx_type: TxMessageType::Direct(iface), packet }).await;
        if dispatch.sent_ifaces == 0 {
            return false;
        }
        let snapshot = {
            let mut link = link.lock().await;
            link.close();
            link.state_snapshot()
        };
        handler.record_terminal_link(snapshot);
        handler.in_links.remove(link_id);
        handler.out_links.retain(|_, candidate| !Arc::ptr_eq(candidate, &link));
        true
    }

    /// Abort local pending link establishment without claiming remote teardown.
    pub async fn cancel_link_open(&self, link_id: &AddressHash) -> bool {
        let Some(link) = self.find_out_link(link_id).await else { return false };
        let destination = link.lock().await.destination().address_hash;
        self.abort_registered_link(destination, &link).await;
        true
    }

    pub async fn link(&self, destination: DestinationDesc) -> Arc<Mutex<Link>> {
        self.link_with_dispatch(destination, |message| self.dispatch_outbound_link_request(message))
            .await
    }

    /// Start a link while allowing cancellation to clean up pending registration.
    pub async fn link_cancellable(
        &self,
        destination: DestinationDesc,
        cancellation: CancellationToken,
    ) -> Option<LinkDispatch> {
        self.link_with_dispatch_cancellable(destination, cancellation, |message| {
            self.dispatch_outbound_link_request(message)
        })
        .await
    }

    pub async fn request_path(
        &self,
        destination: &AddressHash,
        on_iface: Option<AddressHash>,
        tag: Option<TagBytes>,
    ) {
        self.handler.lock().await.request_path(destination, on_iface, tag).await
    }

    pub fn out_link_events(&self) -> broadcast::Receiver<LinkEventData> {
        self.link_out_event_tx.subscribe()
    }

    pub fn in_link_events(&self) -> broadcast::Receiver<LinkEventData> {
        self.link_in_event_tx.subscribe()
    }

    pub fn received_data_events(&self) -> broadcast::Receiver<ReceivedData> {
        self.received_data_tx.subscribe()
    }

    pub fn server_request_events(&self) -> broadcast::Receiver<ServerRequestEvent> {
        self.server_request_tx.subscribe()
    }

    pub async fn add_destination_checked(
        &mut self,
        identity: PrivateIdentity,
        name: DestinationName,
    ) -> Result<Arc<Mutex<SingleInputDestination>>, DestinationRegistrationError> {
        let destination = SingleInputDestination::new(identity, name);
        let address_hash = destination.desc.address_hash;

        log::debug!("tp({}): add destination {}", self.name, address_hash);

        let destination = Arc::new(Mutex::new(destination));

        let mut handler = self.handler.lock().await;
        if handler.single_in_destinations.contains_key(&address_hash) {
            return Err(DestinationRegistrationError::Duplicate(address_hash));
        }
        handler.single_in_destinations.insert(address_hash, destination.clone());

        Ok(destination)
    }

    pub async fn add_destination(
        &mut self,
        identity: PrivateIdentity,
        name: DestinationName,
    ) -> Arc<Mutex<SingleInputDestination>> {
        self.add_destination_checked(identity, name)
            .await
            .expect("duplicate local destination registration")
    }

    pub async fn has_destination(&self, address: &AddressHash) -> bool {
        self.handler.lock().await.has_destination(address)
    }

    pub async fn knows_destination(&self, address: &AddressHash) -> bool {
        self.handler.lock().await.knows_destination(address)
    }

    pub async fn destination_identity(&self, address: &AddressHash) -> Option<Identity> {
        let destination =
            { self.handler.lock().await.single_out_destinations.get(address).cloned() }?;
        let destination = destination.lock().await;
        Some(destination.identity)
    }

    #[cfg(test)]
    pub(crate) fn get_handler(&self) -> Arc<Mutex<TransportHandler>> {
        // direct access to handler for testing purposes
        self.handler.clone()
    }
}

pub(super) async fn find_link_in_handler(
    handler: &TransportHandler,
    link_id: AddressHash,
) -> Option<Arc<Mutex<Link>>> {
    let links =
        handler.in_links.values().chain(handler.out_links.values()).cloned().collect::<Vec<_>>();
    for link in links {
        if *link.lock().await.id() == link_id {
            return Some(link);
        }
    }
    None
}

pub(super) async fn cancel_correlated_request_resources(
    handler: &mut TransportHandler,
    request_id: RequestId,
) {
    let hashes = handler.request_tracker.correlated_resources(request_id);
    for hash in hashes {
        let Some(cancellation) = handler.resource_manager.cancel_local(hash) else { continue };
        let link = find_link_in_handler(handler, cancellation.link_id).await;
        if let Some(link) = link {
            let link = link.lock().await;
            if let Some(iface) = link.ingress_iface()
                && let Ok(packet) =
                    build_resource_cancel_packet(&link, cancellation.hash, cancellation.context)
            {
                handler.send(TxMessage { tx_type: TxMessageType::Direct(iface), packet }).await;
            }
        }
    }
    let events = handler.resource_manager.drain_events();
    handler.publish_resource_events(events).await;
}

impl Drop for Transport {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Ok(mut task) = self.manager_task.lock()
            && let Some(task) = task.take()
        {
            task.abort();
        }
    }
}

impl Transport {
    pub fn manager_task_finished(&self) -> bool {
        self.manager_task
            .lock()
            .ok()
            .and_then(|task| task.as_ref().map(tokio::task::JoinHandle::is_finished))
            .unwrap_or(true)
    }

    pub async fn shutdown_manager(&self) -> Result<(), tokio::task::JoinError> {
        self.cancel.cancel();
        let task = self.manager_task.lock().ok().and_then(|mut task| task.take());
        if let Some(task) = task { task.await } else { Ok(()) }
    }

    /// Abort the transport scheduler when an async join is not possible.
    pub fn abort_manager(&self) {
        self.cancel.cancel();
        if let Ok(mut task) = self.manager_task.lock()
            && let Some(task) = task.take()
        {
            task.abort();
        }
    }

    pub fn channel_for_link(&self, link_id: AddressHash) -> TransportChannel {
        TransportChannel { handler: self.handler.clone(), link_id }
    }

    pub fn channel(&self, link_id: AddressHash) -> TransportChannel {
        self.channel_for_link(link_id)
    }
}

impl TransportChannel {
    async fn find_link(&self) -> Option<Arc<Mutex<Link>>> {
        let (out_links, in_link) = {
            let handler = self.handler.lock().await;
            (
                handler.out_links.values().cloned().collect::<Vec<_>>(),
                handler.in_links.get(&self.link_id).cloned(),
            )
        };

        if let Some(link) = in_link {
            return Some(link);
        }

        for link in out_links {
            if *link.lock().await.id() == self.link_id {
                return Some(link);
            }
        }

        None
    }

    pub fn link_id(&self) -> AddressHash {
        self.link_id
    }

    pub async fn mdu(&self) -> Result<usize, ChannelError> {
        let link = self.find_link().await.ok_or(ChannelError::LinkNotReady)?;
        Ok(link.lock().await.channel_mdu())
    }

    pub async fn send(&self, msg_type: u16, payload: Vec<u8>) -> Result<u16, ChannelError> {
        let link = self.find_link().await.ok_or(ChannelError::LinkNotReady)?;
        let now = self.handler.lock().await.protocol_clock.now();

        let (sequence, iface, packet) = {
            let mut link = link.lock().await;
            let iface = link.ingress_iface().ok_or(ChannelError::LinkNotReady)?;
            let (sequence, packet) = link.send_channel_message_at(msg_type, payload, now)?;
            (sequence, iface, packet)
        };

        let dispatch = self
            .handler
            .lock()
            .await
            .send(TxMessage { tx_type: TxMessageType::Direct(iface), packet })
            .await;
        if dispatch.sent_ifaces == 0 {
            link.lock().await.mark_channel_failed(sequence);
            return Err(ChannelError::LinkNotReady);
        }

        Ok(sequence)
    }

    pub async fn open(&self) -> Result<(), ChannelError> {
        let link = self.find_link().await.ok_or(ChannelError::LinkNotReady)?;
        link.lock().await.open_channel();
        Ok(())
    }

    pub async fn close(&self) -> Result<(), ChannelError> {
        let link = self.find_link().await.ok_or(ChannelError::LinkNotReady)?;
        link.lock().await.close_channel();
        Ok(())
    }

    pub async fn is_ready_to_send(&self) -> Result<bool, ChannelError> {
        let link = self.find_link().await.ok_or(ChannelError::LinkNotReady)?;
        let ready = link.lock().await.channel_ready_to_send();
        Ok(ready)
    }

    /// Delivery state of a message this channel sent, by its sequence number.
    pub async fn state(
        &self,
        sequence: u16,
    ) -> Result<crate::transport::channel::MessageState, ChannelError> {
        let link = self.find_link().await.ok_or(ChannelError::LinkNotReady)?;
        let state = link.lock().await.channel_state(sequence);
        Ok(state)
    }

    pub async fn close_wait_hint(&self) -> Result<Duration, ChannelError> {
        let link = self.find_link().await.ok_or(ChannelError::LinkNotReady)?;
        let hint = link.lock().await.channel_close_wait_hint();
        Ok(hint)
    }

    pub async fn send_typed<M: TypedMessage>(&self, message: &M) -> Result<u16, ChannelError> {
        self.send(M::MSG_TYPE, message.encode()).await
    }

    pub async fn register_handler<F>(
        &self,
        msg_type: u16,
        handler: F,
    ) -> Result<HandlerId, ChannelError>
    where
        F: FnMut(ChannelEnvelope) -> bool + Send + 'static,
    {
        let link = self.find_link().await.ok_or(ChannelError::LinkNotReady)?;
        let handler_id = link.lock().await.register_channel_handler(msg_type, handler);
        Ok(handler_id)
    }

    pub async fn register_typed_handler<M, F>(
        &self,
        mut handler: F,
    ) -> Result<HandlerId, ChannelError>
    where
        M: TypedMessage,
        F: FnMut(M) -> bool + Send + 'static,
    {
        validate_typed_message_type::<M>()?;
        self.register_handler(M::MSG_TYPE, move |envelope| match M::decode(&envelope.payload) {
            Ok(message) => handler(message),
            Err(_) => false,
        })
        .await
    }

    pub async fn remove_handler(&self, handler_id: HandlerId) -> Result<bool, ChannelError> {
        let link = self.find_link().await.ok_or(ChannelError::LinkNotReady)?;
        let removed = link.lock().await.remove_channel_handler(handler_id);
        Ok(removed)
    }

    pub async fn message_state(&self, sequence: u16) -> Result<ChannelMessageState, ChannelError> {
        let link = self.find_link().await.ok_or(ChannelError::LinkNotReady)?;
        let state = link.lock().await.channel_state(sequence);
        Ok(state)
    }
}
