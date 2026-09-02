pub struct ResourceManager {
    pending_outgoing: HashMap<Hash, ResourceSender>,
    outgoing: HashMap<Hash, ResourceSender>,
    incoming: HashMap<Hash, ResourceReceiver>,
    incoming_limits: HashMap<Hash, usize>,
    events: Vec<ResourceEvent>,
    retry_interval: Duration,
    retry_limit: u8,
    clock: Arc<dyn MonotonicClock>,
}

pub(crate) struct ResourceRetryRequest {
    pub link_id: AddressHash,
    pub request: ResourceRequest,
}

pub(crate) struct ResourceCancellation {
    pub link_id: AddressHash,
    pub hash: Hash,
    pub context: PacketContext,
}

#[derive(Default)]
pub(crate) struct ResourcePollActions {
    pub requests: Vec<ResourceRetryRequest>,
    pub packets: Vec<(AddressHash, Packet)>,
    pub cancellations: Vec<ResourceCancellation>,
    pub proof_requests: Vec<(AddressHash, Hash)>,
}

impl ResourceManager {
    pub fn new() -> Self {
        Self::new_with_config_and_clock(
            Duration::from_secs(2),
            5,
            Arc::new(SystemMonotonicClock),
        )
    }

    pub fn new_with_config(retry_interval: Duration, retry_limit: u8) -> Self {
        Self::new_with_config_and_clock(
            retry_interval,
            retry_limit,
            Arc::new(SystemMonotonicClock),
        )
    }

    pub(crate) fn new_with_config_and_clock(
        retry_interval: Duration,
        retry_limit: u8,
        clock: Arc<dyn MonotonicClock>,
    ) -> Self {
        Self {
            pending_outgoing: HashMap::new(),
            outgoing: HashMap::new(),
            incoming: HashMap::new(),
            incoming_limits: HashMap::new(),
            events: Vec::new(),
            retry_interval,
            retry_limit,
            clock,
        }
    }

    pub(crate) fn set_incoming_limit(&mut self, hash: Hash, maximum_data_size: usize) -> bool {
        if maximum_data_size == 0 || maximum_data_size > MAX_NEGOTIATED_RESOURCE_SIZE {
            return false;
        }
        self.incoming_limits.insert(hash, maximum_data_size);
        true
    }

    pub fn start_send(
        &mut self,
        link: &Link,
        data: Vec<u8>,
        metadata: Option<Vec<u8>>,
    ) -> Result<(Hash, Packet), RnsError> {
        let sender =
            ResourceSender::new(link, data, metadata, None, false, self.clock.now())?;
        let resource_hash = sender.resource_hash;
        let packet = sender.advertisement_packet();
        self.pending_outgoing.insert(resource_hash, sender);
        Ok((resource_hash, packet))
    }

    pub fn start_response(
        &mut self,
        link: &Link,
        data: Vec<u8>,
        request_id: [u8; ADDRESS_HASH_SIZE],
    ) -> Result<(Hash, Packet), RnsError> {
        let sender = ResourceSender::new(
            link,
            data,
            None,
            Some(ByteBuf::from(request_id.to_vec())),
            true,
            self.clock.now(),
        )?;
        let resource_hash = sender.resource_hash;
        let packet = sender.advertisement_packet();
        self.pending_outgoing.insert(resource_hash, sender);
        Ok((resource_hash, packet))
    }

    pub fn start_request(
        &mut self,
        link: &Link,
        data: Vec<u8>,
        request_id: [u8; ADDRESS_HASH_SIZE],
    ) -> Result<(Hash, Packet), RnsError> {
        let sender = ResourceSender::new(
            link,
            data,
            None,
            Some(ByteBuf::from(request_id.to_vec())),
            false,
            self.clock.now(),
        )?;
        let resource_hash = sender.resource_hash;
        let packet = sender.advertisement_packet();
        self.pending_outgoing.insert(resource_hash, sender);
        Ok((resource_hash, packet))
    }

    pub fn confirm_outbound_dispatch(&mut self, resource_hash: Hash, sent: bool) -> bool {
        let Some(mut sender) = self.pending_outgoing.remove(&resource_hash) else {
            return false;
        };

        if sent {
            sender.mark_advertised(self.retry_limit, self.clock.now());
            self.outgoing.insert(resource_hash, sender);
            true
        } else {
            false
        }
    }

