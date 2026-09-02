//! Integration test: inbound worker processes transport events through service layer.

use rns_core::destination::{DestinationName, SingleOutputDestination};
use rns_core::hash::AddressHash;
use rns_core::hash::Hash;
use rns_core::identity::PrivateIdentity;
use rns_core::transport::core_transport::{ReceivedData, ReceivedPayloadMode};
use rns_core::transport::resource::{
    ResourceComplete, ResourceEvent, ResourceEventKind, ResourceFailure,
};
use std::sync::Arc;
use styrened::services::{
    AutoReplyConfig, AutoReplyMode, AutoReplyService, EventService, MessagingService,
    PropagationService, ProtocolService,
};
use styrened::storage::messages::MessagesStore;
use styrened::storage::messages::{MessageRecord, OutboundAttemptRecord, OutboundRouteRecord};
use styrened::transport::mock_transport::{MockCall, MockTransport};
use styrened::workers::inbound::{
    InboundDestinations, spawn_inbound_worker, spawn_inbound_worker_with_auto_reply,
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

fn build_signed_lxmf_wire(
    signer: &PrivateIdentity,
    destination: [u8; 16],
    content: &str,
    fields: Option<serde_json::Value>,
) -> ([u8; 16], Vec<u8>) {
    let source = SingleOutputDestination::new(
        *signer.as_identity(),
        DestinationName::new("lxmf", "delivery"),
    )
    .desc
    .address_hash
    .as_slice()
    .try_into()
    .unwrap();
    let wire =
        styrened::lxmf_bridge::build_wire_message(source, destination, "", content, fields, signer)
            .unwrap();
    (source, wire)
}

fn spawn_echo_worker(
    transport: Arc<MockTransport>,
    messaging: Arc<MessagingService>,
    local_destination: [u8; 16],
) -> styrened::workers::inbound::InboundWorkerHandle {
    messaging.set_signer(
        transport.clone(),
        Arc::new(PrivateIdentity::new_from_name("worker-echo-local")),
    );
    let auto_reply = Arc::new(AutoReplyService::new());
    auto_reply.set_config(AutoReplyConfig {
        mode: AutoReplyMode::Echo,
        message: String::new(),
        cooldown: std::time::Duration::ZERO,
    });
    spawn_inbound_worker_with_auto_reply(
        transport,
        messaging,
        Arc::new(ProtocolService::new()),
        Arc::new(EventService::new()),
        Arc::new(PropagationService::new(Arc::new(std::sync::Mutex::new(
            MessagesStore::in_memory().unwrap(),
        )))),
        InboundDestinations::new(Some(hex::encode(local_destination)), None),
        Some(auto_reply),
    )
}

async fn assert_single_structured_echo(
    transport: &MockTransport,
    source: [u8; 16],
    body: &str,
    request_id: &str,
) {
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        transport.wait_for_calls(1, |call| matches!(call, MockCall::SendRaw { .. })),
    )
    .await
    .expect("echo send timeout");
    let sends = transport
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            MockCall::SendRaw { dest, data } => Some((dest, data)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(sends.len(), 1, "exactly one echo must be sent");
    assert_eq!(sends[0].0, AddressHash::new(source));

    let mut full_wire = source.to_vec();
    full_wire.extend_from_slice(&sends[0].1);
    let echo = lxmf::WireMessage::unpack(&full_wire).expect("decode echo wire");
    assert_eq!(
        echo.payload.content.as_ref().map(|content| content.as_slice()),
        Some(body.as_bytes())
    );
    let fields = serde_json::to_value(echo.payload.fields.expect("echo fields")).unwrap();
    assert_eq!(fields["styrene_echo"]["response"], true);
    assert_eq!(fields["styrene_echo"]["request_id"], request_id);
}

#[tokio::test]
async fn trusted_packet_sends_one_correlated_structured_echo() {
    let destination = [0x31; 16];
    let transport =
        Arc::new(MockTransport::new(AddressHash::new([0x30; 16]), AddressHash::new(destination)));
    let messaging = Arc::new(MessagingService::new());
    let sender = PrivateIdentity::new_from_name("trusted-packet-echo-sender");
    let (source, wire) = build_signed_lxmf_wire(&sender, destination, "packet echo body", None);
    transport.queue_resolve(Some(*sender.as_identity()));
    transport.queue_resolve(Some(*sender.as_identity()));
    let mut handle = spawn_echo_worker(transport.clone(), messaging.clone(), destination);

    transport.inject_inbound(ReceivedData {
        destination: AddressHash::new(destination),
        link_id: None,
        data: rns_core::packet::PacketDataBuffer::new_from_slice(&wire),
        payload_mode: ReceivedPayloadMode::FullWire,
        ratchet_used: false,
        context: None,
        request_id: None,
        hops: None,
        interface: None,
        packet_hash: None,
        receiving_iface: None,
    });

    let request_id = lxmf::WireMessage::unpack(&wire).unwrap().message_id();
    assert_single_structured_echo(&transport, source, "packet echo body", &hex::encode(request_id))
        .await;
    handle.abort();
    handle.wait().await;
}

#[tokio::test]
async fn trusted_completed_resource_sends_one_correlated_structured_echo() {
    let destination = [0x51; 16];
    let transport =
        Arc::new(MockTransport::new(AddressHash::new([0x50; 16]), AddressHash::new(destination)));
    let messaging = Arc::new(MessagingService::new());
    let sender = PrivateIdentity::new_from_name("trusted-resource-echo-sender");
    let (source, wire) = build_signed_lxmf_wire(&sender, destination, "resource echo body", None);
    transport.queue_resolve(Some(*sender.as_identity()));
    transport.queue_resolve(Some(*sender.as_identity()));
    let mut handle = spawn_echo_worker(transport.clone(), messaging, destination);

    transport.inject_resource(ResourceEvent {
        hash: Hash::new([0x52; 32]),
        link_id: AddressHash::new([0x53; 16]),
        kind: ResourceEventKind::Complete(ResourceComplete {
            data: wire.clone(),
            metadata: None,
            request_id: None,
            is_request: false,
            is_response: false,
            transfer_size: wire.len() as u64,
            checksum_verified: true,
        }),
        progress: None,
    });

    let request_id = lxmf::WireMessage::unpack(&wire).unwrap().message_id();
    assert_single_structured_echo(
        &transport,
        source,
        "resource echo body",
        &hex::encode(request_id),
    )
    .await;
    handle.abort();
    handle.wait().await;
}

#[tokio::test]
async fn trusted_protocol_and_marked_response_packets_do_not_echo() {
    let destination = [0x61; 16];
    let transport =
        Arc::new(MockTransport::new(AddressHash::new([0x60; 16]), AddressHash::new(destination)));
    let messaging = Arc::new(MessagingService::new());
    let sender = PrivateIdentity::new_from_name("trusted-non-echo-sender");
    let (_, protocol_wire) = build_signed_lxmf_wire(
        &sender,
        destination,
        "protocol body",
        Some(serde_json::json!({"protocol": "fleet"})),
    );
    let (_, response_wire) = build_signed_lxmf_wire(
        &sender,
        destination,
        "response body",
        Some(serde_json::json!({"styrene_echo": {"response": true, "request_id": "original"}})),
    );
    transport.queue_resolve(Some(*sender.as_identity()));
    transport.queue_resolve(Some(*sender.as_identity()));
    let mut handle = spawn_echo_worker(transport.clone(), messaging.clone(), destination);

    for wire in [protocol_wire, response_wire] {
        transport.inject_inbound(ReceivedData {
            destination: AddressHash::new(destination),
            link_id: None,
            data: rns_core::packet::PacketDataBuffer::new_from_slice(&wire),
            payload_mode: ReceivedPayloadMode::FullWire,
            ratchet_used: false,
            context: None,
            request_id: None,
            hops: None,
            interface: None,
            packet_hash: None,
            receiving_iface: None,
        });
    }
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while messaging.list_messages(10, None).unwrap().len() < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("trusted packets were not accepted");
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    assert!(
        !transport
            .calls()
            .iter()
            .any(|call| matches!(call, MockCall::SendRaw { .. } | MockCall::SendViaLink { .. }))
    );
    handle.abort();
    handle.wait().await;
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
        packet_hash: None,
        receiving_iface: None,
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

    let auto_reply = Arc::new(AutoReplyService::new());
    auto_reply.set_config(AutoReplyConfig {
        mode: AutoReplyMode::Echo,
        message: String::new(),
        cooldown: std::time::Duration::ZERO,
    });
    let mut handle = spawn_inbound_worker_with_auto_reply(
        transport.clone(),
        messaging.clone(),
        protocol,
        events,
        propagation,
        InboundDestinations::new(None, None),
        Some(auto_reply),
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
        packet_hash: None,
        receiving_iface: None,
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
    assert!(!transport.calls().iter().any(|call| matches!(
        call,
        styrened::transport::mock_transport::MockCall::SendRaw { .. }
            | styrened::transport::mock_transport::MockCall::SendViaLink { .. }
    )));
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
        packet_hash: None,
        receiving_iface: None,
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
            route_observation: None,
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
        progress: None,
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
            route_observation: None,
        })
        .unwrap();
    let messaging = Arc::new(MessagingService::with_store(store.clone()));
    let resource_hash = Hash::new([0x66; 32]);
    messaging.track_receipt(&hex::encode(resource_hash.to_bytes()), &message.id);
    assert!(
        !messaging.handle_packet_delivery_receipt(&hex::encode(resource_hash.to_bytes())).unwrap()
    );
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
        progress: None,
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
