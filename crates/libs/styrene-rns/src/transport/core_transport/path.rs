use super::*;
use alloc::collections::BTreeSet;

pub(super) async fn dispatch_pending_path_requests<'a>(
    handler: &mut MutexGuard<'a, TransportHandler>,
) {
    let Some(destination) = handler.path_requests.pending_front() else {
        return;
    };
    let Some(packet) = handler.path_requests.pending_packet(&destination) else {
        return;
    };
    let active_interfaces: BTreeSet<_> =
        handler.iface_manager.lock().await.active_interface_hashes().into_iter().collect();
    let targets = handler.path_requests.pending_targets(&destination, &active_interfaces);
    if targets.is_empty() {
        handler.path_requests.mark_dispatched(&destination);
        return;
    }
    for target in targets {
        if !handler.iface_manager.lock().await.can_egress_path_request_to(&target) {
            continue;
        }
        let dispatch =
            handler.send(TxMessage { tx_type: TxMessageType::Direct(target), packet }).await;
        if dispatch.sent_ifaces > 0 {
            handler.path_requests.mark_iface_dispatched(&destination, target);
        }
    }
    if handler.path_requests.pending_targets(&destination, &active_interfaces).is_empty() {
        handler.path_requests.mark_dispatched(&destination);
    } else {
        handler.path_requests.rotate_pending(&destination);
    }
}

pub(super) async fn send_to_next_hop<'a>(
    packet: &Packet,
    handler: &mut MutexGuard<'a, TransportHandler>,
    lookup: Option<AddressHash>,
) -> bool {
    let destination = lookup.unwrap_or(packet.destination);
    let (packet, maybe_iface) = handler.path_table.handle_inbound_packet(packet, lookup);

    if let Some(iface) = maybe_iface {
        let dispatch =
            handler.send(TxMessage { tx_type: TxMessageType::Direct(iface), packet }).await;
        if dispatch.sent_ifaces > 0 {
            handler.path_table.refresh(&destination);
            return true;
        }
    }

    false
}

pub(super) async fn handle_path_request<'a>(
    packet: &Packet,
    handler: &mut MutexGuard<'a, TransportHandler>,
    iface: AddressHash,
    ingress_limited: bool,
) {
    if let Ok(Some(request)) = handler.path_requests.decode(packet.data.as_slice()) {
        let ingress_limited = ingress_limited
            || handler.iface_manager.lock().await.classify_path_request_ingress(&iface);
        crate::transport_diagnostic!(
            "[tp] path_request dest={} iface={}",
            request.destination,
            iface
        );
        if let Some(dest) = handler.single_in_destinations.get(&request.destination) {
            let response = dest
                .lock()
                .await
                .path_response_with_tag(OsRng, None, Some(request.tag_bytes.as_slice()))
                .expect("valid path response");
            let active_interfaces: BTreeSet<_> =
                handler.iface_manager.lock().await.active_interface_hashes().into_iter().collect();
            let mut waiters =
                handler.path_requests.take_waiters(&request.destination, &active_interfaces);
            if active_interfaces.contains(&iface) {
                waiters.push(iface);
                waiters.sort();
                waiters.dedup();
            }
            for waiter in waiters {
                handler
                    .send(TxMessage { tx_type: TxMessageType::Direct(waiter), packet: response })
                    .await;
            }
            crate::transport_diagnostic!(
                "[tp] path_response dest={} iface={}",
                request.destination,
                iface
            );

            log::trace!("tp({}): send direct path response over {}", handler.config.name, iface);

            return;
        }

        if handler.config.retransmit
            && let Some(entry) = handler.path_table.get(&request.destination)
        {
            if let Some(requestor_id) = request.requesting_transport
                && requestor_id == entry.received_from
            {
                log::trace!(
                    "tp({}): dropping circular path request from {}",
                    handler.config.name,
                    request.destination
                );
                return;
            }

            let hops = entry.hops;
            let active_interfaces: BTreeSet<_> =
                handler.iface_manager.lock().await.active_interface_hashes().into_iter().collect();
            let mut waiters =
                handler.path_requests.take_waiters(&request.destination, &active_interfaces);
            if active_interfaces.contains(&iface) {
                waiters.push(iface);
                waiters.sort();
                waiters.dedup();
            }
            for waiter in waiters {
                handler.announce_table.add_response(request.destination, waiter, hops);
            }

            log::trace!(
                "tp({}): scheduled remote path response to {} ({} hops) over {}",
                handler.config.name,
                request.destination,
                hops,
                iface
            );

            return;
        }

        if handler.config.retransmit {
            let (active_interfaces, lowest_online_bitrate) = {
                let manager = handler.iface_manager.lock().await;
                (
                    manager.active_interface_hashes().into_iter().collect(),
                    manager.lowest_online_positive_bitrate(),
                )
            };
            handler.path_requests.register_discovery_with_bitrate(
                &request,
                iface,
                ingress_limited,
                &active_interfaces,
                lowest_online_bitrate,
            );
        }
    }
}

