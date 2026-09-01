use super::path::send_to_next_hop;
use super::*;
use ed25519_dalek::{SIGNATURE_LENGTH, Signature};

async fn ingress_registration(
    handler: &TransportHandler,
    destination_hash: AddressHash,
    link_id: AddressHash,
    kind: IngressKind,
) -> Option<(IngressHandler, IngressContext, Option<usize>)> {
    let destination = handler.single_in_destinations.get(&destination_hash)?.clone();
    let destination = destination.lock().await;
    let ingress_handler = destination.ingress_handler()?;
    Some((
        ingress_handler,
        IngressContext { destination: destination_hash, link_id, kind },
        destination.ingress_resource_limit(),
    ))
}

pub(super) fn cached_resource_proof(cache: &PacketCache, request: &Packet) -> Option<Packet> {
    if request.context != PacketContext::CacheRequest || request.data.len() != HASH_SIZE {
        return None;
    }
    let mut hash = [0u8; HASH_SIZE];
    hash.copy_from_slice(request.data.as_slice());
    cache.get(&Hash::new(hash)).filter(|cached| {
        cached.header.packet_type == PacketType::Proof
            && cached.context == PacketContext::ResourceProof
            && cached.destination == request.destination
    })
}

fn validate_destination_receipt_proof(
    identity: &Identity,
    packet: &Packet,
) -> Result<Hash, RnsError> {
    if packet.header.packet_type != PacketType::Proof
        || packet.context == PacketContext::LinkRequestProof
        || packet.data.len() < HASH_SIZE + SIGNATURE_LENGTH
    {
        return Err(RnsError::PacketError);
    }

    let mut hash = [0u8; HASH_SIZE];
    hash.copy_from_slice(&packet.data.as_slice()[..HASH_SIZE]);
    let signature =
        Signature::from_slice(&packet.data.as_slice()[HASH_SIZE..HASH_SIZE + SIGNATURE_LENGTH])
            .map_err(|_| RnsError::CryptoError)?;
    identity.verify(&hash, &signature)?;

    Ok(Hash::new(hash))
}

pub(super) async fn validated_receipt_hash(
    packet: &Packet,
    handler: &TransportHandler,
) -> Option<[u8; HASH_SIZE]> {
    if packet.header.packet_type != PacketType::Proof {
        return None;
    }

    // Canonical Reticulum link proofs carry the default context; this
    // implementation also emits `LinkProof`. Accept both for link receipts.
    if packet.header.destination_type == DestinationType::Link
        && matches!(packet.context, PacketContext::LinkProof | PacketContext::None)
    {
        let mut link = handler
            .in_links
            .get(&packet.destination)
            .cloned()
            .or_else(|| handler.out_links.get(&packet.destination).cloned());
        if link.is_none() {
            for candidate in handler.out_links.values() {
                if *candidate.lock().await.id() == packet.destination {
                    link = Some(candidate.clone());
                    break;
                }
            }
        }
        if let Some(link) = link {
            let link = link.lock().await;
            if let Ok(hash) = link.validate_packet_proof(packet) {
                return Some(hash.to_bytes());
            }
        }
        return None;
    }

    if let Some(destination) = handler.single_out_destinations.get(&packet.destination).cloned() {
        let destination = destination.lock().await;
        if let Ok(hash) = validate_destination_receipt_proof(&destination.identity, packet) {
            return Some(hash.to_bytes());
        }
    }
    if let Some(destination) = handler.single_in_destinations.get(&packet.destination).cloned() {
        let destination = destination.lock().await;
        if let Ok(hash) =
            validate_destination_receipt_proof(destination.identity.as_identity(), packet)
        {
            return Some(hash.to_bytes());
        }
    }

    // Canonical single-packet proofs are addressed to the truncated hash of the
    // proved packet and are implicit by default. Resolve the transmitted packet
    // and validate against the destination identity that received it.
    if let Some(pending) = handler.pending_packet_receipt(&packet.destination)
        && let Some(destination) =
            handler.single_out_destinations.get(&pending.destination).cloned()
    {
        let identity = destination.lock().await.identity;
        if let Ok(hash) = validate_pending_packet_proof(&identity, pending.packet_hash, packet) {
            return Some(hash.to_bytes());
        }
    }

    None
}

