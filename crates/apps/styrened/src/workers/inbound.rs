//! Inbound message worker — subscribes to transport inbound events,
//! decodes LXMF wire, persists via MessagingService, routes through
//! ProtocolService, and emits DaemonEvents.
//!
//! In hub mode with propagation enabled, messages for non-local destinations
//! are stored for later retrieval rather than decoded locally.

use crate::services::{
    AutoReplyService, EventService, MessagingService, PropagationService, ProtocolService,
};
use crate::transport::mesh_transport::MeshTransport;
use lxmf::inbound_decode::InboundPayloadMode;
use rns_core::transport::core_transport::ReceivedPayloadMode;
use rns_core::transport::resource::ResourceEventKind;
use std::sync::Arc;
use tokio::task::JoinHandle;

fn to_lxmf_mode(mode: ReceivedPayloadMode) -> InboundPayloadMode {
    match mode {
        ReceivedPayloadMode::FullWire => InboundPayloadMode::FullWire,
        ReceivedPayloadMode::DestinationStripped => InboundPayloadMode::DestinationStripped,
    }
}

/// Spawn the inbound message processing worker.
///
/// Subscribes to transport inbound data events and:
/// 1. If propagation is enabled and destination is not local → store for propagation
/// 2. Otherwise: decode LXMF wire → MessageRecord → persist → protocol dispatch → emit event
pub fn spawn_inbound_worker(
    transport: Arc<dyn MeshTransport>,
    messaging: Arc<MessagingService>,
    protocol: Arc<ProtocolService>,
    events: Arc<EventService>,
    propagation: Arc<PropagationService>,
    local_delivery_hash: Option<String>,
) -> JoinHandle<()> {
    spawn_inbound_worker_with_auto_reply(
        transport,
        messaging,
        protocol,
        events,
        propagation,
        local_delivery_hash,
        None,
    )
}

