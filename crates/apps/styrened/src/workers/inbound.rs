//! Inbound message worker — subscribes to transport inbound events,
//! decodes LXMF wire, persists via MessagingService, routes through
//! ProtocolService, and emits DaemonEvents.
//!
//! In hub mode with propagation enabled, messages for non-local destinations
//! are stored for later retrieval rather than decoded locally.

use crate::services::{EventService, MessagingService, PropagationService, ProtocolService};
use crate::transport::mesh_transport::MeshTransport;
use lxmf::inbound_decode::InboundPayloadMode;
use rns_core::transport::core_transport::ReceivedPayloadMode;
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
    let mut rx = transport.subscribe_inbound();

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
