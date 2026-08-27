const MAX_AWAITING_PROOF_RETRIES: u8 = 3;

#[derive(Debug, Clone)]
struct ResourceSender {
    link_id: AddressHash,
    resource_hash: Hash,
    parts: Vec<Vec<u8>>,
    sent_parts: Vec<bool>,
    map_hashes: Vec<[u8; MAPHASH_LEN]>,
    expected_proof: Hash,
    advertisement_packet: Packet,
    last_activity: Duration,
    adv_sent: Duration,
    last_part_sent: Duration,
    max_retries: u8,
    retries_left: u8,
    status: ResourceStatus,
}

enum OutboundResourcePoll {
    None,
    Send(Box<Packet>),
    RequestProof(Hash),
    Failed,
}

impl ResourceSender {
    fn new(
        link: &Link,
        data: Vec<u8>,
        metadata: Option<Vec<u8>>,
        request_id: Option<ByteBuf>,
        is_response: bool,
        now: Duration,
    ) -> Result<Self, RnsError> {
        let has_metadata = metadata.is_some();
        let metadata_prefix = if let Some(payload) = metadata.as_ref() {
            if payload.len() > METADATA_MAX_SIZE {
                return Err(RnsError::InvalidArgument);
            }
            let size = payload.len() as u32;
            let size_bytes = size.to_be_bytes();
            let mut prefix = Vec::with_capacity(3 + payload.len());
            prefix.extend_from_slice(&size_bytes[1..]);
            prefix.extend_from_slice(payload);
            prefix
        } else {
            Vec::new()
        };
        let mut combined = metadata_prefix.clone();
        combined.extend_from_slice(&data);
        let random_hash = random_bytes::<RANDOM_HASH_SIZE>();
        let data_size = combined.len() as u64;
        let mut hasher = sha2::Sha256::new();
        hasher.update(&combined);
        hasher.update(random_hash);
        let resource_hash = Hash::new(copy_hash(&hasher.finalize())?);

        let mut proof_hasher = sha2::Sha256::new();
        proof_hasher.update(&combined);
        proof_hasher.update(resource_hash.as_slice());
        let expected_proof = Hash::new(copy_hash(&proof_hasher.finalize())?);

        let mut prefix = random_bytes::<RANDOM_HASH_SIZE>().to_vec();
        prefix.extend_from_slice(&combined);

        let mut cipher_buf = vec![0u8; prefix.len() + 128];
        let cipher = link.encrypt(&prefix, &mut cipher_buf).map_err(|_| RnsError::CryptoError)?;
        let cipher_text = cipher.to_vec();

        let resource_sdu = link.resource_sdu();
        let mut parts = Vec::new();
        for chunk in cipher_text.chunks(resource_sdu) {
            parts.push(chunk.to_vec());
        }

        let mut map_hashes = Vec::with_capacity(parts.len());
        for part in &parts {
            map_hashes.push(map_hash(part, &random_hash));
        }

        let advertisement = ResourceAdvertisement {
            transfer_size: parts.iter().map(|part| part.len() as u64).sum(),
            data_size,
            parts: parts.len() as u32,
            hash: resource_hash,
            random_hash,
            original_hash: resource_hash,
            segment_index: 1,
            total_segments: 1,
            request_id: request_id.clone(),
            flags: {
                let mut flags = FLAG_ENCRYPTED;
                if is_response {
                    flags |= FLAG_RESPONSE;
                } else if request_id.is_some() {
                    flags |= FLAG_REQUEST;
                }
                if has_metadata {
                    flags |= FLAG_METADATA;
                }
                flags
            },
            hashmap: slice_hashmap_segment(&map_hashes, 0),
        };
        let advertisement_packet = build_link_packet(
            link,
            PacketType::Data,
            PacketContext::ResourceAdvrtisement,
            &advertisement.pack()?,
        )?;
        Ok(Self {
            link_id: *link.id(),
            resource_hash,
            parts,
            sent_parts: vec![false; map_hashes.len()],
            map_hashes,
            expected_proof,
            advertisement_packet,
            last_activity: now,
            adv_sent: now,
            last_part_sent: now,
            max_retries: 0,
            retries_left: 0,
            status: ResourceStatus::None,
        })
    }

    fn advertisement_packet(&self) -> Packet {
        self.advertisement_packet
    }

    fn mark_advertised(&mut self, retry_limit: u8, now: Duration) {
        self.last_activity = now;
        self.adv_sent = now;
        self.last_part_sent = now;
        self.max_retries = retry_limit;
        self.retries_left = retry_limit;
        self.status = ResourceStatus::Advertised;
    }

