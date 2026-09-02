pub struct ResourceManager {
    pending_outgoing: HashMap<Hash, ResourceSender>,
    outgoing: HashMap<Hash, ResourceSender>,
    incoming: HashMap<Hash, ResourceReceiver>,
    incoming_limits: HashMap<Hash, usize>,
    /// Outbound split resources keyed by original hash.
    split_outgoing: HashMap<Hash, SplitOutbound>,
    /// Active outbound segment hash to its original hash.
    outbound_owner: HashMap<Hash, Hash>,
    /// Inbound split resources keyed by original hash.
    split_incoming: HashMap<Hash, SplitInbound>,
    /// Active inbound segment hash to its original hash.
    inbound_owner: HashMap<Hash, Hash>,
    /// Originals whose next segment may be prepared outside the lock.
    due_segments: Vec<Hash>,
    split_segment_size: usize,
    events: Vec<ResourceEvent>,
    retry_interval: Duration,
    retry_limit: u8,
    clock: Arc<dyn MonotonicClock>,
}

/// Outbound split state. Only the first segment is prepared eagerly; the
/// bytes of later segments stay here until the previous segment is proved.
#[derive(Debug, Clone)]
struct SplitOutbound {
    link_id: AddressHash,
    total_segments: u32,
    /// Index of the next segment to prepare.
    next_index: u32,
    remaining: Vec<u8>,
    segment_size: usize,
    /// Segment currently pending or transferring.
    active: Option<Hash>,
    /// A later segment is being prepared outside the lock.
    building: bool,
    sent_bytes: u64,
    total_bytes: u64,
}

impl SplitOutbound {
    /// Segments proved so far: every segment adopted before the next index,
    /// minus the one still in flight. A segment being prepared has not been
    /// adopted yet and is not counted either way.
    fn completed_segments(&self) -> usize {
        let adopted = self.next_index.saturating_sub(1) as usize;
        adopted.saturating_sub(usize::from(self.active.is_some()))
    }

    fn progress(&self) -> ResourceProgress {
        ResourceProgress {
            received_bytes: self.sent_bytes,
            total_bytes: self.total_bytes,
            received_parts: self.completed_segments(),
            total_parts: self.total_segments as usize,
        }
    }
}

/// Inbound split state: segments are appended in order and released as one
/// completion or one terminal failure keyed by the original hash.
#[derive(Debug, Clone)]
struct SplitInbound {
    link_id: AddressHash,
    total_segments: u32,
    /// Segment index expected next.
    next_index: u32,
    data: Vec<u8>,
    metadata: Option<Vec<u8>>,
    received_bytes: u64,
    maximum_data_size: usize,
    active: Option<Hash>,
    request_id: Option<[u8; ADDRESS_HASH_SIZE]>,
    is_request: bool,
    is_response: bool,
}

impl SplitInbound {
    fn progress(&self) -> ResourceProgress {
        ResourceProgress {
            received_bytes: self.received_bytes,
            total_bytes: 0,
            received_parts: self.next_index.saturating_sub(1) as usize,
            total_parts: self.total_segments as usize,
        }
    }
}

/// Bytes of one later segment handed out for construction outside the
/// transport lock.
#[derive(Debug)]
pub(crate) struct PendingSegment {
    pub link_id: AddressHash,
    pub original_hash: Hash,
    pub index: u32,
    pub total: u32,
    pub bytes: Vec<u8>,
}

/// A later segment constructed outside the lock, ready to be adopted.
pub(crate) struct PreparedSegment {
    original_hash: Hash,
    sender: ResourceSender,
}

