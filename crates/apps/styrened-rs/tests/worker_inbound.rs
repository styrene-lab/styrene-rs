//! Integration test: inbound worker processes transport events through service layer.

use reticulum_daemon::services::{EventService, MessagingService, ProtocolService};
use reticulum_daemon::transport::mock_transport::MockTransport;
use reticulum_daemon::workers::inbound::spawn_inbound_worker;
use rns_core::hash::AddressHash;
use rns_core::transport::core_transport::{ReceivedData, ReceivedPayloadMode};
use std::sync::Arc;

fn build_lxmf_wire(destination: [u8; 16], source: [u8; 16], content: &str) -> Vec<u8> {
    let signature = [0x33u8; 64];
    let payload = rmp_serde::to_vec(&rmpv::Value::Array(vec![
        rmpv::Value::from(1_770_000_000_i64),
        rmpv::Value::from(""),
        rmpv::Value::from(content),
        rmpv::Value::Nil,
    ]))
    .expect("payload encoding");
    let mut wire = Vec::new();
    wire.extend_from_slice(&destination);
    wire.extend_from_slice(&source);
    wire.extend_from_slice(&signature);
    wire.extend_from_slice(&payload);
    wire
}

#[tokio::test]
async fn inbound_worker_decodes_and_persists_message() {
    let transport = Arc::new(MockTransport::new_default());
    let messaging = Arc::new(MessagingService::new());
    let protocol = Arc::new(ProtocolService::new());
    let events = Arc::new(EventService::new());

    let mut event_rx = events.subscribe();

    // Spawn worker
    let _handle = spawn_inbound_worker(
        transport.clone(),
        messaging.clone(),
        protocol.clone(),
        events.clone(),
    );

    // Give worker time to subscribe
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Inject a valid LXMF wire message
    let dest = [0x11u8; 16];
    let source = [0x22u8; 16];
    let wire_data = build_lxmf_wire(dest, source, "hello from mesh");

    transport.inject_inbound(ReceivedData {
        destination: AddressHash::new(dest),
        data: rns_core::packet::PacketDataBuffer::new_from_slice(&wire_data),
        payload_mode: ReceivedPayloadMode::FullWire,
        ratchet_used: false,
        context: None,
        request_id: None,
        hops: None,
        interface: None,
    });

    // Wait for worker to process
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Verify message was persisted
    let messages = messaging.list_messages(10, None).unwrap();
    assert_eq!(messages.len(), 1, "message should be persisted");
    assert_eq!(messages[0].content, "hello from mesh");
    assert_eq!(messages[0].direction, "in");
    assert_eq!(messages[0].source, hex::encode(source));

    // Verify event was emitted
    let event = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        event_rx.recv(),
    )
    .await
    .expect("should receive event")
    .expect("event");
    assert_eq!(event.event_type, "message_received");
}
