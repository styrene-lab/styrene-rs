use std::sync::{Arc, Mutex};
use styrene_ipc::types::{DaemonEvent, MessageAuthenticationState, MessageStampState};
use styrened::services::messaging::InboundAcceptOutcome;
use styrened::services::{EventService, MessagingService};
use styrened::storage::messages::{
    CanonicalInboundRecord, LxmfStampPolicy, LxmfTicketRecord, MessageRecord, MessagesStore,
    OutboundRouteRecord,
};

fn projection(id: &str) -> MessageRecord {
    MessageRecord {
        id: id.into(),
        source: "11".repeat(16),
        destination: "22".repeat(16),
        title: "projection".into(),
        content: "projection".into(),
        timestamp: 10,
        direction: "in".into(),
        fields: None,
        receipt_status: None,
        read: false,
    }
}

fn canonical(id: &str, auth: &str) -> CanonicalInboundRecord {
    CanonicalInboundRecord {
        message_id: id.into(),
        source: [0x11; 16],
        destination: [0x22; 16],
        title: vec![0xfe],
        content: vec![0xff],
        timestamp: 10.5,
        fields_msgpack: Some(vec![0xc0]),
        signature: Some(vec![0x33; 64]),
        stamp: None,
        wire: vec![0x44; 128],
        authentication_state: auth.into(),
        stamp_state: "not_applicable".into(),
        stamp_value: None,
        stamp_target: None,
    }
}

fn ticket(peer: &str, direction: &str) -> LxmfTicketRecord {
    LxmfTicketRecord {
        peer: peer.into(),
        ticket: vec![0x55; lxmf::stamps::TICKET_LENGTH],
        expires_at: 10_000_000,
        direction: direction.into(),
    }
}

#[test]
fn ticket_learning_is_atomic_and_requires_verified_authentication() {
    let store = MessagesStore::in_memory().unwrap();
    let received = ticket(&"11".repeat(16), "received");
    assert!(store
        .insert_canonical_with_verified_ticket(
            &projection("unknown"),
            &canonical("unknown", "unknown_identity"),
            Some(&received),
        )
        .unwrap());
    assert!(store.active_lxmf_ticket(&received.peer, "received", 1).unwrap().is_none());
    assert!(store
        .insert_canonical_with_verified_ticket(
            &projection("verified"),
            &canonical("verified", "verified"),
            Some(&received),
        )
        .unwrap());
    assert!(store.active_lxmf_ticket(&received.peer, "received", 1).unwrap().is_some());
}

#[test]
fn learned_cost_expires_after_pinned_window_boundary() {
    const EXPIRY: i64 = 45 * 24 * 60 * 60;
    let store = MessagesStore::in_memory().unwrap();
    store.learn_peer_stamp_cost("peer", 254, 100).unwrap();
    assert_eq!(store.peer_stamp_cost_at("peer", 100 + EXPIRY).unwrap(), Some(254));
    assert_eq!(store.peer_stamp_cost_at("peer", 101 + EXPIRY).unwrap(), None);
    store.set_lxmf_stamp_policy(LxmfStampPolicy { target_cost: 254, flexibility: 0 }).unwrap();
}

#[test]
fn ticket_delivery_cadence_and_repair_survive_restart() {
    const INTERVAL: i64 = 24 * 60 * 60;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ticket-cadence.sqlite");
    let peer = "22".repeat(16);
    let offered = ticket(&peer, "issued");
    {
        let store = MessagesStore::open(&path).unwrap();
        let message = projection("offered");
        let route = OutboundRouteRecord {
            message_id: message.id.clone(),
            requested_method: "direct".into(),
            actual_method: "direct".into(),
            representation: "packet".into(),
            fallback_reason: None,
            correlation_id: "offered".into(),
            retry_of: None,
            deadline_unix_ms: i64::MAX,
            state: "queued".into(),
            attempt_count: 0,
        };
        store.insert_outbound_message(&message, &route).unwrap();
        store.track_ticket_offer("offered", &offered).unwrap();
        assert!(!store.ticket_offer_due_at(&peer, 1_000).unwrap());
        store.finish_outbound("offered", "delivered", "delivered").unwrap();
    }
    let store = MessagesStore::open(&path).unwrap();
    assert_eq!(store.repair_ticket_offer_deliveries(1_000).unwrap(), 1);
    assert!(!store.ticket_offer_due_at(&peer, 1_000 + INTERVAL - 1).unwrap());
    assert!(store.ticket_offer_due_at(&peer, 1_000 + INTERVAL).unwrap());
    assert_eq!(store.repair_ticket_offer_deliveries(2_000).unwrap(), 0);
}

