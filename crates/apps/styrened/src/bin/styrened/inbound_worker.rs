use super::bridge_helpers::{diagnostics_enabled, payload_preview};
use lxmf::inbound_decode::InboundPayloadMode;
use rns_core::transport::core_transport::{ReceivedPayloadMode, Transport};
use std::sync::Arc;
use styrened::inbound_delivery::{decode_inbound_payload, decode_inbound_payload_with_diagnostics};
use styrened::rpc::RpcDaemon;

fn inbound_payload_mode(mode: ReceivedPayloadMode) -> InboundPayloadMode {
    match mode {
        ReceivedPayloadMode::FullWire => InboundPayloadMode::FullWire,
        ReceivedPayloadMode::DestinationStripped => InboundPayloadMode::DestinationStripped,
    }
}

pub(super) fn spawn_inbound_worker(
    daemon: Arc<RpcDaemon>,
    transport: Arc<Transport>,
) -> tokio::task::JoinHandle<()> {
    let daemon_inbound = daemon;
    let inbound_transport = transport;
    tokio::spawn(async move {
        let mut rx = inbound_transport.received_data_events();
        loop {
            if let Ok(event) = rx.recv().await {
                let data = event.data.as_slice();
                let destination_hex = hex::encode(event.destination.as_slice());
                if diagnostics_enabled() {
                    eprintln!(
                        "[daemon-rx] dst={} len={} ratchet_used={} data_prefix={}",
                        destination_hex,
                        data.len(),
                        event.ratchet_used,
                        payload_preview(data, 16)
                    );
                } else {
                    eprintln!("[daemon] rx data len={} dst={}", data.len(), destination_hex);
                }
                let mut destination = [0u8; 16];
                destination.copy_from_slice(event.destination.as_slice());
                let payload_mode = inbound_payload_mode(event.payload_mode);
                let record = if diagnostics_enabled() {
                    let (record, diagnostics) =
                        decode_inbound_payload_with_diagnostics(destination, data, payload_mode);
                    if let Some(ref decoded) = record {
                        eprintln!(
                            "[daemon-rx] decoded msg_id={} src={} dst={} title_len={} content_len={}",
                            decoded.id,
                            decoded.source,
                            decoded.destination,
                            decoded.title.len(),
                            decoded.content.len()
                        );
                    } else {
                        eprintln!(
                            "[daemon-rx] decode-failed dst={} attempts={}",
                            destination_hex,
                            diagnostics.summary()
                        );
                    }
                    record
                } else {
                    decode_inbound_payload(destination, data, payload_mode)
                };
                if let Some(record) = record {
                    match daemon_inbound.accept_inbound(record) {
                        Ok(true) => {}
                        Ok(false) => {}
                        Err(error) => {
                            daemon_inbound.emit_event(styrened::rpc::RpcEvent {
                                event_type: "inbound_dropped".into(),
                                payload: serde_json::json!({
                                    "path": "legacy_direct_packet",
                                    "reason": "storage_error",
                                    "destination": destination_hex,
                                    "detail": error.to_string(),
                                }),
                            });
                        }
                    }
                } else {
                    daemon_inbound.emit_event(styrened::rpc::RpcEvent {
                        event_type: "inbound_dropped".into(),
                        payload: serde_json::json!({
                            "path": "legacy_direct_packet",
                            "reason": "malformed",
                            "destination": destination_hex,
                        }),
                    });
                }
            }
        }
    })
}