pub(super) async fn handle_fixed_destinations<'a>(
    packet: &Packet,
    handler: &mut MutexGuard<'a, TransportHandler>,
    iface: AddressHash,
    ingress_limited: bool,
) -> bool {
    if packet.destination == handler.fixed_dest_path_requests {
        handle_path_request(packet, handler, iface, ingress_limited).await;
        true
    } else {
        false
    }
}

pub(super) async fn handle_link_request_as_destination<'a>(
    destination: Arc<Mutex<SingleInputDestination>>,
    packet: &Packet,
    iface: AddressHash,
    mut handler: MutexGuard<'a, TransportHandler>,
) {
    let mut destination = destination.lock().await;
    match destination.handle_packet(packet) {
        DestinationHandleStatus::LinkProof => {
            let link_id = LinkId::from(packet);
            if !handler.in_links.contains_key(&link_id) {
                log::trace!("tp({}): send proof to {}", handler.config.name, packet.destination);

                let link = Link::new_from_request(
                    packet,
                    destination.sign_key().clone(),
                    destination.desc,
                    handler.link_in_event_tx.clone(),
                );

                if let Ok(mut link) = link {
                    link.set_ingress_iface(iface);
                    crate::transport_diagnostic!(
                        "[tp] link_proof_tx dst={} link_id={}",
                        packet.destination,
                        link.id()
                    );
                    // Link-request proofs must go back over the interface that delivered
                    // the request so multi-hop requestors can activate the link.
                    handler
                        .send(TxMessage {
                            tx_type: TxMessageType::Direct(iface),
                            packet: link.prove(),
                        })
                        .await;

                    log::debug!(
                        "tp({}): save input link {} for destination {}",
                        handler.config.name,
                        link.id(),
                        link.destination().address_hash
                    );

                    handler.in_links.insert(*link.id(), Arc::new(Mutex::new(link)));
                }
            }
        }
        DestinationHandleStatus::None => {}
    }
}

pub(super) async fn handle_link_request_as_intermediate<'a>(
    received_from: AddressHash,
    next_hop: AddressHash,
    next_hop_iface: AddressHash,
    remaining_hops: u8,
    outbound_bitrate: Option<u64>,
    packet: &Packet,
    mut handler: MutexGuard<'a, TransportHandler>,
) {
    handler.link_table.add(
        packet,
        packet.destination,
        received_from,
        next_hop,
        next_hop_iface,
        remaining_hops,
        outbound_bitrate,
    );

    send_to_next_hop(packet, &mut handler, None).await;
}

pub(super) async fn handle_link_request<'a>(
    packet: &Packet,
    iface: AddressHash,
    handler: MutexGuard<'a, TransportHandler>,
) {
    crate::transport_diagnostic!(
        "[tp] link_request dst={} ctx={:02x} hops={}",
        packet.destination,
        packet.context as u8,
        packet.header.hops
    );
    if let Some(destination) = handler.single_in_destinations.get(&packet.destination).cloned() {
        log::trace!("tp({}): handle link request for {}", handler.config.name, packet.destination);

        let ingress_mtu = handler.iface_manager.lock().await.online_link_mtu(&iface);
        let Ok(packet) =
            crate::transport::destination_ext::link::clamp_link_request_mtu(packet, ingress_mtu)
        else {
            return;
        };
        handle_link_request_as_destination(destination, &packet, iface, handler).await;
    } else if let Some(entry) = handler.path_table.next_hop_with_hops(&packet.destination) {
        log::trace!(
            "tp({}): handle link request for remote destination {}",
            handler.config.name,
            packet.destination
        );

        let (next_hop, next_iface, remaining_hops) = entry;
        let (outbound_bitrate, route_mtu) = {
            let manager = handler.iface_manager.lock().await;
            let ingress_mtu = manager.online_link_mtu(&iface);
            let outbound_mtu = manager.online_link_mtu(&next_iface);
            (
                manager.online_positive_bitrate(&next_iface),
                ingress_mtu.zip(outbound_mtu).map(|(ingress, outbound)| ingress.min(outbound)),
            )
        };
        let Ok(packet) =
            crate::transport::destination_ext::link::clamp_link_request_mtu(packet, route_mtu)
        else {
            return;
        };
        handle_link_request_as_intermediate(
            iface,
            next_hop,
            next_iface,
            remaining_hops,
            outbound_bitrate,
            &packet,
            handler,
        )
        .await;
    } else {
        log::trace!(
            "tp({}): dropping link request to unknown destination {}",
            handler.config.name,
            packet.destination
        );
    }
}
