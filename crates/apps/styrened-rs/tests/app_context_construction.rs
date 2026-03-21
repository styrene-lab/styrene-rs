//! Integration tests for AppContext — verifying the composition root
//! can be constructed and all service accessors work.

use reticulum_daemon::app_context::AppContext;
use reticulum_daemon::storage::messages::MessagesStore;
use reticulum_daemon::transport::mesh_transport::MeshTransport;
use reticulum_daemon::transport::null_transport::NullTransport;
use std::sync::{Arc, Mutex};

fn test_store() -> Arc<Mutex<MessagesStore>> {
    Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()))
}

#[test]
fn app_context_constructs_with_null_transport() {
    let transport: Arc<dyn MeshTransport> = Arc::new(NullTransport::new());
    let ctx = AppContext::new(transport, "test-identity".into(), test_store());
    assert!(!ctx.transport().is_connected());
}

#[test]
fn app_context_all_service_accessors_work() {
    let transport: Arc<dyn MeshTransport> = Arc::new(NullTransport::new());
    let ctx = AppContext::new(transport, "test-identity".into(), test_store());

    let _ = ctx.identity();
    let _ = ctx.config();
    let _ = ctx.status();
    let _ = ctx.fleet();
    let _ = ctx.auth();
    let _ = ctx.auto_reply();
    let _ = ctx.messaging();
    let _ = ctx.discovery();
    let _ = ctx.protocol();
    let _ = ctx.events();
    let _ = ctx.tunnel();
}

#[test]
fn app_context_transport_arc_returns_clone() {
    let transport: Arc<dyn MeshTransport> = Arc::new(NullTransport::new());
    let ctx = AppContext::new(transport, String::new(), test_store());
    let arc = ctx.transport_arc();
    assert!(!arc.is_connected());
}

#[test]
fn app_context_can_be_wrapped_in_arc() {
    let transport: Arc<dyn MeshTransport> = Arc::new(NullTransport::new());
    let ctx = Arc::new(AppContext::new(transport, "arc-test".into(), test_store()));
    let ctx_clone = ctx.clone();
    assert!(!ctx_clone.transport().is_connected());
    let _ = ctx_clone.identity();
    let _ = ctx_clone.messaging();
}

#[test]
fn app_context_identity_service_has_correct_hash() {
    let transport: Arc<dyn MeshTransport> = Arc::new(NullTransport::new());
    let ctx = AppContext::new(transport, "my-hash-abc123".into(), test_store());
    assert_eq!(ctx.identity().identity_hash(), "my-hash-abc123");
}

#[test]
fn app_context_config_service_starts_empty() {
    let transport: Arc<dyn MeshTransport> = Arc::new(NullTransport::new());
    let ctx = AppContext::new(transport, String::new(), test_store());
    assert!(!ctx.config().is_loaded());
}

#[test]
fn app_context_messaging_and_discovery_share_store() {
    let transport: Arc<dyn MeshTransport> = Arc::new(NullTransport::new());
    let ctx = AppContext::new(transport, String::new(), test_store());

    // Discovery writes an announce
    ctx.discovery()
        .accept_announce_with_details("peer1".into(), 1000, Some("TestNode".into()), None, None)
        .unwrap();

    // Discovery can read it back
    let announces = ctx.discovery().list_announces(10).unwrap();
    assert_eq!(announces.len(), 1);
    assert_eq!(announces[0].name, Some("TestNode".into()));

    // Messaging can insert and read messages through the same store
    let record = reticulum_daemon::storage::messages::MessageRecord {
        id: "msg1".into(),
        source: "src".into(),
        destination: "dst".into(),
        title: "Test".into(),
        content: "Hello".into(),
        timestamp: 2000,
        direction: "out".into(),
        fields: None,
        receipt_status: None,
            read: false,
    };
    ctx.messaging().accept_inbound_record(&record).unwrap();
    assert!(ctx.messaging().get_message("msg1").unwrap().is_some());
}

#[test]
fn app_context_set_signer_wires_transport() {
    let transport: Arc<dyn MeshTransport> = Arc::new(NullTransport::new());
    let ctx = AppContext::new(transport.clone(), "signer-test".into(), test_store());

    // Before set_signer, send_chat should fail with "transport not available"
    let rt = tokio::runtime::Runtime::new().unwrap();
    let err = rt.block_on(ctx.messaging().send_chat("deadbeef01020304deadbeef01020304", "hello", None));
    assert!(err.is_err());
    assert!(
        err.unwrap_err().to_string().contains("transport not available"),
        "should fail before set_signer"
    );

    // After set_signer, send_chat should fail with a different error (not connected)
    // because NullTransport.is_connected() returns false
    let identity = rns_core::identity::PrivateIdentity::new_from_name("test-signer");
    ctx.set_signer(Arc::new(identity));

    let err2 = rt.block_on(ctx.messaging().send_chat("deadbeef01020304deadbeef01020304", "hello", None));
    assert!(err2.is_err());
    assert!(
        err2.unwrap_err().to_string().contains("not connected"),
        "should fail with transport-not-connected after set_signer, not transport-not-available"
    );
}
