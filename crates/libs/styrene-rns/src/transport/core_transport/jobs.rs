use super::announce::{handle_announce, retransmit_announces};
use super::path::{handle_fixed_destinations, handle_link_request};
use super::wire::{handle_data, handle_proof};
use super::*;
use crate::transport::destination_ext::link::{LinkCloseReason, LinkWatchdogAction};

pub(super) async fn handle_check_links<'a>(mut handler: MutexGuard<'a, TransportHandler>) {
    let mut links_to_remove: Vec<AddressHash> = Vec::new();
    let mut pending_packets: Vec<Packet> = Vec::new();
    let mut terminal_snapshots = Vec::new();

    // Clean up input links
    for link_entry in &handler.in_links {
        let mut link = link_entry.1.lock().await;
        match link.status() {
            LinkStatus::Closed => {
                terminal_snapshots.push(link.state_snapshot());
                links_to_remove.push(*link_entry.0);
            }
            LinkStatus::Pending | LinkStatus::Handshake => {
                if link.elapsed() > INTERVAL_INPUT_LINK_CLEANUP {
                    link.close_with_reason(LinkCloseReason::EstablishmentTimeout);
                    terminal_snapshots.push(link.state_snapshot());
                    links_to_remove.push(*link_entry.0);
                }
            }
            LinkStatus::Active | LinkStatus::Stale => {
                if link.check_watchdog(false) == LinkWatchdogAction::Close {
                    terminal_snapshots.push(link.state_snapshot());
                    links_to_remove.push(*link_entry.0);
                }
            }
        }
    }

    for snapshot in terminal_snapshots.drain(..) {
        handler.record_terminal_link(snapshot);
    }

    for addr in &links_to_remove {
        handler.in_links.remove(addr);
    }

    links_to_remove.clear();

    // Manage output links with RTT-driven watchdog
    for link_entry in &handler.out_links {
        let mut link = link_entry.1.lock().await;
        match link.status() {
            LinkStatus::Closed => {
                terminal_snapshots.push(link.state_snapshot());
                links_to_remove.push(*link_entry.0);
            }
            LinkStatus::Active | LinkStatus::Stale => match link.check_watchdog(true) {
                LinkWatchdogAction::SendKeepAlive => {
                    pending_packets.push(link.keep_alive_packet(KEEP_ALIVE_REQUEST));
                }
                LinkWatchdogAction::Close => {
                    terminal_snapshots.push(link.state_snapshot());
                    links_to_remove.push(*link_entry.0);
                }
                LinkWatchdogAction::None => {}
            },
            LinkStatus::Pending => {
                if link.elapsed() > INTERVAL_OUTPUT_LINK_REPEAT {
                    log::warn!("tp({}): repeat link request {}", handler.config.name, link.id());
                    pending_packets.push(link.request());
                }
            }
            LinkStatus::Handshake => {}
        }
    }

    for snapshot in terminal_snapshots {
        handler.record_terminal_link(snapshot);
    }

    for addr in &links_to_remove {
        handler.out_links.remove(addr);
    }

    for packet in pending_packets {
        handler.send_packet(packet).await;
    }
}

pub(super) async fn handle_cleanup<'a>(handler: MutexGuard<'a, TransportHandler>) {
    handler.iface_manager.lock().await.cleanup();
}

pub(super) async fn handle_cull_paths<'a>(mut handler: MutexGuard<'a, TransportHandler>) {
    let active_interfaces = handler.iface_manager.lock().await.active_interface_hashes();
    handler.link_table.remove_unavailable_interfaces(&active_interfaces);

    let out_links = handler
        .out_links
        .iter()
        .map(|(destination, link)| (*destination, link.clone()))
        .collect::<Vec<_>>();
    let mut removed_out = Vec::new();
    for (destination, link) in out_links {
        let mut link = link.lock().await;
        if link.ingress_iface().is_some_and(|iface| !active_interfaces.contains(&iface)) {
            link.close_with_reason(LinkCloseReason::SendFailure);
            handler.record_terminal_link(link.state_snapshot());
            removed_out.push(destination);
        }
    }
    for destination in removed_out {
        handler.out_links.remove(&destination);
    }

    let in_links =
        handler.in_links.iter().map(|(id, link)| (*id, link.clone())).collect::<Vec<_>>();
    let mut removed_in = Vec::new();
    for (id, link) in in_links {
        let mut link = link.lock().await;
        if link.ingress_iface().is_some_and(|iface| !active_interfaces.contains(&iface)) {
            link.close_with_reason(LinkCloseReason::SendFailure);
            handler.record_terminal_link(link.state_snapshot());
            removed_in.push(id);
        }
    }
    for id in removed_in {
        handler.in_links.remove(&id);
    }

    let events =
        handler.path_table.cull(std::time::Instant::now(), SystemTime::now(), &active_interfaces);
    for event in events {
        let _ = handler.route_tx.send(event);
    }
}

