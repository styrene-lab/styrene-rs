#[derive(Debug, Clone)]
struct ResourceReceiver {
    resource_hash: Hash,
    link_id: AddressHash,
    random_hash: [u8; RANDOM_HASH_SIZE],
    parts: Vec<Option<Vec<u8>>>,
    hashmap: Vec<Option<[u8; MAPHASH_LEN]>>,
    consecutive_completed: usize,
    received: usize,
    received_bytes: u64,
    total_bytes: u64,
    data_size: usize,
    resource_sdu: usize,
    maximum_data_size: usize,
    encrypted: bool,
    compressed: bool,
    has_metadata: bool,
    request_id: Option<[u8; ADDRESS_HASH_SIZE]>,
    is_request: bool,
    is_response: bool,
    last_progress: Duration,
    last_request: Duration,
    /// Timeout-driven retries since the last progress; arriving fragments
    /// never consume this budget.
    retry_count: u8,
    /// Map hashes of fragments requested and not yet received. A fragment in
    /// this set is never requested a second time before its round times out.
    outstanding: BTreeSet<[u8; MAPHASH_LEN]>,
    /// A hashmap continuation has been requested and neither arrived nor
    /// expired; at most one continuation is outstanding at a time.
    continuation_pending: bool,
    /// Current bounded request window, grown by clean rounds and shrunk by
    /// timed-out rounds.
    window: usize,
    status: ResourceStatus,
}

/// Why an inbound resource emits a request round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestRound {
    /// First request after the advertisement.
    Initial,
    /// Every outstanding fragment of the previous round arrived.
    Drained,
    /// The previous round timed out without progress.
    Retry,
    /// A hashmap continuation arrived and the active window can refill.
    Continuation,
}

#[derive(Debug, Clone)]
struct ResourcePayload {
    data: Vec<u8>,
    metadata: Option<Vec<u8>>,
}

fn decompress_payload_bounded(
    compressed: &[u8],
    declared_size: usize,
    maximum_size: usize,
) -> Result<Vec<u8>, RnsError> {
    if declared_size > maximum_size {
        return Err(RnsError::InvalidArgument);
    }
    let read_limit = declared_size.checked_add(1).ok_or(RnsError::InvalidArgument)?;
    let decoder = BzDecoder::new(compressed);
    let mut bounded = decoder.take(read_limit as u64);
    let mut decompressed = Vec::with_capacity(declared_size.min(64 * 1024));
    bounded.read_to_end(&mut decompressed).map_err(|_| RnsError::PacketError)?;
    if decompressed.len() != declared_size {
        return Err(RnsError::InvalidArgument);
    }
    Ok(decompressed)
}

#[allow(clippy::large_enum_variant)]
enum PartOutcome {
    NoMatch,
    Incomplete,
    Failed,
    Complete(Packet, ResourcePayload),
}

impl ResourceReceiver {
    fn new(
        adv: &ResourceAdvertisement,
        link_id: AddressHash,
        resource_sdu: usize,
        maximum_data_size: usize,
        now: Duration,
    ) -> Result<Self, RnsError> {
        if resource_sdu == 0
            || maximum_data_size == 0
            || maximum_data_size > MAX_NEGOTIATED_RESOURCE_SIZE
            || adv.transfer_size == 0
            || adv.data_size > maximum_data_size as u64
            || adv.transfer_size
                > maximum_data_size
                    .checked_add(RESOURCE_WIRE_OVERHEAD)
                    .ok_or(RnsError::InvalidArgument)? as u64
        {
            return Err(RnsError::InvalidArgument);
        }
        let transfer_size = usize::try_from(adv.transfer_size)
            .map_err(|_| RnsError::InvalidArgument)?;
        let data_size = usize::try_from(adv.data_size).map_err(|_| RnsError::InvalidArgument)?;
        let expected_parts = transfer_size.div_ceil(resource_sdu);
        let total_parts = usize::try_from(adv.parts).map_err(|_| RnsError::InvalidArgument)?;
        // `segment_index` and `total_segments` describe this resource's place
        // in a split transfer, never the hashmap: an advertisement always
        // carries the first hashmap segment of its own resource, and later
        // hashmap segments arrive as hashmap updates.
        let expected_advertised_hashes = expected_parts.min(HASHMAP_MAX_LEN);
        if total_parts == 0
            || total_parts != expected_parts
            || adv.total_segments == 0
            || adv.segment_index == 0
            || adv.segment_index > adv.total_segments
            || adv.hashmap.len() != expected_advertised_hashes.saturating_mul(MAPHASH_LEN)
        {
            return Err(RnsError::InvalidArgument);
        }
        let mut receiver = Self {
            resource_hash: adv.hash,
            link_id,
            random_hash: adv.random_hash,
            parts: vec![None; total_parts],
            hashmap: vec![None; total_parts],
            consecutive_completed: 0,
            received: 0,
            received_bytes: 0,
            total_bytes: adv.transfer_size,
            data_size,
            resource_sdu,
            maximum_data_size,
            encrypted: adv.encrypted(),
            compressed: adv.compressed(),
            has_metadata: (adv.flags & FLAG_METADATA) == FLAG_METADATA,
            request_id: adv.request_id.as_ref().and_then(|id| id.as_slice().try_into().ok()),
            is_request: adv.is_request(),
            is_response: adv.is_response(),
            last_progress: now,
            last_request: now,
            retry_count: 0,
            outstanding: BTreeSet::new(),
            continuation_pending: false,
            window: WINDOW,
            status: ResourceStatus::Advertised,
        };
        receiver.apply_hashmap_segment(0, &adv.hashmap);
        Ok(receiver)
    }