#[test]
fn failed_ticket_offer_rolls_back_message_and_route_atomically() {
    let store = MessagesStore::in_memory().unwrap();
    let message = projection("atomic-offer");
    let route = OutboundRouteRecord {
        message_id: message.id.clone(),
        requested_method: "direct".into(),
        actual_method: "direct".into(),
        representation: "packet".into(),
        fallback_reason: None,
        correlation_id: message.id.clone(),
        retry_of: None,
        deadline_unix_ms: i64::MAX,
        state: "queued".into(),
        attempt_count: 0,
    };
    let mut invalid_ticket = ticket(&message.destination, "issued");
    invalid_ticket.ticket.pop();
    assert!(store.reserve_ticket_offer(&message.destination, "atomic-reservation", 1_000).unwrap());
    let reservation = styrened::storage::messages::LxmfTicketOfferReservation {
        reservation_id: "atomic-reservation".into(),
        ticket: invalid_ticket,
    };

    assert!(store
        .insert_outbound_message_with_ticket_offer(&message, &route, Some(&reservation))
        .is_err());
    assert!(store.get_message(&message.id).unwrap().is_none());
    assert!(store.outbound_route(&message.id).unwrap().is_none());
}

#[test]
fn concurrent_ticket_offer_reservation_has_exactly_one_winner_and_recovers_on_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ticket-reservation.sqlite");
    MessagesStore::open(&path).unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let reserve = |reservation: &'static str| {
        let path = path.clone();
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            let store = MessagesStore::open(&path).unwrap();
            barrier.wait();
            store.reserve_ticket_offer("peer", reservation, 1_000).unwrap()
        })
    };
    let first = reserve("first");
    let second = reserve("second");
    barrier.wait();
    let winners = [first.join().unwrap(), second.join().unwrap()]
        .into_iter()
        .filter(|reserved| *reserved)
        .count();
    assert_eq!(winners, 1);

    let store = Arc::new(Mutex::new(MessagesStore::open(&path).unwrap()));
    let _service = MessagingService::with_store(store.clone());
    assert!(store.lock().unwrap().reserve_ticket_offer("peer", "after-restart", 1_001).unwrap());
}

#[test]
fn startup_ticket_reconciliation_pages_all_delivered_offers() {
    let store = MessagesStore::in_memory().unwrap();
    let offered = ticket("peer", "issued");
    for index in 0..1_025 {
        let id = format!("reconcile-{index:04}");
        let message = projection(&id);
        let route = OutboundRouteRecord {
            message_id: id.clone(),
            requested_method: "direct".into(),
            actual_method: "direct".into(),
            representation: "packet".into(),
            fallback_reason: None,
            correlation_id: id.clone(),
            retry_of: None,
            deadline_unix_ms: i64::MAX,
            state: "queued".into(),
            attempt_count: 0,
        };
        store.insert_outbound_message(&message, &route).unwrap();
        store.track_ticket_offer(&id, &offered).unwrap();
        store.finish_outbound(&id, "delivered", "delivered").unwrap();
    }
    assert_eq!(store.reconcile_ticket_offer_startup(2_000).unwrap(), 1_025);
    assert_eq!(store.repair_ticket_offer_deliveries(2_001).unwrap(), 0);
}

#[test]
fn ticket_reconciliation_failure_blocks_router_operations() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("broken-ticket-startup.sqlite");
    MessagesStore::open(&path).unwrap();
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute("DROP TABLE lxmf_ticket_offer_reservations", [])
        .unwrap();
    let store = Arc::new(Mutex::new(MessagesStore::open(&path).unwrap()));
    let service = MessagingService::with_store(store);
    let error = service.outbound_lifecycle("missing").unwrap_err();
    assert!(error.to_string().contains("ticket offer reconciliation failed"));
}

