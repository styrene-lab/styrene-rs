//! Inbound message worker — subscribes to transport inbound events,
//! decodes LXMF wire, persists via MessagingService, routes through
//! ProtocolService, and emits DaemonEvents.
//!
//! In hub mode with propagation enabled, messages for non-local destinations
//! are stored for later retrieval rather than decoded locally.

use crate::services::messaging::InboundAcceptOutcome;
use crate::services::{
    AutoReplyService, EventService, MessagingService, PropagationService, ProtocolService,
};
use crate::transport::mesh_transport::MeshTransport;
use lxmf::inbound_decode::InboundPayloadMode;
use rns_core::transport::core_transport::ReceivedPayloadMode;
use rns_core::transport::resource::{ResourceEventKind, ResourceFailure};
use std::sync::Arc;
use tokio::task::JoinHandle;

/// Tasks owned by the inbound worker.
///
/// The resource task is separate because large LXMF payloads arrive through a
/// different transport subscription than packet-sized messages.
pub struct InboundWorkerHandle {
    packet: JoinHandle<()>,
    resource: JoinHandle<()>,
}

pub struct InboundDestinations {
    local_delivery_hash: Option<String>,
    excluded_propagation_destination: Option<String>,
}

impl InboundDestinations {
    pub fn new(
        local_delivery_hash: Option<String>,
        excluded_propagation_destination: Option<String>,
    ) -> Self {
        Self { local_delivery_hash, excluded_propagation_destination }
    }
}

impl InboundWorkerHandle {
    pub fn abort(&self) {
        self.packet.abort();
        self.resource.abort();
    }

    pub fn is_finished(&self) -> bool {
        self.packet.is_finished() && self.resource.is_finished()
    }

    pub async fn wait(&mut self) {
        let _ = (&mut self.packet).await;
        let _ = (&mut self.resource).await;
    }

    #[cfg(test)]
    pub(crate) fn abort_handles(&self) -> [tokio::task::AbortHandle; 2] {
        [self.packet.abort_handle(), self.resource.abort_handle()]
    }
}

fn to_lxmf_mode(mode: ReceivedPayloadMode) -> InboundPayloadMode {
    match mode {
        ReceivedPayloadMode::FullWire => InboundPayloadMode::FullWire,
        ReceivedPayloadMode::DestinationStripped => InboundPayloadMode::DestinationStripped,
    }
}