    fn apply_hashmap_segment(&mut self, segment: usize, bytes: &[u8]) {
        let hashes = bytes.len() / MAPHASH_LEN;
        for i in 0..hashes {
            let start = i * MAPHASH_LEN;
            let mut entry = [0u8; MAPHASH_LEN];
            entry.copy_from_slice(&bytes[start..start + MAPHASH_LEN]);
            let idx = segment * HASHMAP_MAX_LEN + i;
            if idx < self.hashmap.len() {
                self.hashmap[idx] = Some(entry);
            }
        }
    }

    /// Missing fragments of the active window that are not already in
    /// flight, plus a hashmap continuation when the window reaches unmapped
    /// fragments and no continuation is outstanding.
    fn build_request(&self) -> ResourceRequest {
        let mut requested = Vec::new();
        let mut exhausted_at = None;

        let end = (self.consecutive_completed + self.window).min(self.hashmap.len());
        for idx in self.consecutive_completed..end {
            if let Some(hash) = &self.hashmap[idx] {
                if self.parts[idx].is_none() && !self.outstanding.contains(hash) {
                    if requested.len() + self.outstanding.len() >= self.window {
                        break;
                    }
                    requested.push(*hash);
                }
            } else {
                exhausted_at = Some(idx);
                break;
            }
        }

        // The continuation is anchored at the last mapped hash before the
        // gap, which may precede the consecutive height once a whole segment
        // has been received.
        let last_map_hash = exhausted_at
            .filter(|_| !self.continuation_pending)
            .and_then(|idx| self.hashmap[..idx].iter().rev().find_map(|entry| *entry));
        let hashmap_exhausted = last_map_hash.is_some();

        ResourceRequest {
            hashmap_exhausted,
            last_map_hash,
            resource_hash: self.resource_hash,
            requested_hashes: requested,
        }
    }

    fn handle_hash_update(&mut self, update: &ResourceHashUpdate) {
        if update.resource_hash != self.resource_hash {
            return;
        }
        self.apply_hashmap_segment(update.segment as usize, &update.hashmap);
        self.continuation_pending = false;
    }