    pub fn drain_events(&mut self) -> Vec<ResourceEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn state_counts(&self) -> ResourceStateCounts {
        ResourceStateCounts {
            pending_outgoing: self.pending_outgoing.len(),
            outgoing: self.outgoing.len(),
            incoming: self.incoming.len(),
        }
    }

    pub(crate) fn poll(&mut self) -> ResourcePollActions {
        let now = self.clock.now();
        let mut actions = ResourcePollActions::default();
        let pending_timed_out = self
            .pending_outgoing
            .iter()
            .filter_map(|(hash, sender)| {
                (now.saturating_sub(sender.last_activity) >= self.retry_interval)
                    .then_some((*hash, sender.link_id))
            })
            .collect::<Vec<_>>();
        for (hash, link_id) in pending_timed_out {
            self.pending_outgoing.remove(&hash);
            self.events.push(ResourceEvent {
                hash,
                link_id,
                kind: ResourceEventKind::Failed(ResourceFailure::TimedOut),
            });
            actions.cancellations.push(ResourceCancellation {
                link_id,
                hash,
                context: PacketContext::ResourceInitiatorCancel,
            });
        }
        let mut failed = Vec::new();
        for (hash, receiver) in self.incoming.iter_mut() {
            if receiver.retry_due(now, self.retry_interval, self.retry_limit)
                && let Some(request) = receiver.request_round(RequestRound::Retry, now)
            {
                actions.requests.push(ResourceRetryRequest { link_id: receiver.link_id, request });
            }
            if receiver.timeout_due(now, self.retry_interval, self.retry_limit) {
                failed.push((*hash, receiver.link_id));
            }
        }
        for (hash, link_id) in failed {
            self.incoming.remove(&hash);
            self.events.push(ResourceEvent {
                hash,
                link_id,
                kind: ResourceEventKind::Failed(ResourceFailure::TimedOut),
            });
            actions.cancellations.push(ResourceCancellation {
                link_id,
                hash,
                context: PacketContext::ResourceReceiverCancel,
            });
        }

        let mut failed = Vec::new();
        for (hash, sender) in self.outgoing.iter_mut() {
            match sender.poll(now, self.retry_interval) {
                OutboundResourcePoll::Send(packet) => {
                    actions.packets.push((sender.link_id, *packet));
                }
                OutboundResourcePoll::RequestProof(hash) => {
                    actions.proof_requests.push((sender.link_id, hash));
                }
                OutboundResourcePoll::Failed => {
                    failed.push((*hash, sender.link_id));
                }
                OutboundResourcePoll::None => {}
            }
        }

        for (hash, link_id) in failed {
            self.outgoing.remove(&hash);
            self.events.push(ResourceEvent {
                hash,
                link_id,
                kind: ResourceEventKind::Failed(ResourceFailure::TimedOut),
            });
            actions.cancellations.push(ResourceCancellation {
                link_id,
                hash,
                context: PacketContext::ResourceInitiatorCancel,
            });
        }

        actions
    }

    #[cfg(test)]
    fn poll_outgoing(&mut self) -> Vec<(AddressHash, Packet)> {
        self.poll().packets
    }

    pub(crate) fn cancel_local(&mut self, hash: Hash) -> Option<ResourceCancellation> {
        let (link_id, context) = if let Some(receiver) = self.incoming.remove(&hash) {
            (receiver.link_id, PacketContext::ResourceReceiverCancel)
        } else if let Some(sender) = self.pending_outgoing.remove(&hash) {
            (sender.link_id, PacketContext::ResourceInitiatorCancel)
        } else {
            let sender = self.outgoing.remove(&hash)?;
            (sender.link_id, PacketContext::ResourceInitiatorCancel)
        };
        self.events.push(ResourceEvent {
            hash,
            link_id,
            kind: ResourceEventKind::Failed(ResourceFailure::Cancelled),
        });
        Some(ResourceCancellation { link_id, hash, context })
    }

