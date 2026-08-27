//! Integration test: inbound worker processes transport events through service layer.

use rns_core::hash::AddressHash;
use rns_core::hash::Hash;
use rns_core::transport::core_transport::{ReceivedData, ReceivedPayloadMode};
use rns_core::transport::resource::{ResourceEvent, ResourceEventKind, ResourceFailure};
use std::sync::Arc;
use styrened::services::{EventService, MessagingService, PropagationService, ProtocolService};
use styrened::storage::messages::MessagesStore;
use styrened::storage::messages::{MessageRecord, OutboundAttemptRecord, OutboundRouteRecord};
use styrened::transport::mock_transport::MockTransport;
use styrened::workers::inbound::{
    spawn_inbound_worker, spawn_inbound_worker_with_auto_reply, InboundDestinations,
};

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
async fn standard_propagation_packet_is_explicitly_excluded_from_generic_store() {
    let transport = Arc::new(MockTransport::new_default());
    let propagation_hash = [0x71; 16];
    let store = Arc::new(std::sync::Mutex::new(MessagesStore::in_memory().unwrap()));
    let propagation = Arc::new(PropagationService::new(store));
    propagation.set_enabled(true);
    let mut handle = spawn_inbound_worker_with_auto_reply(
        transport.clone(),
        Arc::new(MessagingService::new()),
        Arc::new(ProtocolService::new()),
        Arc::new(EventService::new()),
        propagation.clone(),
        InboundDestinations::new(
            Some(hex::encode([0x72; 16])),
            Some(hex::encode(propagation_hash)),
        ),
        None,
    );
    tokio::task::yield_now().await;
    transport.inject_inbound(ReceivedData {
        destination: AddressHash::new(propagation_hash),
        link_id: Some(AddressHash::new([0x73; 16])),
        data: rns_core::packet::PacketDataBuffer::new_from_slice(&[0x92, 0, 0x90]),
        payload_mode: ReceivedPayloadMode::DestinationStripped,
        ratchet_used: false,
        context: None,
        request_id: None,
        hops: None,
        interface: None,
    });
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    assert_eq!(propagation.stats().unwrap(), (0, 0));
    handle.abort();
    handle.wait().await;
}

#[tokio::test]
async fn duplicate_unknown_identity_is_stored_once_with_trust_and_duplicate_drops() {
    let transport = Arc::new(MockTransport::new_default());
    let messaging = Arc::new(MessagingService::new());
    let protocol = Arc::new(ProtocolService::new());
    let events = Arc::new(EventService::new());
    let mut event_rx = events.subscribe();
    let prop_store = Arc::new(std::sync::Mutex::new(MessagesStore::in_memory().unwrap()));
    let propagation = Arc::new(PropagationService::new(prop_store));

    let mut handle = spawn_inbound_worker(
        transport.clone(),
        messaging.clone(),
        protocol,
        events,
        propagation,
        None,
    );
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;

    let dest = [0x41; 16];
    let source = [0x42; 16];
    let wire_data = build_lxmf_wire(dest, source, "deliver once");
    let inbound = || ReceivedData {
        destination: AddressHash::new(dest),
        link_id: None,
        data: rns_core::packet::PacketDataBuffer::new_from_slice(&wire_data),
        payload_mode: ReceivedPayloadMode::FullWire,
        ratchet_used: false,
        context: None,
        request_id: None,
        hops: None,
        interface: None,
    };

    transport.inject_inbound(inbound());
    transport.inject_inbound(inbound());

    let first = tokio::time::timeout(std::time::Duration::from_secs(1), event_rx.recv())
        .await
        .expect("new-message event timeout")
        .expect("new-message event");
    let trust_drop = tokio::time::timeout(std::time::Duration::from_secs(1), event_rx.recv())
        .await
        .expect("drop event timeout")
        .expect("drop event");
    let duplicate_drop = tokio::time::timeout(std::time::Duration::from_secs(1), event_rx.recv())
        .await
        .expect("duplicate drop event timeout")
        .expect("duplicate drop event");

    assert_eq!(first.event_type, "message_received");
    assert_eq!(trust_drop.event_type, "inbound_dropped");
    assert_eq!(trust_drop.payload["path"], "direct_packet");
    assert_eq!(trust_drop.payload["reason"], "authentication_or_stamp_untrusted");
    assert_eq!(duplicate_drop.event_type, "inbound_dropped");
    assert_eq!(duplicate_drop.payload["path"], "direct_packet");
    assert_eq!(duplicate_drop.payload["reason"], "duplicate");
    assert_eq!(messaging.list_messages(10, None).unwrap().len(), 1);
    handle.abort();
    handle.wait().await;
}