/// Spawn the inbound worker with optional auto-reply support.
pub fn spawn_inbound_worker_with_auto_reply(
    transport: Arc<dyn MeshTransport>,
    messaging: Arc<MessagingService>,
    protocol: Arc<ProtocolService>,
    events: Arc<EventService>,
    propagation: Arc<PropagationService>,
    local_delivery_hash: Option<String>,
    auto_reply: Option<Arc<AutoReplyService>>,
) -> JoinHandle<()> {
    let mut rx = transport.subscribe_inbound();

    // Spawn a resource event handler that processes completed resource transfers.
    // Large payloads (> LINK_PACKET_MDU) are sent as RNS resources and arrive
    // via the resource_events channel rather than the inbound data channel.
    {
        let mut resource_rx = transport.subscribe_resources();
        let messaging = messaging.clone();
        let events = events.clone();
        let protocol = protocol.clone();
        let local_delivery_hash = local_delivery_hash.clone();
        tokio::spawn(async move {
            loop {
                match resource_rx.recv().await {
                    Ok(event) => {
                        if let ResourceEventKind::Complete(complete) = event.kind {
                            let data = &complete.data;
                            eprintln!(
                                "[worker] resource complete: len={} link={}",
                                data.len(),
                                event.link_id
                            );

                            // Resource data is the full LXMF wire payload.
                            // Determine destination from the first 16 bytes.
                            if data.len() < 32 {
                                eprintln!("[worker] resource too short to decode");
                                continue;
                            }
                            let mut destination = [0u8; 16];
                            destination.copy_from_slice(&data[..16]);
                            let dest_hex = hex::encode(destination);
                            let payload_mode = InboundPayloadMode::FullWire;

                            let is_local = local_delivery_hash
                                .as_ref()
                                .is_some_and(|local| *local == dest_hex);

                            if !is_local {
                                continue; // not for us
                            }

                            if let Some(record) =
                                messaging.accept_inbound(destination, data, payload_mode)
                            {
                                eprintln!(
                                    "[worker] resource message: id={} src={} content_len={}",
                                    record.id,
                                    record.source,
                                    record.content.len()
                                );
                                protocol.dispatch_inbound(&record).await;
                                events.emit_message_new(&record);
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        eprintln!("[worker] resource worker lagged, skipped {n}");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let data = event.data.as_slice();
                    let mut destination = [0u8; 16];
                    destination.copy_from_slice(event.destination.as_slice());
                    let dest_hex = hex::encode(destination);
                    let payload_mode = to_lxmf_mode(event.payload_mode);

                    eprintln!(
                        "[worker] inbound: dst={} len={} mode={:?}",
                        dest_hex,
                        data.len(),
                        payload_mode
                    );

                    // Hub propagation: if destination is not local, store for later delivery
                    let is_local =
                        local_delivery_hash.as_ref().is_some_and(|local| *local == dest_hex);

                    if !is_local {
                        eprintln!(
                            "[worker] non-local message: dest={} local={:?}",
                            dest_hex,
                            local_delivery_hash.as_deref().unwrap_or("none")
                        );
                    }

                    if !is_local && propagation.is_enabled() {
                        match propagation.store_for_propagation(&dest_hex, data, None) {
                            Ok(true) => {
                                eprintln!(
                                    "[worker] propagation: stored message for dst={} ({} bytes)",
                                    dest_hex,
                                    data.len()
                                );
                            }
                            Ok(false) => {
                                eprintln!("[worker] propagation: duplicate for dst={}", dest_hex);
                            }
                            Err(e) => {
                                eprintln!(
                                    "[worker] propagation: store error for dst={}: {e}",
                                    dest_hex
                                );
                            }
                        }
                        // Non-local message stored for propagation — skip local delivery
                        continue;
                    }

                    // Signature verification: look up the sender's identity
                    // from the transport announce table and verify the LXMF
                    // Ed25519 signature before accepting the message.
                    // Messages with invalid signatures are logged and dropped.
                    //
                    // Note: the sender's identity must have been received via
                    // a verified announce (announce signatures ARE checked by
                    // the RNS transport layer). If the sender hasn't announced,
                    // we can't verify — accept with a warning for now, since
                    // the link handshake provides transport-layer authentication.
                    {
                        // Extract source hash from the wire to look up identity
                        const SIG_OFFSET: usize = 32; // dest(16) + source(16)
                        if data.len() >= SIG_OFFSET {
                            let mut source_bytes = [0u8; 16];
                            let source_start = match payload_mode {
                                InboundPayloadMode::FullWire => 16, // after dest
                                InboundPayloadMode::DestinationStripped => 0, // source is first
                            };
                            if data.len() > source_start + 16 {
                                source_bytes.copy_from_slice(&data[source_start..source_start + 16]);
                                // Compute the sender's delivery destination hash for identity lookup
                                let sender_delivery_hash = {
                                    let name = rns_core::destination::DestinationName::new("lxmf", "delivery");
                                    rns_core::hash::AddressHash::new(
                                        rns_core::hash::address_hash(
                                            &[name.as_name_hash_slice(), &source_bytes].concat(),
                                        ),
                                    )
                                };
                                if let Some(sender_identity) = transport.resolve_identity(&sender_delivery_hash).await {
                                    let verified = crate::inbound_delivery::verify_inbound_signature(
                                        data,
                                        payload_mode,
                                        destination,
                                        &sender_identity,
                                    );
                                    match verified {
                                        Some(true) => {} // signature valid
                                        Some(false) => {
                                            eprintln!(
                                                "[worker] REJECTED inbound message: invalid signature from {}",
                                                hex::encode(source_bytes)
                                            );
                                            continue;
                                        }
                                        None => {
                                            eprintln!(
                                                "[worker] WARNING: could not parse wire for signature verification from {}",
                                                hex::encode(source_bytes)
                                            );
                                            // Accept anyway — wire format issue, not forgery
                                        }
                                    }
                                }
                                // If sender identity not found: accept with implicit trust
                                // from the link-layer authentication
                            }
                        }
                    }

                    // Local delivery: decode and persist
                    if let Some(record) = messaging.accept_inbound(destination, data, payload_mode)
                    {
                        eprintln!(
                            "[worker] inbound message: id={} src={} content_len={}",
                            record.id,
                            record.source,
                            record.content.len()
                        );

                        // Route through protocol dispatch (async)
                        protocol.dispatch_inbound(&record).await;

                        // Emit event for IPC subscribers
                        events.emit_message_new(&record);

                        // Auto-reply: if enabled and cooldown permits, send reply.
                        // Skip auto-reply for:
                        // - Protocol messages (e.g., Fleet RPC with fields.protocol set)
                        // - Messages that are themselves auto-replies (title contains "[auto-reply]")
                        let is_protocol_message = record
                            .fields
                            .as_ref()
                            .and_then(|f| f.get("protocol"))
                            .is_some();
                        let is_auto_reply_message = record.title.contains("[auto-reply]");
                        let should_auto_reply =
                            !is_protocol_message && !is_auto_reply_message;

                        if should_auto_reply {
                        if let Some(ref ar) = auto_reply {
                            if let Some(reply_text) = ar.should_reply(&record.source) {
                                // Determine the sender's delivery hash for reply routing.
                                // The record.source is the sender's identity hash; we need
                                // their delivery destination hash for send_chat.
                                // The inbound worker knows the delivery hash is what the
                                // transport resolved — look it up from the source identity.
                                let source_delivery_hash = {
                                    let name = rns_core::destination::DestinationName::new("lxmf", "delivery");
                                    let source_bytes: Result<[u8; 16], _> = hex::decode(&record.source)
                                        .and_then(|b| b.try_into().map_err(|_| hex::FromHexError::InvalidStringLength));
                                    source_bytes.ok().map(|id_bytes| {
                                        let truncated = rns_core::hash::address_hash(
                                            &[name.as_name_hash_slice(), &id_bytes].concat(),
                                        );
                                        hex::encode(truncated)
                                    })
                                };

                                if let Some(dest_hash) = source_delivery_hash {
                                    let m = messaging.clone();
                                    tokio::spawn(async move {
                                        if let Err(e) = m.send_chat(&dest_hash, &reply_text, Some("[auto-reply]")).await {
                                            eprintln!("[worker] auto-reply failed: {e}");
                                        } else {
                                            eprintln!("[worker] auto-reply sent to {}", dest_hash);
                                        }
                                    });
                                }
                            }
                        }
                        } // close should_auto_reply
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    eprintln!("[worker] inbound worker lagged, skipped {n} events");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    eprintln!("[worker] inbound channel closed, worker stopping");
                    break;
                }
            }
        }
    })
}