async fn resolve_sender_identity(
    transport: &dyn MeshTransport,
    data: &[u8],
    mode: InboundPayloadMode,
) -> (Option<[u8; 16]>, Option<rns_core::identity::Identity>) {
    let start = match mode {
        InboundPayloadMode::FullWire => 16,
        InboundPayloadMode::DestinationStripped => 0,
    };
    let source = data.get(start..start + 16).and_then(|value| value.try_into().ok());
    let identity = if let Some(source) = source {
        transport.resolve_identity(&rns_core::hash::AddressHash::new(source)).await
    } else {
        None
    };
    (source, identity)
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
) -> InboundWorkerHandle {
    spawn_inbound_worker_with_auto_reply(
        transport,
        messaging,
        protocol,
        events,
        propagation,
        InboundDestinations::new(local_delivery_hash, None),
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
    destinations: InboundDestinations,
    auto_reply: Option<Arc<AutoReplyService>>,
) -> InboundWorkerHandle {
    let InboundDestinations { local_delivery_hash, excluded_propagation_destination } =
        destinations;
    let mut rx = transport.subscribe_inbound();

    // Spawn a resource event handler that processes completed resource transfers.
    // Large payloads (> LINK_PACKET_MDU) are sent as RNS resources and arrive
    // via the resource_events channel rather than the inbound data channel.
    let resource = {
        let mut resource_rx = transport.subscribe_resources();
        let messaging = messaging.clone();
        let events = events.clone();
        let protocol = protocol.clone();
        let transport = transport.clone();
        let local_delivery_hash = local_delivery_hash.clone();
        tokio::spawn(async move {
            loop {
                match resource_rx.recv().await {
                    Ok(event) => {
                        events.emit_resource_event(&event);
                        if let ResourceEventKind::Progress(progress) = &event.kind {
                            if let Err(error) = messaging.handle_resource_progress(
                                &event.hash.to_bytes(),
                                progress.received_bytes,
                                progress.total_bytes,
                            ) {
                                crate::daemon_diagnostic!(
                                    "[worker] attachment resource progress correlation error: {error}"
                                );
                            }
                            continue;
                        }
                        if matches!(event.kind, ResourceEventKind::OutboundComplete) {
                            if let Err(error) = messaging
                                .handle_resource_complete(&hex::encode(event.hash.to_bytes()))
                            {
                                crate::daemon_diagnostic!(
                                    "[worker] outbound resource completion error: {error}"
                                );
                            }
                            continue;
                        }
                        if let ResourceEventKind::Failed(failure) = event.kind {
                            messaging.forget_pending_resource(&event.hash.to_bytes());
                            let hash = hex::encode(event.hash.to_bytes());
                            let result = match failure {
                                ResourceFailure::Cancelled => {
                                    messaging.handle_resource_cancelled(&hash)
                                }
                                ResourceFailure::TimedOut => {
                                    messaging.handle_resource_failure(&hash, "timeout")
                                }
                                ResourceFailure::LinkClosed => {
                                    messaging.handle_resource_failure(&hash, "link-closed")
                                }
                                ResourceFailure::Integrity => {
                                    messaging.handle_resource_failure(&hash, "integrity")
                                }
                            };
                            if let Err(error) = result {
                                crate::daemon_diagnostic!(
                                    "[worker] outbound resource failure correlation error: {error}"
                                );
                            }
                            continue;
                        }
                        if let ResourceEventKind::Complete(complete) = event.kind {
                            let data = &complete.data;
                            crate::daemon_diagnostic!(
                                "[worker] resource complete: len={} link={}",
                                data.len(),
                                event.link_id
                            );

                            // Resource data is the full LXMF wire payload.
                            // Determine destination from the first 16 bytes.
                            if data.len() < 32 {
                                messaging.forget_pending_resource(&event.hash.to_bytes());
                                crate::daemon_diagnostic!("[worker] resource too short to decode");
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
                                messaging.forget_pending_resource(&event.hash.to_bytes());
                                continue; // not for us
                            }

                            let (source, sender_identity) =
                                resolve_sender_identity(transport.as_ref(), data, payload_mode)
                                    .await;
                            if let (Some(source), Some(identity)) =
                                (source, sender_identity.as_ref())
                                && let Err(error) =
                                    messaging.revalidate_unknown_identity(source, identity)
                            {
                                crate::daemon_diagnostic!(
                                    "[worker] deferred resource LXMF authentication failed: {error}"
                                );
                            }
                            match messaging.accept_inbound_resource_with_identity(
                                destination,
                                data,
                                payload_mode,
                                sender_identity.as_ref(),
                                crate::storage::messages::InboundAttachmentTransferEvidence {
                                    resource_hash: event.hash.to_bytes(),
                                    transferred: complete.transfer_size,
                                    total: complete.transfer_size,
                                    checksum_verified: complete.checksum_verified,
                                },
                            ) {
                                InboundAcceptOutcome::Accepted(record) => {
                                    crate::daemon_diagnostic!(
                                        "[worker] resource message: id={} src={} content_len={}",
                                        record.id,
                                        record.source,
                                        record.content.len()
                                    );
                                    let canonical = match messaging.canonical_inbound(&record.id) {
                                        Ok(Some(canonical)) => canonical,
                                        Ok(None) => {
                                            events.emit_reconciliation_required(
                                                "accepted resource message missing canonical record",
                                            );
                                            continue;
                                        }
                                        Err(error) => {
                                            crate::daemon_diagnostic!(
                                                "[worker] canonical resource event projection failed: {error}"
                                            );
                                            events.emit_reconciliation_required(
                                                "canonical resource event projection failed",
                                            );
                                            continue;
                                        }
                                    };
                                    events.emit_message_new(&record, Some(&canonical));
                                    if messaging
                                        .inbound_is_dispatchable(&record.id)
                                        .unwrap_or(false)
                                    {
                                        protocol.dispatch_inbound(&record).await;
                                    } else {
                                        events.emit_inbound_drop(
                                            "direct_resource",
                                            "authentication_or_stamp_untrusted",
                                            Some(&record.id),
                                            Some(&record.destination),
                                            None,
                                        );
                                    }
                                }
                                InboundAcceptOutcome::Duplicate { message_id } => {
                                    events.emit_inbound_drop(
                                        "direct_resource",
                                        "duplicate",
                                        Some(&message_id),
                                        Some(&dest_hex),
                                        None,
                                    );
                                }
                                InboundAcceptOutcome::Rejected { diagnostics } => {
                                    events.emit_inbound_drop(
                                        "direct_resource",
                                        "malformed",
                                        None,
                                        Some(&dest_hex),
                                        Some(&diagnostics.summary()),
                                    );
                                }
                                InboundAcceptOutcome::StorageError { message_id, error } => {
                                    events.emit_inbound_drop(
                                        "direct_resource",
                                        "storage_error",
                                        Some(&message_id),
                                        Some(&dest_hex),
                                        Some(&error.to_string()),
                                    );
                                }
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        crate::daemon_diagnostic!("[worker] resource worker lagged, skipped {n}");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        })
    };

    let packet = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let data = event.data.as_slice();
                    let mut destination = [0u8; 16];
                    destination.copy_from_slice(event.destination.as_slice());
                    let dest_hex = hex::encode(destination);
                    if excluded_propagation_destination.as_deref() == Some(dest_hex.as_str()) {
                        continue;
                    }
                    let payload_mode = to_lxmf_mode(event.payload_mode);

                    crate::daemon_diagnostic!(
                        "[worker] inbound: dst={} len={} mode={:?}",
                        dest_hex,
                        data.len(),
                        payload_mode
                    );

                    crate::daemon_diagnostic!(
                        "[messaging-flow] stage=inbound_event destination={} bytes={} mode={:?}",
                        dest_hex,
                        data.len(),
                        payload_mode
                    );

                    // Hub propagation: if destination is not local, store for later delivery
                    let is_local =
                        local_delivery_hash.as_ref().is_some_and(|local| *local == dest_hex);

                    if !is_local {
                        crate::daemon_diagnostic!(
                            "[worker] non-local message: dest={} local={:?}",
                            dest_hex,
                            local_delivery_hash.as_deref().unwrap_or("none")
                        );
                    }

                    if !is_local && propagation.is_enabled() {
                        match propagation.store_for_propagation(&dest_hex, data, None) {
                            Ok(true) => {
                                crate::daemon_diagnostic!(
                                    "[worker] propagation: stored message for dst={} ({} bytes)",
                                    dest_hex,
                                    data.len()
                                );
                            }
                            Ok(false) => {
                                crate::daemon_diagnostic!(
                                    "[worker] propagation: duplicate for dst={}",
                                    dest_hex
                                );
                                events.emit_inbound_drop(
                                    "propagation_store",
                                    "duplicate",
                                    None,
                                    Some(&dest_hex),
                                    None,
                                );
                            }
                            Err(e) => {
                                crate::daemon_diagnostic!(
                                    "[worker] propagation: store error for dst={}: {e}",
                                    dest_hex
                                );
                                events.emit_inbound_drop(
                                    "propagation_store",
                                    "storage_error",
                                    None,
                                    Some(&dest_hex),
                                    Some(&e.to_string()),
                                );
                            }
                        }
                        // Non-local message stored for propagation — skip local delivery
                        continue;
                    }

                    let (source, sender_identity) =
                        resolve_sender_identity(transport.as_ref(), data, payload_mode).await;
                    if let (Some(source), Some(identity)) = (source, sender_identity.as_ref())
                        && let Err(error) = messaging.revalidate_unknown_identity(source, identity)
                    {
                        crate::daemon_diagnostic!(
                            "[worker] deferred LXMF authentication failed: {error}"
                        );
                    }

                    // Local delivery: decode and persist exactly once.
                    match messaging.accept_inbound_with_identity(
                        destination,
                        data,
                        payload_mode,
                        sender_identity.as_ref(),
                    ) {
                        InboundAcceptOutcome::Accepted(record) => {
                            crate::daemon_diagnostic!(
                                "[messaging-flow] stage=durable_insert id={} source={} destination={} bytes={}",
                                record.id,
                                record.source,
                                record.destination,
                                data.len()
                            );
                            crate::daemon_diagnostic!(
                                "[worker] inbound message: id={} src={} content_len={}",
                                record.id,
                                record.source,
                                record.content.len()
                            );

                            // Emit event for IPC subscribers
                            let canonical = match messaging.canonical_inbound(&record.id) {
                                Ok(Some(canonical)) => canonical,
                                Ok(None) => {
                                    events.emit_reconciliation_required(
                                        "accepted packet message missing canonical record",
                                    );
                                    continue;
                                }
                                Err(error) => {
                                    crate::daemon_diagnostic!(
                                        "[worker] canonical packet event projection failed: {error}"
                                    );
                                    events.emit_reconciliation_required(
                                        "canonical packet event projection failed",
                                    );
                                    continue;
                                }
                            };
                            events.emit_message_new(&record, Some(&canonical));

                            let trusted =
                                messaging.inbound_is_dispatchable(&record.id).unwrap_or(false);
                            if !trusted {
                                events.emit_inbound_drop(
                                    "direct_packet",
                                    "authentication_or_stamp_untrusted",
                                    Some(&record.id),
                                    Some(&record.destination),
                                    None,
                                );
                                continue;
                            }

                            // Route only authenticated and stamp-policy-compliant messages.
                            protocol.dispatch_inbound(&record).await;

                            // Auto-reply: if enabled and cooldown permits, send reply.
                            // Skip auto-reply for:
                            // - Protocol messages (e.g., Fleet RPC with fields.protocol set)
                            // - Messages that are themselves auto-replies (title contains "[auto-reply]")
                            let is_protocol_message =
                                record.fields.as_ref().and_then(|f| f.get("protocol")).is_some();
                            let is_auto_reply_message = record.title.contains("[auto-reply]");
                            let should_auto_reply = !is_protocol_message && !is_auto_reply_message;

                            if should_auto_reply
                                && let Some(ref ar) = auto_reply
                                && let Some(reply_text) = ar.should_reply(&record.source)
                            {
                                // Determine the sender's delivery hash for reply routing.
                                // The record.source is the sender's identity hash; we need
                                // their delivery destination hash for send_chat.
                                // The inbound worker knows the delivery hash is what the
                                // transport resolved — look it up from the source identity.
                                let source_delivery_hash =
                                    (record.source.len() == 32).then(|| record.source.clone());

                                if let Some(dest_hash) = source_delivery_hash {
                                    let m = messaging.clone();
                                    tokio::spawn(async move {
                                        if let Err(e) = m
                                            .send_chat(
                                                &dest_hash,
                                                &reply_text,
                                                Some("[auto-reply]"),
                                            )
                                            .await
                                        {
                                            crate::daemon_diagnostic!(
                                                "[worker] auto-reply failed: {e}"
                                            );
                                        } else {
                                            crate::daemon_diagnostic!(
                                                "[worker] auto-reply sent to {}",
                                                dest_hash
                                            );
                                        }
                                    });
                                }
                            } // close should_auto_reply
                        }
                        InboundAcceptOutcome::Duplicate { message_id } => {
                            events.emit_inbound_drop(
                                "direct_packet",
                                "duplicate",
                                Some(&message_id),
                                Some(&dest_hex),
                                None,
                            );
                        }
                        InboundAcceptOutcome::Rejected { diagnostics } => {
                            events.emit_inbound_drop(
                                "direct_packet",
                                "malformed",
                                None,
                                Some(&dest_hex),
                                Some(&diagnostics.summary()),
                            );
                        }
                        InboundAcceptOutcome::StorageError { message_id, error } => {
                            crate::daemon_diagnostic!(
                                "[messaging-flow] stage=durable_insert_failed id={} destination={} error={}",
                                message_id,
                                dest_hex,
                                error
                            );
                            events.emit_inbound_drop(
                                "direct_packet",
                                "storage_error",
                                Some(&message_id),
                                Some(&dest_hex),
                                Some(&error.to_string()),
                            );
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    crate::daemon_diagnostic!("[worker] inbound worker lagged, skipped {n} events");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    crate::daemon_diagnostic!("[worker] inbound channel closed, worker stopping");
                    break;
                }
            }
        }
    });

    InboundWorkerHandle { packet, resource }
}