pub(super) fn should_rebroadcast(packet_type: PacketType) -> bool {
    matches!(packet_type, PacketType::Data)
}

async fn find_live_link(
    handler: &TransportHandler,
    link_id: AddressHash,
) -> Option<Arc<Mutex<Link>>> {
    let links =
        handler.in_links.values().chain(handler.out_links.values()).cloned().collect::<Vec<_>>();
    for link in links {
        let is_match = {
            let link = link.lock().await;
            *link.id() == link_id && matches!(link.status(), LinkStatus::Active | LinkStatus::Stale)
        };
        if is_match {
            return Some(link);
        }
    }
    None
}

pub(super) async fn handle_protocol_deadlines(mut handler: MutexGuard<'_, TransportHandler>) {
    let now = handler.protocol_clock.now();
    let timed_out = handler.request_tracker.timeout_due_ids();
    for request_id in timed_out {
        handler.request_tracker.timeout(request_id);
        super::links::cancel_correlated_request_resources(&mut handler, request_id).await;
    }
    let links =
        handler.in_links.values().chain(handler.out_links.values()).cloned().collect::<Vec<_>>();
    let mut live_link_ids = Vec::new();
    let mut messages = Vec::new();

    for link in links {
        let mut link = link.lock().await;
        if !matches!(link.status(), LinkStatus::Active | LinkStatus::Stale) {
            continue;
        }
        let link_id = *link.id();
        if live_link_ids.contains(&link_id) {
            continue;
        }
        live_link_ids.push(link_id);
        if let Some(iface) = link.ingress_iface()
            && link.next_channel_retry_at().is_some_and(|deadline| deadline <= now)
        {
            for packet in link.poll_channel_timeouts(now) {
                messages.push(TxMessage { tx_type: TxMessageType::Direct(iface), packet });
            }
        }
    }

    handler.resource_manager.remove_orphaned(&live_link_ids);
    let actions = handler.resource_manager.poll();

    for retry in actions.requests {
        if let Some(link) = find_live_link(&handler, retry.link_id).await {
            let link = link.lock().await;
            if let Some(iface) = link.ingress_iface() {
                messages.push(TxMessage {
                    tx_type: TxMessageType::Direct(iface),
                    packet: build_resource_request_packet(&link, &retry.request),
                });
            }
        }
    }
    for (link_id, packet) in actions.packets {
        if let Some(link) = find_live_link(&handler, link_id).await
            && let Some(iface) = link.lock().await.ingress_iface()
        {
            messages.push(TxMessage { tx_type: TxMessageType::Direct(iface), packet });
        }
    }
    for (link_id, proof_hash) in actions.proof_requests {
        if let Some(link) = find_live_link(&handler, link_id).await {
            let link = link.lock().await;
            if let Some(iface) = link.ingress_iface()
                && let Ok(packet) = build_resource_cache_request_packet(&link, proof_hash)
            {
                messages.push(TxMessage { tx_type: TxMessageType::Direct(iface), packet });
            }
        }
    }
    for cancellation in actions.cancellations {
        if let Some(link) = find_live_link(&handler, cancellation.link_id).await {
            let link = link.lock().await;
            if let Some(iface) = link.ingress_iface()
                && let Ok(packet) =
                    build_resource_cancel_packet(&link, cancellation.hash, cancellation.context)
            {
                messages.push(TxMessage { tx_type: TxMessageType::Direct(iface), packet });
            }
        }
    }

    for message in messages {
        handler.send(message).await;
    }
    let events = handler.resource_manager.drain_events();
    handler.publish_resource_events(events).await;
}