impl PreparedSegment {
    /// Encrypt and fragment one later segment. This touches only the Link.
    pub(crate) fn build(
        link: &Link,
        pending: PendingSegment,
        now: Duration,
    ) -> Result<Self, RnsError> {
        let sender = ResourceSender::new_segment(
            link,
            pending.bytes,
            None,
            None,
            false,
            Some(SegmentDescriptor {
                original_hash: pending.original_hash,
                index: pending.index,
                total: pending.total,
            }),
            now,
        )?;
        Ok(Self { original_hash: pending.original_hash, sender })
    }
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
            split_outgoing: HashMap::new(),
            outbound_owner: HashMap::new(),
            split_incoming: HashMap::new(),
            inbound_owner: HashMap::new(),
            due_segments: Vec::new(),
            split_segment_size: SPLIT_SEGMENT_SIZE,
            events: Vec::new(),
            retry_interval,
            retry_limit,
            clock,
        }
    }

    pub(crate) fn now(&self) -> Duration {
        self.clock.now()
    }

    #[cfg(test)]
    fn set_split_segment_size(&mut self, size: usize) {
        self.split_segment_size = size;
    }

    /// Map a segment hash to its outbound original hash.
    fn outbound_original(&self, hash: Hash) -> Option<Hash> {
        if self.split_outgoing.contains_key(&hash) {
            Some(hash)
        } else {
            self.outbound_owner.get(&hash).copied()
        }
    }

    /// Map a segment hash to its inbound original hash.
    fn inbound_original(&self, hash: Hash) -> Option<Hash> {
        if self.split_incoming.contains_key(&hash) {
            Some(hash)
        } else {
            self.inbound_owner.get(&hash).copied()
        }
    }

    /// Record one terminal outbound failure. `hash` is a plain resource hash
    /// or a segment hash whose sender the caller already removed; a split
    /// releases its original-hash state and reports exactly once.
    fn fail_outbound(&mut self, hash: Hash, link_id: AddressHash, failure: ResourceFailure) {
        if let Some(original) = self.outbound_original(hash) {
            self.outbound_owner.retain(|_, owner| *owner != original);
            let Some(split) = self.split_outgoing.remove(&original) else {
                return;
            };
            if let Some(active) = split.active {
                self.pending_outgoing.remove(&active);
                self.outgoing.remove(&active);
            }
            self.due_segments.retain(|due| *due != original);
            self.events.push(ResourceEvent {
                hash: original,
                link_id,
                kind: ResourceEventKind::Failed(failure),
                progress: Some(split.progress()),
            });
        } else {
            self.events.push(ResourceEvent::new(hash, link_id, ResourceEventKind::Failed(failure)));
        }
    }

    /// Record one terminal inbound failure; see [`Self::fail_outbound`].
    fn fail_inbound(&mut self, hash: Hash, link_id: AddressHash, failure: ResourceFailure) {
        if let Some(original) = self.inbound_original(hash) {
            self.inbound_owner.retain(|_, owner| *owner != original);
            let Some(split) = self.split_incoming.remove(&original) else {
                return;
            };
            if let Some(active) = split.active {
                self.incoming.remove(&active);
            }
            self.events.push(ResourceEvent {
                hash: original,
                link_id,
                kind: ResourceEventKind::Failed(failure),
                progress: Some(split.progress()),
            });
        } else {
            self.events.push(ResourceEvent::new(hash, link_id, ResourceEventKind::Failed(failure)));
        }
    }

    /// Hand out the bytes of every segment that may now be prepared outside
    /// the lock: its predecessor was proved and nothing is in flight.
    pub(crate) fn take_due_segments(&mut self) -> Vec<PendingSegment> {
        let due = std::mem::take(&mut self.due_segments);
        due.into_iter()
            .filter_map(|original| {
                let split = self.split_outgoing.get_mut(&original)?;
                if split.active.is_some() || split.building || split.remaining.is_empty() {
                    return None;
                }
                let take = split.remaining.len().min(split.segment_size);
                let bytes = split.remaining.drain(..take).collect();
                split.building = true;
                Some(PendingSegment {
                    link_id: split.link_id,
                    original_hash: original,
                    index: split.next_index,
                    total: split.total_segments,
                    bytes,
                })
            })
            .collect()
    }

    /// Adopt a prepared later segment as the split's active pending sender.
    /// Returns `None` when the split was released in the meantime.
    pub(crate) fn adopt_segment(&mut self, prepared: PreparedSegment) -> Option<(Hash, Packet)> {
        let split = self.split_outgoing.get_mut(&prepared.original_hash)?;
        let hash = prepared.sender.resource_hash;
        let packet = prepared.sender.advertisement_packet();
        split.building = false;
        split.active = Some(hash);
        split.next_index = split.next_index.saturating_add(1);
        self.pending_outgoing.insert(hash, prepared.sender);
        self.outbound_owner.insert(hash, prepared.original_hash);
        Some((hash, packet))
    }

    /// Fail an outbound split from outside the manager, such as when a later
    /// segment cannot be built or dispatched. Returns the cancellation to
    /// send so the peer releases its side.
    pub(crate) fn fail_split_outbound(
        &mut self,
        original: Hash,
        failure: ResourceFailure,
    ) -> Option<ResourceCancellation> {
        let split = self.split_outgoing.get(&original)?;
        let link_id = split.link_id;
        let hash = split.active.unwrap_or(original);
        self.fail_outbound(original, link_id, failure);
        Some(ResourceCancellation {
            link_id,
            hash,
            context: PacketContext::ResourceInitiatorCancel,
        })
    }

    pub(crate) fn set_incoming_limit(&mut self, hash: Hash, maximum_data_size: usize) -> bool {
        if maximum_data_size == 0 || maximum_data_size > MAX_NEGOTIATED_RESOURCE_SIZE {
            return false;
        }
        self.incoming_limits.insert(hash, maximum_data_size);
        true
    }

    /// Start an outbound transfer. A payload longer than the split segment
    /// size becomes a split resource: only the first segment is prepared
    /// here, later segments are prepared outside the lock as their
    /// predecessors are proved, and the returned hash is the original hash
    /// that identifies the whole transfer.
    pub fn start_send(
        &mut self,
        link: &Link,
        data: Vec<u8>,
        metadata: Option<Vec<u8>>,
    ) -> Result<(Hash, Packet), RnsError> {
        let prefix_len = metadata.as_ref().map_or(0, |metadata| 3 + metadata.len());
        let segment_size = self.split_segment_size;
        if prefix_len + data.len() <= segment_size {
            let sender =
                ResourceSender::new(link, data, metadata, None, false, self.clock.now())?;
            let resource_hash = sender.resource_hash;
            let packet = sender.advertisement_packet();
            self.pending_outgoing.insert(resource_hash, sender);
            return Ok((resource_hash, packet));
        }
        if prefix_len >= segment_size {
            return Err(RnsError::InvalidArgument);
        }
        let total_bytes = (prefix_len + data.len()) as u64;
        let first_len = segment_size - prefix_len;
        let mut data = data;
        let remaining = data.split_off(first_len);
        let total_segments = 1 + remaining.len().div_ceil(segment_size);
        let total_segments = u32::try_from(total_segments).map_err(|_| RnsError::InvalidArgument)?;
        let original_hash = Hash::new(random_bytes::<HASH_SIZE>());
        let sender = ResourceSender::new_segment(
            link,
            data,
            metadata,
            None,
            false,
            Some(SegmentDescriptor { original_hash, index: 1, total: total_segments }),
            self.clock.now(),
        )?;
        let segment_hash = sender.resource_hash;
        let packet = sender.advertisement_packet();
        self.pending_outgoing.insert(segment_hash, sender);
        self.outbound_owner.insert(segment_hash, original_hash);
        self.split_outgoing.insert(
            original_hash,
            SplitOutbound {
                link_id: *link.id(),
                total_segments,
                next_index: 2,
                remaining,
                segment_size,
                active: Some(segment_hash),
                building: false,
                sent_bytes: 0,
                total_bytes,
            },
        );
        Ok((original_hash, packet))
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
        let resource_hash = self
            .split_outgoing
            .get(&resource_hash)
            .and_then(|split| split.active)
            .unwrap_or(resource_hash);
        let Some(mut sender) = self.pending_outgoing.remove(&resource_hash) else {
            return false;
        };

        if sent {
            sender.mark_advertised(self.retry_limit, self.clock.now());
            self.outgoing.insert(resource_hash, sender);
            true
        } else {
            if let Some(original) = self.outbound_owner.remove(&resource_hash) {
                self.split_outgoing.remove(&original);
                self.due_segments.retain(|due| *due != original);
            }
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
            self.fail_outbound(hash, link_id, ResourceFailure::TimedOut);
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
            self.fail_inbound(hash, link_id, ResourceFailure::TimedOut);
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
            self.fail_outbound(hash, link_id, ResourceFailure::TimedOut);
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

    /// Cancel a local transfer by its resource hash, segment hash, or split
    /// original hash. The returned cancellation names the hash the peer can
    /// resolve: the active segment while one is in flight, otherwise the
    /// original hash.
    pub(crate) fn cancel_local(&mut self, hash: Hash) -> Option<ResourceCancellation> {
        if let Some(original) = self.inbound_original(hash) {
            let split = self.split_incoming.get(&original)?;
            let link_id = split.link_id;
            let wire_hash = split.active.unwrap_or(original);
            self.fail_inbound(original, link_id, ResourceFailure::Cancelled);
            return Some(ResourceCancellation {
                link_id,
                hash: wire_hash,
                context: PacketContext::ResourceReceiverCancel,
            });
        }
        if let Some(original) = self.outbound_original(hash) {
            return self.fail_split_outbound(original, ResourceFailure::Cancelled);
        }
        let (link_id, context) = if let Some(receiver) = self.incoming.remove(&hash) {
            (receiver.link_id, PacketContext::ResourceReceiverCancel)
        } else if let Some(sender) = self.pending_outgoing.remove(&hash) {
            (sender.link_id, PacketContext::ResourceInitiatorCancel)
        } else {
            let sender = self.outgoing.remove(&hash)?;
            (sender.link_id, PacketContext::ResourceInitiatorCancel)
        };
        self.events
            .push(ResourceEvent::new(hash, link_id, ResourceEventKind::Failed(ResourceFailure::Cancelled)));
        Some(ResourceCancellation { link_id, hash, context })
    }

    pub(crate) fn remove_orphaned(&mut self, live_links: &[AddressHash]) {
        self.release_links(|link_id| !live_links.contains(&link_id));
    }

    pub(crate) fn cancel_link(&mut self, link_id: AddressHash) {
        self.release_links(|candidate| candidate == link_id);
    }

    /// Release every transfer whose Link matches `dead`, reporting each
    /// single resource and each split exactly once as link-closed.
    fn release_links(&mut self, dead: impl Fn(AddressHash) -> bool) {
        let pending = self
            .pending_outgoing
            .iter()
            .filter_map(|(hash, sender)| dead(sender.link_id).then_some((*hash, sender.link_id)))
            .collect::<Vec<_>>();
        let outgoing = self
            .outgoing
            .iter()
            .filter_map(|(hash, sender)| dead(sender.link_id).then_some((*hash, sender.link_id)))
            .collect::<Vec<_>>();
        let incoming = self
            .incoming
            .iter()
            .filter_map(|(hash, receiver)| {
                dead(receiver.link_id).then_some((*hash, receiver.link_id))
            })
            .collect::<Vec<_>>();
        for (hash, link_id) in pending {
            if self.pending_outgoing.remove(&hash).is_some() {
                self.fail_outbound(hash, link_id, ResourceFailure::LinkClosed);
            }
        }
        for (hash, link_id) in outgoing {
            if self.outgoing.remove(&hash).is_some() {
                self.fail_outbound(hash, link_id, ResourceFailure::LinkClosed);
            }
        }
        for (hash, link_id) in incoming {
            if self.incoming.remove(&hash).is_some() {
                self.fail_inbound(hash, link_id, ResourceFailure::LinkClosed);
            }
        }
        // Splits caught between segments have no active sender or receiver.
        let idle_outbound = self
            .split_outgoing
            .iter()
            .filter_map(|(original, split)| dead(split.link_id).then_some((*original, split.link_id)))
            .collect::<Vec<_>>();
        for (original, link_id) in idle_outbound {
            self.fail_outbound(original, link_id, ResourceFailure::LinkClosed);
        }
        let idle_inbound = self
            .split_incoming
            .iter()
            .filter_map(|(original, split)| dead(split.link_id).then_some((*original, split.link_id)))
            .collect::<Vec<_>>();
        for (original, link_id) in idle_inbound {
            self.fail_inbound(original, link_id, ResourceFailure::LinkClosed);
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
        let split =
            (advertisement.flags & FLAG_SPLIT) == FLAG_SPLIT && advertisement.total_segments > 1;
        if split
            && (advertisement.segment_index == 0
                || advertisement.segment_index > advertisement.total_segments)
        {
            log::warn!("resource: rejecting split advertisement with an invalid segment index");
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
        let mut maximum_data_size = self
            .incoming_limits
            .remove(&resource_hash)
            .unwrap_or(MAX_UNSOLICITED_RESOURCE_SIZE);
        if split {
            let original = advertisement.original_hash;
            match self.split_incoming.get(&original) {
                None => {
                    if advertisement.segment_index != 1 {
                        log::warn!("resource: rejecting split segment without its first segment");
                        return;
                    }
                }
                Some(record) => {
                    if record.active.is_some() {
                        return;
                    }
                    let mismatch = record.link_id != *link.id()
                        || record.total_segments != advertisement.total_segments
                        || record.next_index != advertisement.segment_index;
                    let overflow = record
                        .received_bytes
                        .saturating_add(advertisement.data_size)
                        > record.maximum_data_size as u64;
                    if mismatch || overflow {
                        log::warn!("resource: split segment does not continue its original");
                        let link_id = record.link_id;
                        self.fail_inbound(original, link_id, ResourceFailure::Integrity);
                        return;
                    }
                    maximum_data_size = record.maximum_data_size;
                }
            }
        }
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
        if split {
            let original = advertisement.original_hash;
            let record = self.split_incoming.entry(original).or_insert_with(|| SplitInbound {
                link_id: *link.id(),
                total_segments: advertisement.total_segments,
                next_index: 1,
                data: Vec::new(),
                metadata: None,
                received_bytes: 0,
                maximum_data_size,
                active: None,
                request_id: receiver.request_id,
                is_request: receiver.is_request,
                is_response: receiver.is_response,
            });
            record.active = Some(resource_hash);
            self.inbound_owner.insert(resource_hash, original);
        }
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
                        self.events.push(ResourceEvent::new(*hash, receiver.link_id, ResourceEventKind::Progress(receiver.progress())));
                    }
                    break;
                }
            }
        }
        if let Some((hash, link_id)) = failed {
            self.incoming.remove(&hash);
            self.fail_inbound(hash, link_id, ResourceFailure::Integrity);
            return;
        }
        if let Some(hash) = completed {
            self.incoming.remove(&hash);
            let split_original = self.inbound_owner.remove(&hash);
            let assembled = match (split_original, payload) {
                (Some(original), Some(payload)) => {
                    let Some(record) = self.split_incoming.get_mut(&original) else {
                        return;
                    };
                    record.active = None;
                    if record.next_index == 1 {
                        record.metadata = payload.metadata;
                    }
                    record.data.extend_from_slice(&payload.data);
                    record.received_bytes =
                        record.received_bytes.saturating_add(receiver_transfer_size);
                    record.next_index = record.next_index.saturating_add(1);
                    if record.next_index <= record.total_segments {
                        let progress = record.progress();
                        self.events.push(ResourceEvent::new(
                            original,
                            *link.id(),
                            ResourceEventKind::Progress(progress),
                        ));
                        None
                    } else {
                        let record = self.split_incoming.remove(&original);
                        record.map(|record| {
                            (
                                original,
                                ResourcePayload { data: record.data, metadata: record.metadata },
                                record.request_id,
                                record.is_request,
                                record.is_response,
                                record.received_bytes,
                            )
                        })
                    }
                }
                (None, Some(payload)) => Some((
                    hash,
                    payload,
                    receiver_request_id,
                    receiver_is_request,
                    receiver_is_response,
                    receiver_transfer_size,
                )),
                (_, None) => None,
            };
            if let Some((hash, payload, request_id, is_request, is_response, transfer_size)) =
                assembled
            {
                let complete = ResourceComplete {
                    data: payload.data,
                    metadata: payload.metadata,
                    request_id,
                    is_request,
                    is_response,
                    transfer_size,
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
                    self.events.push(ResourceEvent::new(
                        hash,
                        *link.id(),
                        ResourceEventKind::Complete(complete),
                    ));
                } else {
                    proof_packet = None;
                    rejection_packet = build_resource_cancel_packet(
                        link,
                        hash,
                        PacketContext::ResourceReceiverCancel,
                    )
                    .ok();
                    self.events.push(ResourceEvent::new(
                        hash,
                        *link.id(),
                        ResourceEventKind::Failed(ResourceFailure::Cancelled),
                    ));
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
            && sender.handle_proof(&proof)
        {
            let Some(sender) = self.outgoing.remove(&proof.resource_hash) else {
                return;
            };
            if let Some(original) = self.outbound_owner.remove(&proof.resource_hash) {
                let Some(split) = self.split_outgoing.get_mut(&original) else {
                    return;
                };
                split.active = None;
                split.sent_bytes = split
                    .sent_bytes
                    .saturating_add(sender.parts.iter().map(|part| part.len() as u64).sum());
                if split.next_index > split.total_segments {
                    self.split_outgoing.remove(&original);
                    self.events.push(ResourceEvent::new(
                        original,
                        packet.destination,
                        ResourceEventKind::OutboundComplete,
                    ));
                } else {
                    self.due_segments.push(original);
                }
                return;
            }
            self.events.push(ResourceEvent::new(
                proof.resource_hash,
                packet.destination,
                ResourceEventKind::OutboundComplete,
            ));
        }
    }

    fn cancel_into(&mut self, packet: &Packet, _responses: &mut Vec<Packet>) {
        if let Ok(hash_bytes) = copy_hash(packet.data.as_slice()) {
            let hash = Hash::new(hash_bytes);
            match packet.context {
                PacketContext::ResourceInitiatorCancel => {
                    let removed = self.incoming.remove(&hash).is_some();
                    if removed || self.inbound_original(hash).is_some() {
                        self.fail_inbound(hash, packet.destination, ResourceFailure::Cancelled);
                    }
                }
                PacketContext::ResourceReceiverCancel => {
                    let removed = self.pending_outgoing.remove(&hash).is_some()
                        || self.outgoing.remove(&hash).is_some();
                    if removed || self.outbound_original(hash).is_some() {
                        self.fail_outbound(hash, packet.destination, ResourceFailure::Cancelled);
                    }
                }
                _ => {}
            }
        }
    }
}

impl Default for ResourceManager {
    fn default() -> Self {
        Self::new()
    }
}