#[tokio::test]
async fn inbound_worker_decodes_and_persists_message() {
    let transport = Arc::new(MockTransport::new_default());
    let messaging = Arc::new(MessagingService::new());
    let protocol = Arc::new(ProtocolService::new());
    let events = Arc::new(EventService::new());

    let mut event_rx = events.subscribe();

    // Spawn worker
    let prop_store = Arc::new(std::sync::Mutex::new(MessagesStore::in_memory().unwrap()));
    let propagation = Arc::new(PropagationService::new(prop_store));

    let mut handle = spawn_inbound_worker(
        transport.clone(),
        messaging.clone(),
        protocol.clone(),
        events.clone(),
        propagation,
        None, // no local delivery hash for test
    );

    // Give worker time to subscribe
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Inject a valid LXMF wire message
    let dest = [0x11u8; 16];
    let source = [0x22u8; 16];
    let wire_data = build_lxmf_wire(dest, source, "hello from mesh");

    transport.inject_inbound(ReceivedData {
        destination: AddressHash::new(dest),
        link_id: None,
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
    let event = tokio::time::timeout(std::time::Duration::from_millis(500), event_rx.recv())
        .await
        .expect("should receive event")
        .expect("event");
    assert_eq!(event.event_type, "message_received");
    handle.abort();
    handle.wait().await;
}

#[tokio::test]
async fn outbound_resource_completion_updates_the_service_store() {
    let transport = Arc::new(MockTransport::new_default());
    let store = Arc::new(std::sync::Mutex::new(MessagesStore::in_memory().unwrap()));
    store
        .lock()
        .unwrap()
        .insert_outbound_message(
            &MessageRecord {
                id: "resource-message".into(),
                source: "local".into(),
                destination: "remote".into(),
                title: String::new(),
                content: "large payload".into(),
                timestamp: 1,
                direction: "out".into(),
                fields: None,
                receipt_status: Some("sent: direct".into()),
                read: true,
            },
            &OutboundRouteRecord {
                message_id: "resource-message".into(),
                requested_method: "direct".into(),
                actual_method: "direct".into(),
                representation: "resource".into(),
                fallback_reason: None,
                correlation_id: "resource-message".into(),
                retry_of: None,
                deadline_unix_ms: i64::MAX,
                state: "sent".into(),
                attempt_count: 1,
            },
        )
        .unwrap();
    store
        .lock()
        .unwrap()
        .begin_outbound_attempt(&OutboundAttemptRecord {
            message_id: "resource-message".into(),
            attempt_number: 1,
            started_unix_ms: 1,
            deadline_unix_ms: i64::MAX,
            state: "sent".into(),
        })
        .unwrap();
    let messaging = Arc::new(MessagingService::with_store(store.clone()));
    let resource_hash = Hash::new([0x55; 32]);
    assert!(!messaging.handle_resource_complete(&hex::encode([0x54; 32])).unwrap());
    messaging.track_receipt(&hex::encode(resource_hash.to_bytes()), "resource-message");
    let propagation = Arc::new(PropagationService::new(Arc::new(std::sync::Mutex::new(
        MessagesStore::in_memory().unwrap(),
    ))));
    let mut handle = spawn_inbound_worker(
        transport.clone(),
        messaging,
        Arc::new(ProtocolService::new()),
        Arc::new(EventService::new()),
        propagation,
        None,
    );

    transport.inject_resource(ResourceEvent {
        hash: resource_hash,
        link_id: AddressHash::new([0x33; 16]),
        kind: ResourceEventKind::OutboundComplete,
    });

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        let record = store.lock().unwrap().get_message("resource-message").unwrap().unwrap();
        if record.receipt_status.as_deref() == Some("delivered: resource-complete") {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "resource completion timed out");
        tokio::task::yield_now().await;
    }
    handle.abort();
    handle.wait().await;
}

#[tokio::test]
async fn outbound_resource_integrity_failure_is_terminal_and_sticky() {
    let transport = Arc::new(MockTransport::new_default());
    let store = Arc::new(std::sync::Mutex::new(MessagesStore::in_memory().unwrap()));
    let message = MessageRecord {
        id: "failed-resource".into(),
        source: "local".into(),
        destination: "remote".into(),
        title: String::new(),
        content: "large payload".into(),
        timestamp: 1,
        direction: "out".into(),
        fields: None,
        receipt_status: Some("sent: direct".into()),
        read: true,
    };
    store
        .lock()
        .unwrap()
        .insert_outbound_message(
            &message,
            &OutboundRouteRecord {
                message_id: message.id.clone(),
                requested_method: "direct".into(),
                actual_method: "direct".into(),
                representation: "resource".into(),
                fallback_reason: None,
                correlation_id: message.id.clone(),
                retry_of: None,
                deadline_unix_ms: i64::MAX,
                state: "sent".into(),
                attempt_count: 1,
            },
        )
        .unwrap();
    store
        .lock()
        .unwrap()
        .begin_outbound_attempt(&OutboundAttemptRecord {
            message_id: message.id.clone(),
            attempt_number: 1,
            started_unix_ms: 1,
            deadline_unix_ms: i64::MAX,
            state: "sent".into(),
        })
        .unwrap();
    let messaging = Arc::new(MessagingService::with_store(store.clone()));
    let resource_hash = Hash::new([0x66; 32]);
    messaging.track_receipt(&hex::encode(resource_hash.to_bytes()), &message.id);
    assert!(!messaging
        .handle_packet_delivery_receipt(&hex::encode(resource_hash.to_bytes()))
        .unwrap());
    let mut handle = spawn_inbound_worker(
        transport.clone(),
        messaging.clone(),
        Arc::new(ProtocolService::new()),
        Arc::new(EventService::new()),
        Arc::new(PropagationService::new(Arc::new(std::sync::Mutex::new(
            MessagesStore::in_memory().unwrap(),
        )))),
        None,
    );

    transport.inject_resource(ResourceEvent {
        hash: resource_hash,
        link_id: AddressHash::new([0x44; 16]),
        kind: ResourceEventKind::Failed(ResourceFailure::Integrity),
    });
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if messaging.outbound_lifecycle(&message.id).unwrap().unwrap().0.state == "failed" {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("resource failure was not correlated");

    assert!(!messaging.handle_resource_complete(&hex::encode(resource_hash.to_bytes())).unwrap());
    let lifecycle = messaging.outbound_lifecycle(&message.id).unwrap().unwrap();
    assert_eq!(lifecycle.0.state, "failed");
    assert_eq!(
        messaging.get_message(&message.id).unwrap().unwrap().receipt_status.as_deref(),
        Some("failed: resource-integrity")
    );
    handle.abort();
    handle.wait().await;
}