pub(super) async fn manage_transport(
    handler_arc: Arc<Mutex<TransportHandler>>,
    rx_receiver: Arc<Mutex<InterfaceRxReceiver>>,
    iface_messages_tx: broadcast::Sender<RxMessage>,
) {
    let cancel = handler_arc.lock().await.cancel.clone();
    let retransmit = handler_arc.lock().await.config.retransmit;

    let _packet_task = {
        let handler_arc = handler_arc.clone();
        let cancel = cancel.clone();

        log::trace!("tp({}): start packet task", handler_arc.lock().await.config.name);

        tokio::spawn(async move {
            loop {
                let mut rx_receiver = rx_receiver.lock().await;

                if cancel.is_cancelled() {
                    break;
                }

                tokio::select! {
                    _ = cancel.cancelled() => {
                        break;
                    },
                    Some(message) = rx_receiver.recv() => {
                        let Ok(message) = message.admit() else {
                            log::warn!("tp: dropping invalid packet before ingress state mutation");
                            continue;
                        };
                        let _ = iface_messages_tx.send(message);

                        let packet = message.packet;

                        let mut handler = handler_arc.lock().await;

                        // Record rx bytes for the originating interface.
                        handler
                            .iface_manager
                            .lock()
                            .await
                            .record_rx(&message.address, packet.data.len() as u64);

                        if PACKET_TRACE {
                            log::debug!("tp: << rx({}) = {} {}", message.address, packet, packet.hash());
                        }

                        if handle_fixed_destinations(
                            &packet,
                            &mut handler,
                            message.address
                        ).await {
                            continue;
                        }

                        if !handler.filter_duplicate_packets(&packet).await {
                            log::debug!(
                                "tp({}): dropping duplicate packet: dst={}, ctx={:?}, type={:?}",
                                handler.config.name,
                                packet.destination,
                                packet.context,
                                packet.header.packet_type
                            );
                            continue;
                        }

                        // Link requests and proofs have dedicated directed forwarding paths.
                        // Rebroadcasting them here as well creates a topology loop because
                        // their hop-mutated copies evade packet-hash deduplication.
                        if handler.config.broadcast
                            && should_rebroadcast(packet.header.packet_type)
                        {
                            handler
                                .send(TxMessage {
                                    tx_type: TxMessageType::Broadcast(Some(message.address)),
                                    packet,
                                })
                                .await;
                        }

                        match packet.header.packet_type {
                            PacketType::Announce => handle_announce(
                                &packet,
                                handler,
                                message.address
                            ).await,
                            PacketType::LinkRequest => handle_link_request(
                                &packet,
                                message.address,
                                handler
                            ).await,
                            PacketType::Proof => {
                                drop(handler);
                                handle_proof(packet, handler_arc.clone(), message.address).await;
                            }
                            PacketType::Data => handle_data(&packet, message.address, handler).await,
                        }
                    }
                };
            }
        })
    };

    {
        let handler = handler_arc.clone();
        let cancel = cancel.clone();

        tokio::spawn(async move {
            loop {
                if cancel.is_cancelled() {
                    break;
                }

                tokio::select! {
                    _ = cancel.cancelled() => {
                        break;
                    },
                    _ = time::sleep(INTERVAL_LINKS_CHECK) => {
                        handle_check_links(handler.lock().await).await;
                    }
                }
            }
        });
    }

    {
        let handler = handler_arc.clone();
        let cancel = cancel.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    _ = time::sleep(INTERVAL_PATH_CULL) => {
                        handle_cull_paths(handler.lock().await).await;
                    }
                }
            }
        });
    }

    {
        let handler = handler_arc.clone();
        let cancel = cancel.clone();

        tokio::spawn(async move {
            loop {
                if cancel.is_cancelled() {
                    break;
                }

                tokio::select! {
                    _ = cancel.cancelled() => {
                        break;
                    },
                    _ = time::sleep(INTERVAL_IFACE_CLEANUP) => {
                        handle_cleanup(handler.lock().await).await;
                    }
                }
            }
        });
    }

    {
        let handler = handler_arc.clone();
        let cancel = cancel.clone();

        tokio::spawn(async move {
            loop {
                if cancel.is_cancelled() {
                    break;
                }

                tokio::select! {
                    _ = cancel.cancelled() => {
                        break;
                    },
                    _ = time::sleep(INTERVAL_PACKET_CACHE_CLEANUP) => {
                        let mut handler = handler.lock().await;

                        handler
                            .packet_cache
                            .lock()
                            .await
                            .release(INTERVAL_KEEP_PACKET_CACHED);

                        handler.link_table.remove_stale();
                    },
                }
            }
        });
    }

    if retransmit {
        let handler = handler_arc.clone();
        let cancel = cancel.clone();

        tokio::spawn(async move {
            let mut last_old_retransmit = time::Instant::now();
            loop {
                if cancel.is_cancelled() {
                    break;
                }

                tokio::select! {
                    _ = cancel.cancelled() => {
                        break;
                    },
                    _ = time::sleep(INTERVAL_ANNOUNCES_RETRANSMIT) => {
                        let mut retransmit_old = false;
                        let now = time::Instant::now();
                        if now - last_old_retransmit > INTERVAL_OLD_ANNOUNCES_RETRANSMIT {
                            retransmit_old = true;
                            last_old_retransmit = now;
                        }
                        retransmit_announces(handler.lock().await, retransmit_old).await;
                    }
                }
            }
        });
    }

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = time::sleep(INTERVAL_PROTOCOL_SCHEDULER) => {
                handle_protocol_deadlines(handler_arc.lock().await).await;
            }
        }
    }
}