/// Validate an implicit (signature only) or explicit (hash and signature)
/// delivery proof for a packet this transport sent.
fn validate_pending_packet_proof(
    identity: &Identity,
    expected: [u8; HASH_SIZE],
    packet: &Packet,
) -> Result<Hash, RnsError> {
    if packet.header.packet_type != PacketType::Proof
        || packet.context == PacketContext::LinkRequestProof
    {
        return Err(RnsError::PacketError);
    }
    let data = packet.data.as_slice();
    let signature_bytes = match data.len() {
        SIGNATURE_LENGTH => data,
        len if len >= HASH_SIZE + SIGNATURE_LENGTH => {
            if data[..HASH_SIZE] != expected {
                return Err(RnsError::IncorrectHash);
            }
            &data[HASH_SIZE..HASH_SIZE + SIGNATURE_LENGTH]
        }
        _ => return Err(RnsError::PacketError),
    };
    let signature = Signature::from_slice(signature_bytes).map_err(|_| RnsError::CryptoError)?;
    identity.verify(&expected, &signature)?;
    Ok(Hash::new(expected))
}

async fn should_forward_link_request_proof(
    packet: &Packet,
    handler: &TransportHandler,
    iface: AddressHash,
) -> bool {
    if packet.context != PacketContext::LinkRequestProof {
        return true;
    }

    let Some((original_destination, expected_iface)) =
        handler.link_table.proof_validation_context(&packet.destination)
    else {
        return false;
    };
    if expected_iface != iface {
        return false;
    }

    let Some(destination) = handler.single_out_destinations.get(&original_destination).cloned()
    else {
        return false;
    };
    let destination = destination.lock().await;

    crate::transport::destination_ext::link::validate_link_request_proof_packet(
        &destination.desc,
        &packet.destination,
        packet,
    )
    .is_ok()
}

pub(super) async fn handle_proof(
    packet: Packet,
    handler: Arc<Mutex<TransportHandler>>,
    iface: AddressHash,
) {
    if packet.context == PacketContext::ResourceProof
        && packet.header.destination_type == DestinationType::Link
    {
        let mut handler = handler.lock().await;
        let mut link = handler
            .in_links
            .get(&packet.destination)
            .cloned()
            .or_else(|| handler.out_links.get(&packet.destination).cloned());
        if link.is_none() {
            for candidate in handler.out_links.values() {
                if *candidate.lock().await.id() == packet.destination {
                    link = Some(candidate.clone());
                    break;
                }
            }
        }
        if let Some(link) = link {
            let mut link = link.lock().await;
            if !matches!(link.status(), LinkStatus::Active | LinkStatus::Stale) {
                return;
            }
            let responses = handler.resource_manager.handle_packet(&packet, &mut link);
            let events = handler.resource_manager.drain_events();
            drop(link);
            for response in responses {
                handler.send_packet(response).await;
            }
            handler.publish_resource_events(events).await;
        }
        return;
    }
    crate::transport_diagnostic!(
        "[tp] proof dst={} ctx={:02x}",
        packet.destination,
        packet.context as u8
    );
    let receipt_hash = {
        let handler = handler.lock().await;
        validated_receipt_hash(&packet, &handler).await
    };
    if let Some(receipt_hash) = receipt_hash {
        let conclusion = {
            let mut handler = handler.lock().await;
            log::trace!("tp({}): handle proof for {}", handler.config.name, packet.destination);
            handler.conclude_receipt(receipt_hash)
        };

        if let Some((receipt, receipt_handler)) = conclusion {
            receipt_handler.on_receipt(&receipt);
        }
    }

    let mut handler = handler.lock().await;

    let mut rtt_packets = Vec::new();
    for link in handler.out_links.values() {
        let mut link = link.lock().await;
        if let LinkHandleResult::Activated = link.handle_packet(&packet, iface) {
            rtt_packets.push(link.create_rtt());
        }
    }
    for packet in rtt_packets {
        handler.send_packet(packet).await;
    }

    let maybe_packet = if should_forward_link_request_proof(&packet, &handler, iface).await {
        handler.link_table.handle_proof(&packet)
    } else {
        None
    };

    if let Some((packet, iface)) = maybe_packet {
        handler.send(TxMessage { tx_type: TxMessageType::Direct(iface), packet }).await;
    }
}