    pub(crate) fn remove_orphaned(&mut self, live_links: &[AddressHash]) {
        let pending = self
            .pending_outgoing
            .iter()
            .filter_map(|(hash, sender)| (!live_links.contains(&sender.link_id)).then_some(*hash))
            .collect::<Vec<_>>();
        let outgoing = self
            .outgoing
            .iter()
            .filter_map(|(hash, sender)| (!live_links.contains(&sender.link_id)).then_some(*hash))
            .collect::<Vec<_>>();
        let incoming = self
            .incoming
            .iter()
            .filter_map(|(hash, receiver)| (!live_links.contains(&receiver.link_id)).then_some(*hash))
            .collect::<Vec<_>>();
        for (hash, link_id) in pending
            .into_iter()
            .filter_map(|hash| self.pending_outgoing.remove(&hash).map(|sender| (hash, sender.link_id)))
            .chain(
                outgoing
                    .into_iter()
                    .filter_map(|hash| self.outgoing.remove(&hash).map(|sender| (hash, sender.link_id))),
            )
            .chain(
                incoming
                    .into_iter()
                    .filter_map(|hash| self.incoming.remove(&hash).map(|receiver| (hash, receiver.link_id))),
            )
        {
            self.events.push(ResourceEvent {
                hash,
                link_id,
                kind: ResourceEventKind::Failed(ResourceFailure::LinkClosed),
            });
        }
    }

    pub(crate) fn cancel_link(&mut self, link_id: AddressHash) {
        let pending = self
            .pending_outgoing
            .iter()
            .filter_map(|(hash, sender)| (sender.link_id == link_id).then_some(*hash))
            .collect::<Vec<_>>();
        let outgoing = self
            .outgoing
            .iter()
            .filter_map(|(hash, sender)| (sender.link_id == link_id).then_some(*hash))
            .collect::<Vec<_>>();
        let incoming = self
            .incoming
            .iter()
            .filter_map(|(hash, receiver)| (receiver.link_id == link_id).then_some(*hash))
            .collect::<Vec<_>>();
        for hash in pending {
            if self.pending_outgoing.remove(&hash).is_some() {
                self.events.push(ResourceEvent {
                    hash,
                    link_id,
                    kind: ResourceEventKind::Failed(ResourceFailure::LinkClosed),
                });
            }
        }
        for hash in outgoing {
            if self.outgoing.remove(&hash).is_some() {
                self.events.push(ResourceEvent {
                    hash,
                    link_id,
                    kind: ResourceEventKind::Failed(ResourceFailure::LinkClosed),
                });
            }
        }
        for hash in incoming {
            if self.incoming.remove(&hash).is_some() {
                self.events.push(ResourceEvent {
                    hash,
                    link_id,
                    kind: ResourceEventKind::Failed(ResourceFailure::LinkClosed),
                });
            }
        }
    }

    pub fn handle_packet(&mut self, packet: &Packet, link: &mut Link) -> Vec<Packet> {
        let mut responses = Vec::new();
        self.handle_packet_into(packet, link, &mut responses, None, None);
        responses
    }

    pub(crate) fn handle_packet_with_ingress(
        &mut self,
        packet: &Packet,
        link: &mut Link,
        ingress: Option<(&crate::destination::IngressHandler, &crate::destination::IngressContext)>,
        maximum_inbound_data_size: Option<usize>,
    ) -> Vec<Packet> {
        if !matches!(link.status(), crate::transport::destination_ext::link::LinkStatus::Active | crate::transport::destination_ext::link::LinkStatus::Stale) {
            return Vec::new();
        }
        let mut responses = Vec::new();
        self.handle_packet_into(
            packet,
            link,
            &mut responses,
            ingress,
            maximum_inbound_data_size,
        );
        responses
    }

    pub fn handle_packet_into(
        &mut self,
        packet: &Packet,
        link: &mut Link,
        responses: &mut Vec<Packet>,
        ingress: Option<(&crate::destination::IngressHandler, &crate::destination::IngressContext)>,
        maximum_inbound_data_size: Option<usize>,
    ) {
        responses.clear();
        match packet.context {
            PacketContext::ResourceAdvrtisement => {
                self.handle_advertisement_into(
                    packet,
                    link,
                    responses,
                    maximum_inbound_data_size,
                )
            }
            PacketContext::ResourceRequest => self.handle_request_into(packet, link, responses),
            PacketContext::ResourceHashUpdate => {
                self.handle_hash_update_into(packet, link, responses)
            }
            PacketContext::Resource => {
                self.handle_resource_part_into(packet, link, responses, ingress)
            }
            PacketContext::ResourceProof => self.handle_proof_into(packet, responses),
            PacketContext::ResourceInitiatorCancel | PacketContext::ResourceReceiverCancel => {
                self.cancel_into(packet, responses)
            }
            _ => {}
        }
    }

