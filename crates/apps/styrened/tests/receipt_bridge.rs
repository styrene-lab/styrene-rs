use rns_core::transport::core_transport::{DeliveryReceipt, ReceiptHandler};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use styrened::receipt_bridge::{
    register_receipt_waiter, CompositeReceiptHandler, ReceiptBridge, ReceiptWaiters,
    ServiceReceiptBridge,
};
use styrened::services::MessagingService;
use styrened::storage::messages::{MessageRecord, MessagesStore};
use tokio::sync::mpsc::unbounded_channel;

#[tokio::test]
async fn receipt_bridge_emits_event_for_known_packet() {
    let (tx, mut rx) = unbounded_channel();
    let map = Arc::new(Mutex::new(HashMap::new()));
    let packet_id = [7u8; 32];
    let packet_hex = hex::encode(packet_id);
    map.lock().unwrap().insert(packet_hex, "msg-1".to_string());

    let waiters: ReceiptWaiters = Arc::new(Mutex::new(HashMap::new()));
    let waiter = register_receipt_waiter(&waiters, "msg-1");
    let bridge = ReceiptBridge::new(map.clone(), waiters, tx);
    bridge.on_receipt(&DeliveryReceipt::new(packet_id));

    let event = rx.recv().await.expect("receipt event");
    assert_eq!(event.message_id, "msg-1");
    assert_eq!(event.status, "delivered");
    assert_eq!(waiter.await.expect("waiter notified"), "delivered");
}

#[tokio::test]
async fn composite_bridge_fans_out_to_legacy_and_service_paths() {
    let packet_id = [8_u8; 32];
    let map = Arc::new(Mutex::new(HashMap::from([(
        hex::encode(packet_id),
        "msg-composite".to_string(),
    )])));
    let (legacy_tx, mut legacy_rx) = unbounded_channel();
    let service_store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
    service_store
        .lock()
        .unwrap()
        .insert_message(&MessageRecord {
            id: "msg-composite".into(),
            source: "local".into(),
            destination: "remote".into(),
            title: String::new(),
            content: "composite".into(),
            timestamp: 1,
            direction: "out".into(),
            fields: None,
            receipt_status: Some("sent: direct".into()),
            read: true,
        })
        .unwrap();
    let service = Arc::new(MessagingService::with_store(service_store.clone()));
    service.track_receipt(&hex::encode(packet_id), "msg-composite");
    let target = Arc::new(std::sync::OnceLock::new());
    target.set(Arc::downgrade(&service)).unwrap_or_else(|_| panic!("set receipt target"));
    let bridge = CompositeReceiptHandler::new(vec![
        Box::new(ReceiptBridge::new(map, Arc::new(Mutex::new(HashMap::new())), legacy_tx)),
        Box::new(ServiceReceiptBridge::new(target)),
    ]);

    bridge.on_receipt(&DeliveryReceipt::new(packet_id));

    let event = tokio::time::timeout(std::time::Duration::from_secs(1), legacy_rx.recv())
        .await
        .expect("legacy receipt bridge timed out")
        .expect("legacy receipt bridge closed");
    assert_eq!(event.message_id, "msg-composite");
    assert_eq!(
        service_store
            .lock()
            .unwrap()
            .get_message("msg-composite")
            .unwrap()
            .unwrap()
            .receipt_status
            .as_deref(),
        Some("delivered: packet-receipt")
    );
}

#[test]
fn service_receipt_bridge_rejects_untracked_receipt_and_does_not_buffer_it() {
    let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
    store
        .lock()
        .unwrap()
        .insert_message(&MessageRecord {
            id: "msg-service".into(),
            source: "local".into(),
            destination: "remote".into(),
            title: String::new(),
            content: "receipt test".into(),
            timestamp: 1,
            direction: "out".into(),
            fields: None,
            receipt_status: Some("sent: direct".into()),
            read: true,
        })
        .unwrap();
    let messaging = Arc::new(MessagingService::with_store(store.clone()));
    let packet_id = [9_u8; 32];
    let target = Arc::new(std::sync::OnceLock::new());
    target.set(Arc::downgrade(&messaging)).unwrap_or_else(|_| panic!("set receipt target"));
    let bridge = ServiceReceiptBridge::new(target);

    bridge.on_receipt(&DeliveryReceipt::new(packet_id));

    assert_eq!(
        store
            .lock()
            .unwrap()
            .get_message("msg-service")
            .unwrap()
            .unwrap()
            .receipt_status
            .as_deref(),
        Some("sent: direct")
    );

    messaging.track_receipt(&hex::encode(packet_id), "msg-service");
    assert_eq!(
        store
            .lock()
            .unwrap()
            .get_message("msg-service")
            .unwrap()
            .unwrap()
            .receipt_status
            .as_deref(),
        Some("sent: direct")
    );

    bridge.on_receipt(&DeliveryReceipt::new(packet_id));
    assert_eq!(
        store
            .lock()
            .unwrap()
            .get_message("msg-service")
            .unwrap()
            .unwrap()
            .receipt_status
            .as_deref(),
        Some("delivered: packet-receipt")
    );
}

#[test]
fn service_receipt_bridge_does_not_retain_messaging_service() {
    let messaging = Arc::new(MessagingService::new());
    let weak = Arc::downgrade(&messaging);
    let target = Arc::new(std::sync::OnceLock::new());
    target.set(weak.clone()).unwrap_or_else(|_| panic!("set receipt target"));
    let bridge = ServiceReceiptBridge::new(target);

    drop(messaging);

    assert!(weak.upgrade().is_none());
    bridge.on_receipt(&DeliveryReceipt::new([0x44; 32]));
}
