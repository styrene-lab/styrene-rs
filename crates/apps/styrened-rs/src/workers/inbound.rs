//! Inbound message worker — subscribes to transport inbound events,
//! decodes LXMF wire, persists via MessagingService, routes through
//! ProtocolService, and emits DaemonEvents.

use crate::services::{EventService, MessagingService, ProtocolService};
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
/// 1. Decodes LXMF wire format → MessageRecord
/// 2. Persists via MessagingService
/// 3. Routes through ProtocolService for protocol-specific handling
/// 4. Emits DaemonEvent::Message via EventService
pub fn spawn_inbound_worker(
    transport: Arc<dyn MeshTransport>,
    messaging: Arc<MessagingService>,
    protocol: Arc<ProtocolService>,
    events: Arc<EventService>,
) -> JoinHandle<()> {
    let mut rx = transport.subscribe_inbound();

    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let data = event.data.as_slice();
                    let mut destination = [0u8; 16];
                    destination.copy_from_slice(event.destination.as_slice());
                    let payload_mode = to_lxmf_mode(event.payload_mode);

                    eprintln!(
                        "[worker] inbound: dst={} len={} mode={:?}",
                        hex::encode(destination),
                        data.len(),
                        payload_mode
                    );

                    // Decode and persist
                    if let Some(record) = messaging.accept_inbound(
                        destination, data, payload_mode,
                    ) {
                        eprintln!(
                            "[worker] inbound message: id={} src={} content_len={}",
                            record.id,
                            record.source,
                            record.content.len()
                        );

                        // Route through protocol dispatch
                        protocol.dispatch_inbound(&record);

                        // Emit event for IPC subscribers
                        events.emit_message_new(&record);
                    } else {
                        eprintln!(
                            "[worker] inbound decode failed: dst={} len={}",
                            hex::encode(destination),
                            data.len()
                        );
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