    fn handle_advertisement_into(
        &mut self,
        packet: &Packet,
        link: &mut Link,
        responses: &mut Vec<Packet>,
        maximum_inbound_data_size: Option<usize>,
    ) {
        let Ok(advertisement) = ResourceAdvertisement::unpack(packet.data.as_slice()) else {
            return;
        };
        if (advertisement.flags & FLAG_SPLIT) == FLAG_SPLIT {
            log::warn!(
                "resource: rejecting unsupported advertisement flags (split={})",
                (advertisement.flags & FLAG_SPLIT) == FLAG_SPLIT
            );
            return;
        }
        if !advertisement.is_response()
            && maximum_inbound_data_size
                .is_some_and(|maximum| advertisement.data_size > maximum as u64)
        {
            log::warn!(
                "resource: rejecting inbound advertisement above destination limit"
            );
            return;
        }
        let resource_hash = advertisement.hash;
        if self.incoming.get(&resource_hash).is_some_and(|receiver| receiver.is_active()) {
            return;
        }
        let now = self.clock.now();
        let maximum_data_size = self
            .incoming_limits
            .remove(&resource_hash)
            .unwrap_or(MAX_UNSOLICITED_RESOURCE_SIZE);
        let Ok(mut receiver) = ResourceReceiver::new(
            &advertisement,
            *link.id(),
            link.resource_sdu(),
            maximum_data_size,
            now,
        ) else {
            log::warn!("resource: rejecting unreasonable advertisement");
            return;
        };
        let request = receiver.request_round(RequestRound::Initial, now);
        self.incoming.insert(resource_hash, receiver);
        let Some(request) = request else {
            return;
        };
        match build_link_packet(
            link,
            PacketType::Data,
            PacketContext::ResourceRequest,
            &request.encode(),
        ) {
            Ok(packet) => responses.push(packet),
            Err(_) => {
                log::warn!("resource: failed to build request packet");
            }
        };
    }

    fn handle_request_into(
        &mut self,
        packet: &Packet,
        link: &mut Link,
        responses: &mut Vec<Packet>,
    ) {
        let Ok(request) = ResourceRequest::decode(packet.data.as_slice()) else {
            return;
        };
        if let Some(sender) = self.outgoing.get_mut(&request.resource_hash) {
            crate::transport_diagnostic!(
                "[resource] request hash={} requested={} exhausted={}",
                request.resource_hash,
                request.requested_hashes.len(),
                request.hashmap_exhausted
            );
            sender.handle_request_into(&request, link, responses, self.clock.now());
        }
    }

    fn handle_hash_update_into(
        &mut self,
        packet: &Packet,
        link: &mut Link,
        responses: &mut Vec<Packet>,
    ) {
        let Ok(update) = ResourceHashUpdate::decode(packet.data.as_slice()) else {
            return;
        };
        if let Some(receiver) = self.incoming.get_mut(&update.resource_hash) {
            receiver.handle_hash_update(&update);
            let Some(request) =
                receiver.request_round(RequestRound::Continuation, self.clock.now())
            else {
                return;
            };
            match build_link_packet(
                link,
                PacketType::Data,
                PacketContext::ResourceRequest,
                &request.encode(),
            ) {
                Ok(packet) => responses.push(packet),
                Err(_) => {
                    log::warn!("resource: failed to build request packet");
                }
            };
        }
    }