#[test]
fn message_snapshot_rejects_huge_limits_and_aggregate_projection_budget() {
    let store = MessagesStore::in_memory().unwrap();
    assert!(store.message_projection_snapshot_for_peer("peer", usize::MAX, None).is_err());
    assert!(store.search_message_projection_snapshot("query", None, usize::MAX).is_err());

    for index in 0..2 {
        let mut message = projection(&format!("budget-{index}"));
        message.source = "peer".into();
        message.content = "x".repeat(7 * 1024 * 1024);
        store.insert_message(&message).unwrap();
    }
    assert!(store.message_projection_snapshot_for_peer("peer", 2, None).is_err());
    assert!(store.search_message_projection_snapshot("x", Some("peer"), 2).is_err());
}

#[test]
fn poisoned_message_store_returns_an_operational_error() {
    let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
    let service = MessagingService::with_store(store.clone());
    let poison = store.clone();
    assert!(std::thread::spawn(move || {
        let _guard = poison.lock().unwrap();
        panic!("poison test store");
    })
    .join()
    .is_err());

    match service.accept_inbound([0; 16], &[], lxmf::inbound_decode::InboundPayloadMode::FullWire) {
        InboundAcceptOutcome::StorageError { error, .. } => {
            assert!(error.to_string().contains("poisoned"));
        }
        outcome => panic!("expected storage error, got {outcome:?}"),
    }
    assert!(service.get_message("missing").is_err());
    assert!(service.list_messages(10, None).is_err());
    assert!(service.search_messages("query", None, 10).is_err());
}

fn signed_wire(
    signer: &rns_core::identity::PrivateIdentity,
    destination: [u8; 16],
    timestamp: f64,
) -> ([u8; 16], Vec<u8>) {
    let source = signer.as_identity().address_hash.as_slice().try_into().unwrap();
    let payload = lxmf::Payload::new(
        timestamp,
        Some(vec![0xfe]),
        Some(vec![0xff]),
        Some(rmpv::Value::Map(vec![(rmpv::Value::from(1), rmpv::Value::Binary(vec![0x80]))])),
        None,
    );
    let mut wire = lxmf::WireMessage::new(destination, source, payload);
    wire.sign(signer).unwrap();
    (source, wire.pack().unwrap())
}

#[test]
fn deferred_authentication_pages_until_all_messages_are_revalidated() {
    let service = MessagingService::new();
    let signer = rns_core::identity::PrivateIdentity::new_from_name("paged-authentication");
    let destination = [0x42; 16];
    let mut source = [0; 16];
    for index in 0..257 {
        let (wire_source, wire) = signed_wire(&signer, destination, 1_700_000_000.0 + index as f64);
        source = wire_source;
        assert!(matches!(
            service.accept_inbound(
                destination,
                &wire,
                lxmf::inbound_decode::InboundPayloadMode::FullWire,
            ),
            InboundAcceptOutcome::Accepted(_)
        ));
    }
    assert_eq!(service.revalidate_unknown_identity(source, signer.as_identity()).unwrap(), 257);
}

#[tokio::test]
async fn deferred_auth_preserves_receipt_stamp_result_and_emits_authoritative_event() {
    let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
    store
        .lock()
        .unwrap()
        .set_lxmf_stamp_policy(LxmfStampPolicy { target_cost: 1, flexibility: 0 })
        .unwrap();
    let service = MessagingService::with_store(store.clone());
    let events = Arc::new(EventService::new());
    let mut receiver = events.subscribe_daemon_events();
    service.set_events(events);
    let signer = rns_core::identity::PrivateIdentity::new_from_name("receipt-stamp-state");
    let destination = [0x42; 16];
    let (source, wire) = signed_wire(&signer, destination, 1_700_000_000.25);
    let id = match service.accept_inbound(
        destination,
        &wire,
        lxmf::inbound_decode::InboundPayloadMode::FullWire,
    ) {
        InboundAcceptOutcome::Accepted(record) => record.id,
        outcome => panic!("expected accepted unknown-identity record, got {outcome:?}"),
    };
    assert_eq!(service.canonical_inbound(&id).unwrap().unwrap().stamp_state, "invalid");
    store
        .lock()
        .unwrap()
        .set_lxmf_stamp_policy(LxmfStampPolicy { target_cost: 0, flexibility: 0 })
        .unwrap();

    assert_eq!(service.revalidate_unknown_identity(source, signer.as_identity()).unwrap(), 1);
    let canonical = service.canonical_inbound(&id).unwrap().unwrap();
    assert_eq!(canonical.authentication_state, "verified");
    assert_eq!(canonical.stamp_state, "invalid");
    let event = receiver.recv().await.unwrap();
    let DaemonEvent::Message { kind, message } = event else {
        panic!("expected authentication reconciliation event");
    };
    assert_eq!(kind, styrene_ipc::types::MessageEventKind::StatusChanged);
    assert_eq!(message.id, id);
    assert_eq!(message.authentication_state, MessageAuthenticationState::Verified);
    assert_eq!(message.stamp_state, MessageStampState::Invalid);
}

