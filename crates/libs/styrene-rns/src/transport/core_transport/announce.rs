use super::announce_limits::AnnounceLimitAction;
use super::*;
use crate::packet::{Header, PropagationType};

async fn process_announce<'a>(
    packet: &Packet,
    mut handler: MutexGuard<'a, TransportHandler>,
    iface: AddressHash,
    announce: crate::destination::AnnounceInfo<'_>,
) -> MutexGuard<'a, TransportHandler> {
    if let Some(existing) = handler.single_out_destinations.get(&packet.destination).cloned() {
        let existing = existing.lock().await;
        if existing.identity.public_key != announce.destination.identity.public_key
            || existing.identity.verifying_key != announce.destination.identity.verifying_key
        {
            log::warn!(
                "tp({}): rejecting announce for {} due to identity drift",
                handler.config.name,
                packet.destination
            );
            return handler;
        }
    }
    let ratchet = announce.ratchet;
    if let Some(ratchet_bytes) = ratchet
        && let Some(store) = handler.ratchet_store.as_mut()
        && let Err(err) = store.remember(&packet.destination, ratchet_bytes)
    {
        log::warn!(
            "tp({}): failed to remember ratchet for {}: {:?}",
            handler.config.name,
            packet.destination,
            err
        );
    }
    // Retransmit/path bookkeeping must use the announced destination hash,
    // not the bare identity hash, otherwise peers learn only identity routes
    // and cannot resolve application destinations like `lxmf.delivery`.
    let dest_hash = announce.destination.desc.address_hash;
    let destination = Arc::new(Mutex::new(announce.destination));

    // Always add to announce/path tables — even for known destinations,
    // updated announces carry new hops/app_data that must be processed.
    // (Upstream fix: BeechatNetworkSystemsLtd/Reticulum-rs PR #83)
    if !handler.single_out_destinations.contains_key(&packet.destination) {
        log::trace!("tp({}): new announce for {}", handler.config.name, packet.destination);
        handler.single_out_destinations.insert(packet.destination, destination.clone());
    }

    // Only a node that drains the retransmission queue may fill it: transport
    // forwarding is enabled, or the announce arrived from a local client
    // instance this node serves. Path responses are never re-queued. Every
    // other accepted announce stays available to path persistence through
    // the bounded cache.
    let (mode, shared_instance) = {
        let manager = handler.iface_manager.lock().await;
        (manager.interface_mode(&iface), manager.is_shared_instance(&iface))
    };
    let drains_queue = handler.config.retransmit || shared_instance;
    if drains_queue && packet.context != PacketContext::PathResponse {
        handler.announce_table.add(packet, dest_hash, iface);
    } else {
        handler.announce_table.retain(packet, dest_hash, iface);
    }
    if let Some(event) = handler.path_table.handle_announce(
        packet,
        packet.transport,
        iface,
        mode,
        announce.random_blob,
    ) {
        let _ = handler.route_tx.send(event);
    }

    if handler.config.retransmit {
        let active_interfaces =
            handler.iface_manager.lock().await.active_interface_hashes().into_iter().collect();
        let waiters = handler.path_requests.take_waiters(&dest_hash, &active_interfaces);
        let transport_id = *handler.config.identity.address_hash();
        for waiter in waiters {
            let response = Packet {
                header: Header {
                    header_type: HeaderType::Type2,
                    propagation_type: PropagationType::Broadcast,
                    destination_type: DestinationType::Single,
                    packet_type: PacketType::Announce,
                    hops: packet.header.hops,
                    context_flag: packet.header.context_flag,
                    ..Default::default()
                },
                destination: dest_hash,
                transport: Some(transport_id),
                context: PacketContext::PathResponse,
                data: packet.data,
                ..Default::default()
            };
            handler
                .send(TxMessage { tx_type: TxMessageType::Direct(waiter), packet: response })
                .await;
        }
    }

    let name_hash = {
        let destination = destination.lock().await;
        let source = destination.desc.name.as_name_hash_slice();
        let mut name_hash = [0u8; crate::destination::NAME_HASH_LENGTH];
        name_hash.copy_from_slice(source);
        name_hash
    };
    let interface = iface.as_slice().to_vec();

    let _ = handler.announce_tx.send(AnnounceEvent {
        destination,
        app_data: PacketDataBuffer::new_from_slice(announce.app_data),
        ratchet,
        name_hash,
        hops: packet.header.hops,
        interface,
    });

    handler
}

pub(super) async fn handle_announce<'a>(
    packet: &Packet,
    handler: MutexGuard<'a, TransportHandler>,
    iface: AddressHash,
) {
    handle_announce_with_class(packet, handler, iface, false).await;
}

pub(super) async fn handle_ingress_limited_announce<'a>(
    packet: &Packet,
    handler: MutexGuard<'a, TransportHandler>,
    iface: AddressHash,
) {
    handle_announce_with_class(packet, handler, iface, true).await;
}

async fn handle_announce_with_class<'a>(
    packet: &Packet,
    mut handler: MutexGuard<'a, TransportHandler>,
    iface: AddressHash,
    ingress_limited: bool,
) {
    // Skip announces for local destinations (upstream PR #83)
    if handler.has_destination(&packet.destination) {
        return;
    }

    let announce = match DestinationAnnounce::validate(packet) {
        Ok(result) => result,
        Err(err) => {
            crate::transport_diagnostic!(
                "[transport] announce validate failed dst={} err={:?}",
                packet.destination,
                err
            );
            return;
        }
    };

    if !ingress_limited {
        let destination_known = handler.has_destination(&packet.destination)
            || handler.knows_destination(&packet.destination);
        match handler.announce_limits.check(iface, packet, destination_known) {
            AnnounceLimitAction::Allow => {}
            AnnounceLimitAction::Hold(release_after) => {
                log::info!(
                    "tp({}): holding announce for {} on iface {} for at least {:?}",
                    handler.config.name,
                    packet.destination,
                    iface,
                    release_after,
                );
                return;
            }
        }
    }

    let _ = process_announce(packet, handler, iface, announce).await;
}

/// Retransmit pending announces.
///
/// When `retransmit_old` is true, also retransmits cached (older) announces
/// that may need periodic re-broadcast for network convergence.
/// Called every `INTERVAL_ANNOUNCES_RETRANSMIT` (1s) with `retransmit_old=false`,
/// and every `INTERVAL_OLD_ANNOUNCES_RETRANSMIT` (300s) with `retransmit_old=true`.
pub(super) async fn retransmit_announces<'a>(
    mut handler: MutexGuard<'a, TransportHandler>,
    retransmit_old: bool,
) {
    let transport_id = *handler.config.identity.address_hash();
    let messages = handler.announce_table.to_retransmit(&transport_id);

    for message in messages {
        handler.send(message).await;
    }

    if retransmit_old {
        let old_messages = handler.announce_table.to_retransmit_old(&transport_id);
        for message in old_messages {
            handler.send(message).await;
        }
    }
}