    fn poll(&mut self, now: Duration, retry_interval: Duration) -> OutboundResourcePoll {
        match self.status {
            ResourceStatus::Advertised => {
                if now.saturating_sub(self.adv_sent) < retry_interval {
                    return OutboundResourcePoll::None;
                }
                if self.retries_left == 0 {
                    return OutboundResourcePoll::Failed;
                }
                self.retries_left -= 1;
                self.last_activity = now;
                self.adv_sent = now;
                OutboundResourcePoll::Send(Box::new(self.advertisement_packet()))
            }
            ResourceStatus::Transferring => {
                if now.saturating_sub(self.last_activity) < retry_interval {
                    return OutboundResourcePoll::None;
                }
                if self.retries_left == 0 {
                    return OutboundResourcePoll::Failed;
                }
                self.retries_left -= 1;
                self.last_activity = now;
                OutboundResourcePoll::None
            }
            ResourceStatus::AwaitingProof => {
                if now.saturating_sub(self.last_part_sent) < retry_interval {
                    return OutboundResourcePoll::None;
                }
                if self.retries_left == 0 {
                    return OutboundResourcePoll::Failed;
                }
                self.retries_left -= 1;
                self.last_part_sent = now;
                OutboundResourcePoll::RequestProof(self.expected_proof_packet_hash())
            }
            _ => OutboundResourcePoll::None,
        }
    }

    fn handle_request_into(
        &mut self,
        request: &ResourceRequest,
        link: &Link,
        packets: &mut Vec<Packet>,
        now: Duration,
    ) {
        if request.resource_hash != self.resource_hash {
            return;
        }

        let mut sent_any = false;
        let mut scratch_packet = Packet::default();
        for hash in &request.requested_hashes {
            if let Some(index) = self.map_hashes.iter().position(|entry| entry == hash) {
                if let Some(part) = self.parts.get(index) {
                    crate::transport_diagnostic!(
                        "[resource] sending part index={} bytes={} hash={}",
                        index,
                        part.len(),
                        hex::encode(hash)
                    );
                    if build_link_packet_into(
                        link,
                        PacketType::Data,
                        PacketContext::Resource,
                        part,
                        &mut scratch_packet,
                    )
                    .is_ok()
                    {
                        self.sent_parts[index] = true;
                        sent_any = true;
                        packets.push(scratch_packet);
                    } else {
                        log::warn!("resource: failed to build resource packet");
                    }
                }
            }
        }

        if request.hashmap_exhausted {
            if let Some(last_hash) = request.last_map_hash {
                if let Some(last_index) =
                    self.map_hashes.iter().position(|entry| *entry == last_hash)
                {
                    let next_segment = (last_index / HASHMAP_MAX_LEN) + 1;
                    if next_segment * HASHMAP_MAX_LEN < self.map_hashes.len() {
                        let update = ResourceHashUpdate {
                            resource_hash: self.resource_hash,
                            segment: next_segment as u32,
                            hashmap: slice_hashmap_segment(&self.map_hashes, next_segment),
                        };
                        if let Ok(payload) = update.encode() {
                            if let Ok(packet) = build_link_packet(
                                link,
                                PacketType::Data,
                                PacketContext::ResourceHashUpdate,
                                &payload,
                            ) {
                                packets.push(packet);
                            } else {
                                log::warn!("resource: failed to build hash update packet");
                            }
                        }
                    }
                }
            }
        }

        if self.status == ResourceStatus::Advertised
            || self.status == ResourceStatus::Transferring
            || self.status == ResourceStatus::AwaitingProof
        {
            self.last_activity = now;
            self.retries_left = self.max_retries;
            if sent_any {
                self.last_part_sent = now;
            }
            if self.sent_parts.iter().all(|sent| *sent) {
                self.status = ResourceStatus::AwaitingProof;
                // Once all parts are sent, only wait a small, bounded number of
                // retry intervals for the terminal proof before timing out.
                self.retries_left = self.max_retries.clamp(1, MAX_AWAITING_PROOF_RETRIES);
            } else {
                self.status = ResourceStatus::Transferring;
            }
        }
    }

    fn handle_proof(&mut self, proof: &ResourceProof) -> bool {
        if proof.resource_hash != self.resource_hash {
            return false;
        }
        if proof.proof == self.expected_proof {
            self.status = ResourceStatus::Complete;
            return true;
        }
        false
    }

    fn expected_proof_packet_hash(&self) -> Hash {
        let proof = ResourceProof { resource_hash: self.resource_hash, proof: self.expected_proof };
        Packet {
            header: Header {
                destination_type: DestinationType::Link,
                packet_type: PacketType::Proof,
                ..Default::default()
            },
            destination: self.link_id,
            context: PacketContext::ResourceProof,
            data: PacketDataBuffer::new_from_slice(&proof.encode()),
            ..Default::default()
        }
        .hash()
    }
}