    fn handle_resource_part_into(
        &mut self,
        packet: &Packet,
        link: &mut Link,
        responses: &mut Vec<Packet>,
        ingress: Option<(&crate::destination::IngressHandler, &crate::destination::IngressContext)>,
    ) {
        let mut completed: Option<Hash> = None;
        let mut proof_packet: Option<Packet> = None;
        let mut request_packet: Option<Packet> = None;
        let mut rejection_packet: Option<Packet> = None;
        let mut payload: Option<ResourcePayload> = None;
        let mut failed: Option<(Hash, AddressHash)> = None;
        let mut receiver_request_id = None;
        let mut receiver_is_request = false;
        let mut receiver_is_response = false;
        let mut receiver_transfer_size = 0;
        for (hash, receiver) in self.incoming.iter_mut() {
            let before_received = receiver.received;
            match receiver.handle_part(packet.data.as_slice(), link, self.clock.now()) {
                PartOutcome::NoMatch => continue,
                PartOutcome::Failed => {
                    failed = Some((*hash, receiver.link_id));
                    break;
                }
                PartOutcome::Complete(packet, data_payload) => {
                    completed = Some(*hash);
                    receiver_request_id = receiver.request_id;
                    receiver_is_request = receiver.is_request;
                    receiver_is_response = receiver.is_response;
                    receiver_transfer_size = receiver.total_bytes;
                    proof_packet = Some(packet);
                    payload = Some(data_payload);
                    break;
                }
                PartOutcome::Incomplete => {
                    // One request per drained round: fragments still in flight
                    // from the current round are never asked for again here.
                    if receiver.received > before_received
                        && receiver.round_drained()
                        && let Some(request) =
                            receiver.request_round(RequestRound::Drained, self.clock.now())
                    {
                        request_packet = match build_link_packet(
                            link,
                            PacketType::Data,
                            PacketContext::ResourceRequest,
                            &request.encode(),
                        ) {
                            Ok(packet) => Some(packet),
                            Err(_) => {
                                log::warn!("resource: failed to build request packet");
                                None
                            }
                        };
                    }
                    if receiver.received > before_received {
                        self.events.push(ResourceEvent {
                            hash: *hash,
                            link_id: receiver.link_id,
                            kind: ResourceEventKind::Progress(receiver.progress()),
                        });
                    }
                    break;
                }
            }
        }
        if let Some((hash, link_id)) = failed {
            self.incoming.remove(&hash);
            self.events.push(ResourceEvent {
                hash,
                link_id,
                kind: ResourceEventKind::Failed(ResourceFailure::Integrity),
            });
            return;
        }
        if let Some(hash) = completed {
            self.incoming.remove(&hash);
            if let Some(payload) = payload {
                let complete = ResourceComplete {
                    data: payload.data,
                    metadata: payload.metadata,
                    request_id: receiver_request_id,
                    is_request: receiver_is_request,
                    is_response: receiver_is_response,
                    transfer_size: receiver_transfer_size,
                    checksum_verified: true,
                };
                let unsolicited = complete.request_id.is_none()
                    && !complete.is_request
                    && !complete.is_response;
                let accepted = !unsolicited
                    || ingress.is_none_or(|(handler, context)| {
                        crate::destination::invoke_ingress_handler(handler, &complete.data, context)
                    });
                if accepted {
                    self.events.push(ResourceEvent {
                        hash,
                        link_id: *link.id(),
                        kind: ResourceEventKind::Complete(complete),
                    });
                } else {
                    proof_packet = None;
                    rejection_packet = build_resource_cancel_packet(
                        link,
                        hash,
                        PacketContext::ResourceReceiverCancel,
                    )
                    .ok();
                    self.events.push(ResourceEvent {
                        hash,
                        link_id: *link.id(),
                        kind: ResourceEventKind::Failed(ResourceFailure::Cancelled),
                    });
                }
            }
        }
        if let Some(packet) = proof_packet {
            responses.push(packet);
        } else if let Some(packet) = request_packet {
            responses.push(packet);
        } else if let Some(packet) = rejection_packet {
            responses.push(packet);
        }
    }

    fn handle_proof_into(&mut self, packet: &Packet, _responses: &mut Vec<Packet>) {
        let Ok(proof) = ResourceProof::decode(packet.data.as_slice()) else {
            return;
        };
        if let Some(sender) = self.outgoing.get_mut(&proof.resource_hash)
            && sender.handle_proof(&proof) {
                self.outgoing.remove(&proof.resource_hash);
                self.events.push(ResourceEvent {
                    hash: proof.resource_hash,
                    link_id: packet.destination,
                    kind: ResourceEventKind::OutboundComplete,
                });
            }
    }

    fn cancel_into(&mut self, packet: &Packet, _responses: &mut Vec<Packet>) {
        if let Ok(hash_bytes) = copy_hash(packet.data.as_slice()) {
            let hash = Hash::new(hash_bytes);
            let removed = match packet.context {
                PacketContext::ResourceInitiatorCancel => self.incoming.remove(&hash).is_some(),
                PacketContext::ResourceReceiverCancel => {
                    self.pending_outgoing.remove(&hash).is_some()
                        || self.outgoing.remove(&hash).is_some()
                }
                _ => false,
            };
            if removed {
                self.events.push(ResourceEvent {
                    hash,
                    link_id: packet.destination,
                    kind: ResourceEventKind::Failed(ResourceFailure::Cancelled),
                });
            }
        }
    }
}

impl Default for ResourceManager {
    fn default() -> Self {
        Self::new()
    }
}