pub(super) async fn handle_keepalive_response<'a>(
    packet: &Packet,
    handler: &mut MutexGuard<'a, TransportHandler>,
) -> bool {
    if packet.context == PacketContext::KeepAlive
        && packet.data.as_slice()[0] == KEEP_ALIVE_RESPONSE
    {
        let lookup = handler.link_table.handle_keepalive(packet);

        if let Some((propagated, iface)) = lookup {
            handler
                .send(TxMessage { tx_type: TxMessageType::Direct(iface), packet: propagated })
                .await;
        }

        return true;
    }

    false
}

pub(super) fn should_encrypt_packet(packet: &Packet) -> bool {
    if packet.header.packet_type != PacketType::Data {
        return false;
    }
    if packet.header.destination_type != DestinationType::Single {
        return false;
    }
    !matches!(
        packet.context,
        PacketContext::Resource
            | PacketContext::ResourceAdvrtisement
            | PacketContext::ResourceRequest
            | PacketContext::ResourceHashUpdate
            | PacketContext::ResourceProof
            | PacketContext::ResourceInitiatorCancel
            | PacketContext::ResourceReceiverCancel
            | PacketContext::KeepAlive
            | PacketContext::CacheRequest
    )
}

pub(super) async fn handle_data<'a>(
    packet: &Packet,
    iface: AddressHash,
    mut handler: MutexGuard<'a, TransportHandler>,
) {
    let mut data_handled = false;

    if packet.header.destination_type == DestinationType::Link {
        if matches!(
            packet.context,
            PacketContext::Resource
                | PacketContext::ResourceAdvrtisement
                | PacketContext::ResourceRequest
                | PacketContext::ResourceHashUpdate
                | PacketContext::ResourceProof
                | PacketContext::ResourceInitiatorCancel
                | PacketContext::ResourceReceiverCancel
                | PacketContext::CacheRequest
        ) {
            let mut link = handler
                .in_links
                .get(&packet.destination)
                .cloned()
                .or_else(|| handler.out_links.get(&packet.destination).cloned());
            if link.is_none() {
                for candidate in handler.out_links.values() {
                    if *candidate.lock().await.id() == packet.destination {
                        link = Some(candidate.clone());
                        break;
                    }
                }
            }

            if let Some(link) = link {
                let mut link = link.lock().await;
                if !matches!(link.status(), LinkStatus::Active | LinkStatus::Stale) {
                    return;
                }
                let needs_decrypt = matches!(
                    packet.context,
                    PacketContext::ResourceAdvrtisement
                        | PacketContext::ResourceRequest
                        | PacketContext::ResourceHashUpdate
                        | PacketContext::ResourceInitiatorCancel
                        | PacketContext::ResourceReceiverCancel
                        | PacketContext::CacheRequest
                );
                let packet_for_manager = if needs_decrypt {
                    let mut buffer = PacketDataBuffer::new();
                    let plain_len =
                        match link.decrypt(packet.data.as_slice(), buffer.accuire_buf_max()) {
                            Ok(plain) => plain.len(),
                            Err(err) => {
                                log::warn!("resource: failed to decrypt packet: {:?}", err);
                                return;
                            }
                        };
                    buffer.resize(plain_len);
                    let mut plain_packet = *packet;
                    plain_packet.data = buffer;
                    plain_packet
                } else {
                    *packet
                };
                if packet_for_manager.context == PacketContext::CacheRequest {
                    let cache = handler.packet_cache.lock().await;
                    let cached = cached_resource_proof(&cache, &packet_for_manager);
                    drop(cache);
                    if let Some(cached) = cached {
                        drop(link);
                        handler
                            .send(TxMessage {
                                tx_type: TxMessageType::Direct(iface),
                                packet: cached,
                            })
                            .await;
                    }
                    return;
                }
                if packet_for_manager.context == PacketContext::ResourceAdvrtisement
                    && let Some(hash) = handler.correlate_resource_advertisement(
                        *link.id(),
                        packet_for_manager.data.as_slice(),
                    )
                {
                    if let Ok(cancel) = crate::transport::resource::build_resource_cancel_packet(
                        &link,
                        hash,
                        PacketContext::ResourceReceiverCancel,
                    ) {
                        drop(link);
                        handler
                            .send(TxMessage {
                                tx_type: TxMessageType::Direct(iface),
                                packet: cancel,
                            })
                            .await;
                    }
                    return;
                }
                let ingress = ingress_registration(
                    &handler,
                    link.destination().address_hash,
                    *link.id(),
                    IngressKind::UnsolicitedResource,
                )
                .await;
                let responses = handler.resource_manager.handle_packet_with_ingress(
                    &packet_for_manager,
                    &mut link,
                    ingress.as_ref().map(|(callback, context, _)| (callback, context)),
                    ingress.as_ref().and_then(|(_, _, limit)| *limit),
                );
                let events = handler.resource_manager.drain_events();
                drop(link);
                for response in responses {
                    handler
                        .send(TxMessage { tx_type: TxMessageType::Direct(iface), packet: response })
                        .await;
                }
                handler.publish_resource_events(events).await;
                return;
            }
        }

        crate::transport_diagnostic!(
            "[tp] link_data dst={} ctx={:02x} len={}",
            packet.destination,
            packet.context as u8,
            packet.data.len()
        );
        let mut link_packets = Vec::new();
        let mut server_request = None;
        let mut inbound_response = None;
        if let Some(link) = handler.in_links.get(&packet.destination).cloned() {
            let mut link = link.lock().await;
            let result = link.handle_packet(packet, iface);
            if let LinkHandleResult::KeepAlive = result {
                link_packets.push(link.keep_alive_packet(KEEP_ALIVE_RESPONSE));
            } else if let LinkHandleResult::Proof(proof_packet) = result {
                link_packets.push(proof_packet);
            } else if let LinkHandleResult::Request(payload) = result {
                let destination_hash = link.destination().address_hash;
                server_request = Some((
                    handler.single_in_destinations.get(&destination_hash).cloned(),
                    destination_hash,
                    *link.id(),
                    link.remote_identity().copied(),
                    payload,
                ));
            } else if let LinkHandleResult::Response(payload) = result {
                inbound_response = Some((*link.id(), payload));
            } else if let LinkHandleResult::Ingress { payload, proof } = result {
                let ingress = ingress_registration(
                    &handler,
                    link.destination().address_hash,
                    *link.id(),
                    IngressKind::LinkPacket,
                )
                .await;
                let accepted = ingress.is_none_or(|(callback, context, _)| {
                    crate::destination::invoke_ingress_handler(
                        &callback,
                        payload.as_slice(),
                        &context,
                    )
                });
                if accepted {
                    link.accept_ingress_payload(payload);
                    link_packets.push(proof);
                }
            }
        }

        if let Some((link_id, payload)) = inbound_response {
            super::requests::correlate_packet_response(&mut handler, link_id, payload.as_slice());
            return;
        }

        if let Some((destination, destination_hash, link_id, remote_identity, payload)) =
            server_request
        {
            let event = super::requests::dispatch_link_request(
                destination,
                destination_hash,
                link_id,
                remote_identity,
                &payload,
            )
            .await;
            super::requests::send_server_response(&mut handler, &event, iface).await;
            let _ = handler.server_request_tx.send(event);
            return;
        }

        let mut proof_packets = Vec::new();
        let mut response_payloads = Vec::new();
        for link in handler.out_links.values() {
            let mut link = link.lock().await;
            let result = link.handle_packet(packet, iface);
            match result {
                LinkHandleResult::Proof(proof_packet) => proof_packets.push(proof_packet),
                LinkHandleResult::Ingress { payload, proof } => {
                    let ingress = ingress_registration(
                        &handler,
                        link.destination().address_hash,
                        *link.id(),
                        IngressKind::LinkPacket,
                    )
                    .await;
                    let accepted = ingress.is_none_or(|(callback, context, _)| {
                        crate::destination::invoke_ingress_handler(
                            &callback,
                            payload.as_slice(),
                            &context,
                        )
                    });
                    if accepted {
                        link.accept_ingress_payload(payload);
                        proof_packets.push(proof);
                    }
                }
                LinkHandleResult::Response(payload) => {
                    response_payloads.push((*link.id(), payload));
                }
                _ => {}
            }
            data_handled = true;
        }

        for (link_id, payload) in response_payloads {
            super::requests::correlate_packet_response(&mut handler, link_id, payload.as_slice());
        }

        for packet in link_packets {
            handler.send(TxMessage { tx_type: TxMessageType::Direct(iface), packet }).await;
        }
        for packet in proof_packets {
            handler.send(TxMessage { tx_type: TxMessageType::Direct(iface), packet }).await;
        }

        if handle_keepalive_response(packet, &mut handler).await {
            return;
        }

        let lookup = handler.link_table.original_destination(&packet.destination);
        if lookup.is_some() {
            let sent = send_to_next_hop(packet, &mut handler, lookup).await;

            log::trace!(
                "tp({}): {} packet to remote link {}",
                handler.config.name,
                if sent { "forwarded" } else { "could not forward" },
                packet.destination
            );
        }
    }

    if packet.header.destination_type == DestinationType::Single {
        if let Some(destination) = handler.single_in_destinations.get(&packet.destination).cloned()
        {
            data_handled = true;
            let mut ratchet_used = false;
            let payload = if should_encrypt_packet(packet) {
                let mut destination = destination.lock().await;
                match destination.decrypt_with_ratchets(packet.data.as_slice()) {
                    Ok((plaintext, used)) => {
                        ratchet_used = used;
                        plaintext
                    }
                    Err(err) => {
                        log::warn!(
                            "tp({}): decrypt failed for {}: {:?}",
                            handler.config.name,
                            packet.destination,
                            err
                        );
                        return;
                    }
                }
            } else {
                packet.data.as_slice().to_vec()
            };
            let mut buffer = PacketDataBuffer::new();
            if buffer.write(&payload).is_err() {
                log::warn!(
                    "tp({}): decrypted payload too large for {}",
                    handler.config.name,
                    packet.destination
                );
                return;
            }
            handler
                .received_data_tx
                .send(ReceivedData {
                    destination: packet.destination,
                    link_id: None,
                    data: buffer,
                    payload_mode: ReceivedPayloadMode::DestinationStripped,
                    ratchet_used,
                    context: Some(packet.context),
                    request_id: if matches!(
                        packet.context,
                        PacketContext::Request | PacketContext::Response
                    ) {
                        let hash = packet.hash().to_bytes();
                        let mut request_id = [0u8; 16];
                        request_id.copy_from_slice(&hash[..16]);
                        Some(request_id)
                    } else {
                        None
                    },
                    hops: Some(packet.header.hops),
                    interface: packet.transport.map(|value| value.as_slice().to_vec()),
                })
                .ok();
        } else {
            data_handled = send_to_next_hop(packet, &mut handler, None).await;
        }
    }

    if data_handled {
        log::trace!(
            "tp({}): handle data request for {} dst={:2x} ctx={:2x}",
            handler.config.name,
            packet.destination,
            packet.header.destination_type as u8,
            packet.context as u8,
        );
    }
}