    fn handle_part(&mut self, part: &[u8], link: &Link, now: Duration) -> PartOutcome {
        let hash = map_hash(part, &self.random_hash);
        let start = self.consecutive_completed;
        let end = (start + self.window).min(self.hashmap.len());
        let Some(index) = self.hashmap[start..end]
            .iter()
            .position(|entry| entry.as_ref() == Some(&hash))
            .map(|index| start + index)
        else {
            return PartOutcome::NoMatch;
        };
        let expected_len = if index + 1 == self.parts.len() {
            usize::try_from(self.total_bytes)
                .ok()
                .and_then(|total| total.checked_sub(index.checked_mul(self.resource_sdu)?))
        } else {
            Some(self.resource_sdu)
        };
        if expected_len != Some(part.len()) {
            self.status = ResourceStatus::Failed;
            return PartOutcome::Failed;
        }

        if self.parts[index].is_none() {
            self.parts[index] = Some(part.to_vec());
            self.received += 1;
            self.received_bytes = self.received_bytes.saturating_add(part.len() as u64);
            self.last_progress = now;
            self.retry_count = 0;
            self.outstanding.remove(&hash);
            while self
                .parts
                .get(self.consecutive_completed)
                .is_some_and(Option::is_some)
            {
                self.consecutive_completed += 1;
            }
        }

        if self.received == self.parts.len() && !self.parts.is_empty() {
            let Some(stream_capacity) = usize::try_from(self.total_bytes).ok() else {
                self.status = ResourceStatus::Failed;
                return PartOutcome::Failed;
            };
            let mut stream = Vec::with_capacity(stream_capacity);
            for part in &self.parts {
                if let Some(bytes) = part {
                    stream.extend_from_slice(bytes);
                } else {
                    return PartOutcome::Incomplete;
                }
            }

            let plain = if self.encrypted {
                let mut out = vec![0u8; stream.len() + 64];
                let decrypted = match link.decrypt(&stream, &mut out) {
                    Ok(value) => value,
                    Err(_) => {
                        self.status = ResourceStatus::Failed;
                        return PartOutcome::Failed;
                    }
                };
                decrypted.to_vec()
            } else {
                stream
            };

            let mut payload = if plain.len() > RANDOM_HASH_SIZE {
                plain[RANDOM_HASH_SIZE..].to_vec()
            } else {
                Vec::new()
            };

            if self.compressed {
                payload = match decompress_payload_bounded(
                    payload.as_slice(),
                    self.data_size,
                    self.maximum_data_size,
                ) {
                    Ok(decompressed) => decompressed,
                    Err(_) => {
                        self.status = ResourceStatus::Failed;
                        return PartOutcome::Failed;
                    }
                };
            }
            if payload.len() != self.data_size || payload.len() > self.maximum_data_size {
                self.status = ResourceStatus::Failed;
                return PartOutcome::Failed;
            }

            let (metadata, data_payload) = if self.has_metadata && payload.len() >= 3 {
                let size = ((payload[0] as usize) << 16)
                    | ((payload[1] as usize) << 8)
                    | payload[2] as usize;
                if size > METADATA_MAX_SIZE {
                    self.status = ResourceStatus::Failed;
                    return PartOutcome::Failed;
                }
                if payload.len() >= 3 + size {
                    let meta = payload[3..3 + size].to_vec();
                    let data = payload[3 + size..].to_vec();
                    (Some(meta), data)
                } else {
                    (None, payload.clone())
                }
            } else {
                (None, payload.clone())
            };

            let mut hasher = sha2::Sha256::new();
            hasher.update(&payload);
            hasher.update(self.random_hash);
            let computed = match copy_hash(&hasher.finalize()) {
                Ok(hash) => Hash::new(hash),
                Err(_) => {
                    self.status = ResourceStatus::Failed;
                    return PartOutcome::Failed;
                }
            };

            if computed == self.resource_hash {
                let mut proof_hasher = sha2::Sha256::new();
                proof_hasher.update(&payload);
                proof_hasher.update(self.resource_hash.as_slice());
                let proof = match copy_hash(&proof_hasher.finalize()) {
                    Ok(hash) => Hash::new(hash),
                    Err(_) => {
                        self.status = ResourceStatus::Failed;
                        return PartOutcome::Failed;
                    }
                };
                let proof_payload = ResourceProof { resource_hash: self.resource_hash, proof };
                self.status = ResourceStatus::Complete;
                let packet = match build_link_packet(
                    link,
                    PacketType::Proof,
                    PacketContext::ResourceProof,
                    &proof_payload.encode(),
                ) {
                    Ok(packet) => packet,
                    Err(_) => {
                        log::warn!("resource: failed to build proof packet");
                        self.status = ResourceStatus::Failed;
                        return PartOutcome::Failed;
                    }
                };
                return PartOutcome::Complete(
                    packet,
                    ResourcePayload { data: data_payload, metadata },
                );
            } else {
                self.status = ResourceStatus::Failed;
                return PartOutcome::Failed;
            }
        }

        PartOutcome::Incomplete
    }

    fn is_active(&self) -> bool {
        !matches!(self.status, ResourceStatus::Complete | ResourceStatus::Failed)
    }

    /// Whether every fragment requested in the current round has arrived.
    fn round_drained(&self) -> bool {
        self.outstanding.is_empty()
    }

    /// Build the request for a new round and account for the transition:
    /// a drained round grows the bounded window, a timed-out round shrinks
    /// it, consumes one retry, and forgets in-flight fragments and any
    /// pending continuation, a continuation refills the window, and the
    /// initial round changes nothing. Returns `None` when there is nothing
    /// to ask for, such as while one continuation is still outstanding.
    fn request_round(&mut self, round: RequestRound, now: Duration) -> Option<ResourceRequest> {
        match round {
            RequestRound::Initial | RequestRound::Continuation => {}
            RequestRound::Drained => {
                self.window = (self.window + 1).min(WINDOW_MAX);
            }
            RequestRound::Retry => {
                self.window = self.window.saturating_sub(1).max(WINDOW_MIN);
                self.retry_count = self.retry_count.saturating_add(1);
                self.outstanding.clear();
                self.continuation_pending = false;
            }
        }
        let request = self.build_request();
        if request.requested_hashes.is_empty() && !request.hashmap_exhausted {
            return None;
        }
        self.outstanding.extend(request.requested_hashes.iter().copied());
        if request.hashmap_exhausted {
            self.continuation_pending = true;
        }
        self.last_request = now;
        Some(request)
    }

    fn retry_due(&self, now: Duration, retry_interval: Duration, max_retries: u8) -> bool {
        if self.status == ResourceStatus::Complete || self.status == ResourceStatus::Failed {
            return false;
        }
        if self.retry_count >= max_retries {
            return false;
        }
        now.saturating_sub(self.last_progress) >= retry_interval
            && now.saturating_sub(self.last_request) >= retry_interval
    }

    fn timeout_due(&self, now: Duration, retry_interval: Duration, max_retries: u8) -> bool {
        self.retry_count >= max_retries
            && now.saturating_sub(self.last_progress) >= retry_interval
            && now.saturating_sub(self.last_request) >= retry_interval
    }

    fn progress(&self) -> ResourceProgress {
        ResourceProgress {
            received_bytes: self.received_bytes,
            total_bytes: self.total_bytes,
            received_parts: self.received,
            total_parts: self.parts.len(),
        }
    }
}