#[test]
fn deferred_authentication_rejects_persisted_canonical_drift() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("canonical-drift.sqlite");
    let store = Arc::new(Mutex::new(MessagesStore::open(&path).unwrap()));
    let service = MessagingService::with_store(store);
    let signer = rns_core::identity::PrivateIdentity::new_from_name("drift-authentication");
    let destination = [0x42; 16];
    let (source, wire) = signed_wire(&signer, destination, 1_700_000_000.5);
    let id = match service.accept_inbound(
        destination,
        &wire,
        lxmf::inbound_decode::InboundPayloadMode::FullWire,
    ) {
        InboundAcceptOutcome::Accepted(record) => record.id,
        outcome => panic!("expected accepted message, got {outcome:?}"),
    };
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute("UPDATE canonical_inbound_messages SET title = X'00' WHERE message_id = ?1", [&id])
        .unwrap();

    let error = service.revalidate_unknown_identity(source, signer.as_identity()).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(
        service.canonical_inbound(&id).unwrap().unwrap().authentication_state,
        "unknown_identity"
    );
}

#[test]
fn v4_backfills_exact_noncanonical_fields_before_deferred_revalidation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("pre-v4-canonical.sqlite");
    let signer = rns_core::identity::PrivateIdentity::new_from_name("pre-v4-authentication");
    let destination = [0x42; 16];
    let (source, mut wire) = signed_wire(&signer, destination, 1_700_000_000.75);
    let canonical_fields = [0x81, 0x01, 0xc4, 0x01, 0x80];
    assert!(wire.ends_with(&canonical_fields));
    wire.truncate(wire.len() - canonical_fields.len());
    let noncanonical_fields = [0x81, 0xcc, 0x01, 0xc5, 0x00, 0x01, 0x80];
    wire.extend_from_slice(&noncanonical_fields);
    let message_id: [u8; 32] = hex::decode(
        lxmf::inbound_decode::outbound_message_id_hex(&wire).expect("exact message id"),
    )
    .unwrap()
    .try_into()
    .unwrap();
    let mut signed = Vec::new();
    signed.extend_from_slice(&destination);
    signed.extend_from_slice(&source);
    signed.extend_from_slice(&wire[96..]);
    signed.extend_from_slice(&message_id);
    wire[32..96].copy_from_slice(&signer.sign(&signed).to_bytes());

    let (_, mut unsigned_wire) = signed_wire(&signer, destination, 1_700_000_000.875);
    assert!(unsigned_wire.ends_with(&canonical_fields));
    unsigned_wire.truncate(unsigned_wire.len() - canonical_fields.len());
    unsigned_wire.extend_from_slice(&noncanonical_fields);

    let (message_id, unsigned_message_id) = {
        let store = Arc::new(Mutex::new(MessagesStore::open(&path).unwrap()));
        let service = MessagingService::with_store(store);
        let exact_id = match service.accept_inbound(
            destination,
            &wire,
            lxmf::inbound_decode::InboundPayloadMode::FullWire,
        ) {
            InboundAcceptOutcome::Accepted(record) => record.id,
            outcome => panic!("expected accepted legacy message, got {outcome:?}"),
        };
        let unsigned_id = match service.accept_inbound(
            destination,
            &unsigned_wire,
            lxmf::inbound_decode::InboundPayloadMode::FullWire,
        ) {
            InboundAcceptOutcome::Accepted(record) => record.id,
            outcome => panic!("expected inspectable rewritten legacy message, got {outcome:?}"),
        };
        (exact_id, unsigned_id)
    };

    {
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "DROP TABLE outbound_ticket_offers;
                 DROP TABLE lxmf_ticket_deliveries;
                 DROP TABLE lxmf_ticket_offer_reservations;
                 DELETE FROM schema_migrations
                    WHERE id IN (
                        '2026-08-23-canonical-lxmf-fidelity-v4',
                        '2026-08-23-lxmf-ticket-offer-reservations-v5'
                    );",
            )
            .unwrap();
        connection
            .execute(
                "UPDATE canonical_inbound_messages SET
                     source = zeroblob(16), destination = zeroblob(16), title = X'00',
                     content = X'00', timestamp = 0.0, fields_msgpack = NULL,
                     signature = NULL, stamp = NULL, authentication_state = 'unknown_identity',
                     stamp_state = 'unknown', stamp_value = NULL
                 WHERE message_id = ?1",
                [&message_id],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE canonical_inbound_messages SET fields_msgpack = NULL,
                     authentication_state = 'unknown_identity'
                 WHERE message_id = ?1",
                [&unsigned_message_id],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO messages
                     (id, source, destination, title, content, timestamp, direction, fields, receipt_status, read)
                 VALUES ('legacy-invalid', '', '', '', '', 0, 'in', NULL, NULL, 0)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO canonical_inbound_messages
                     (message_id, source, destination, title, content, timestamp, fields_msgpack,
                      signature, stamp, wire, authentication_state, stamp_state, stamp_value)
                 VALUES ('legacy-invalid', zeroblob(16), zeroblob(16), X'00', X'00', 0.0,
                         NULL, NULL, NULL, X'00', 'unknown_identity', 'unknown', NULL)",
                [],
            )
            .unwrap();
    }

    let store = Arc::new(Mutex::new(MessagesStore::open(&path).unwrap()));
    let service = MessagingService::with_store(store);
    let migrated = service.canonical_inbound(&message_id).unwrap().unwrap();
    assert_eq!(migrated.source, source);
    assert_eq!(migrated.destination, destination);
    assert_eq!(migrated.timestamp, 1_700_000_000.75);
    assert_eq!(migrated.title, vec![0xff]);
    assert_eq!(migrated.content, vec![0xfe]);
    assert_eq!(migrated.fields_msgpack, Some(noncanonical_fields.to_vec()));
    assert_eq!(migrated.wire, wire);
    assert_eq!(migrated.authentication_state, "unknown_identity");
    assert_eq!(migrated.stamp_state, "unknown");
    assert_eq!(
        service.canonical_inbound("legacy-invalid").unwrap().unwrap().authentication_state,
        "invalid"
    );
    assert_eq!(service.revalidate_unknown_identity(source, signer.as_identity()).unwrap(), 2);
    assert_eq!(
        service.canonical_inbound(&message_id).unwrap().unwrap().authentication_state,
        "verified"
    );
    assert_eq!(service.canonical_inbound(&message_id).unwrap().unwrap().stamp_state, "unknown");
    assert_eq!(
        service.canonical_inbound(&unsigned_message_id).unwrap().unwrap().authentication_state,
        "invalid"
    );
}

#[tokio::test]
async fn typed_message_event_redacts_canonical_bytes_and_preserves_safe_fidelity() {
    let events = EventService::new();
    let mut receiver = events.subscribe_daemon_events();
    let projection = projection("event");
    let mut canonical = canonical("event", "verified");
    canonical.fields_msgpack = Some(vec![0x81, 0xcc, 0x01, 0xcd, 0x00, 0x02]);
    canonical.stamp = Some(vec![0x66; 32]);
    canonical.stamp_state = "verified".into();
    canonical.stamp_value = Some(17);
    canonical.stamp_target = Some(12);

    events.emit_message_new(&projection, Some(&canonical));
    let event = receiver.recv().await.unwrap();
    let DaemonEvent::Message { message, .. } = event else {
        panic!("expected message event");
    };
    assert_eq!(message.lxmf_timestamp, Some(10.5));
    assert!(message.canonical_title.is_none());
    assert!(message.canonical_content.is_none());
    assert!(message.canonical_fields_msgpack.is_none());
    assert!(message.canonical_signature.is_none());
    assert!(message.canonical_stamp.is_none());
    assert!(message.canonical_wire.is_none());
    assert_eq!(message.authentication_state, MessageAuthenticationState::Verified);
    assert_eq!(message.stamp_state, MessageStampState::Verified);
    assert_eq!(message.stamp_value, Some(17));
    assert_eq!(message.stamp_cost, Some(12));
}
