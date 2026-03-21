//! Transport contract tests — verify that MeshTransport trait semantics
//! hold across implementations (NullTransport, MockTransport).
//!
//! These tests define the behavioral contract. Any new MeshTransport
//! implementation must pass these tests.
//!
//! Package C — see ownership-matrix.md §MeshTransport.

use reticulum_daemon::transport::mesh_transport::{
    MeshTransport, TransportError, TransportLifecycleEvent,
};
use reticulum_daemon::transport::mock_transport::MockTransport;
use reticulum_daemon::transport::null_transport::NullTransport;
use rns_core::hash::AddressHash;
use rns_core::packet::PacketDataBuffer;
use rns_core::transport::core_transport::{ReceivedData, ReceivedPayloadMode, SendPacketOutcome};
use std::sync::Arc;
use std::time::Duration;

// ============================================================
// Contract: send_raw
// ============================================================

async fn contract_send_raw_returns_result(transport: &dyn MeshTransport) {
    let dest = AddressHash::new([0xAA; 16]);
    let result = transport.send_raw(dest, b"payload").await;
    // Must return Ok(outcome) or Err(TransportError) — never panic
    match result {
        Ok(_outcome) => {} // valid
        Err(_e) => {}      // valid
    }
}

#[tokio::test]
async fn null_send_raw_contract() {
    contract_send_raw_returns_result(&NullTransport::new()).await;
}

#[tokio::test]
async fn mock_send_raw_contract() {
    contract_send_raw_returns_result(&MockTransport::new_default()).await;
}

// ============================================================
// Contract: is_connected reflects transport state
// ============================================================

#[test]
fn null_transport_is_never_connected() {
    let t = NullTransport::new();
    assert!(!t.is_connected());
}

#[test]
fn mock_transport_is_connected_by_default() {
    let t = MockTransport::new_default();
    assert!(t.is_connected());
}

// ============================================================
// Contract: resolve_identity returns None for unknown peers
// ============================================================

async fn contract_resolve_unknown_returns_none(transport: &dyn MeshTransport) {
    let unknown = AddressHash::new([0xFF; 16]);
    assert!(transport.resolve_identity(&unknown).await.is_none());
}

#[tokio::test]
async fn null_resolve_unknown_contract() {
    contract_resolve_unknown_returns_none(&NullTransport::new()).await;
}

#[tokio::test]
async fn mock_resolve_unknown_contract() {
    contract_resolve_unknown_returns_none(&MockTransport::new_default()).await;
}

// ============================================================
// Contract: shutdown succeeds without panic
// ============================================================

async fn contract_shutdown_succeeds(transport: &dyn MeshTransport) {
    let result = transport.shutdown().await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn null_shutdown_contract() {
    contract_shutdown_succeeds(&NullTransport::new()).await;
}

#[tokio::test]
async fn mock_shutdown_contract() {
    contract_shutdown_succeeds(&MockTransport::new_default()).await;
}

// ============================================================
// Contract: request_path and announce are no-ops that don't panic
// ============================================================

async fn contract_request_path_no_panic(transport: &dyn MeshTransport) {
    let dest = AddressHash::new([0xBB; 16]);
    transport.request_path(&dest).await;
    // No panic = pass
}

async fn contract_announce_no_panic(transport: &dyn MeshTransport) {
    transport.announce(Some(b"test-app-data")).await;
    transport.announce(None).await;
    // No panic = pass
}

#[tokio::test]
async fn null_request_path_contract() {
    contract_request_path_no_panic(&NullTransport::new()).await;
}

#[tokio::test]
async fn mock_request_path_contract() {
    contract_request_path_no_panic(&MockTransport::new_default()).await;
}

#[tokio::test]
async fn null_announce_contract() {
    contract_announce_no_panic(&NullTransport::new()).await;
}

#[tokio::test]
async fn mock_announce_contract() {
    contract_announce_no_panic(&MockTransport::new_default()).await;
}

// ============================================================
// Contract: subscribe channels return valid receivers
// ============================================================

fn contract_subscribe_returns_valid_receiver(transport: &dyn MeshTransport) {
    let _inbound = transport.subscribe_inbound();
    let _announces = transport.subscribe_announces();
    let _lifecycle = transport.subscribe_lifecycle();
    // All must return without panic
}

#[test]
fn null_subscribe_contract() {
    contract_subscribe_returns_valid_receiver(&NullTransport::new());
}

#[test]
fn mock_subscribe_contract() {
    contract_subscribe_returns_valid_receiver(&MockTransport::new_default());
}

// ============================================================
// Contract: dyn MeshTransport works behind Arc
// ============================================================

async fn contract_arc_dyn_works(transport: Arc<dyn MeshTransport>) {
    let _ = transport.is_connected();
    let _ = transport.identity_hash();
    let _ = transport.destination_hash();
    transport.request_path(&AddressHash::new([0xCC; 16])).await;
    // Must compile and run without issues
}

#[tokio::test]
async fn null_arc_dyn_contract() {
    contract_arc_dyn_works(Arc::new(NullTransport::new())).await;
}

#[tokio::test]
async fn mock_arc_dyn_contract() {
    contract_arc_dyn_works(Arc::new(MockTransport::new_default())).await;
}

// ============================================================
// MockTransport-specific: inbound fan-out
// ============================================================

#[tokio::test]
async fn mock_inbound_fanout_reaches_multiple_subscribers() {
    let mock = MockTransport::new_default();
    let mut rx1 = mock.subscribe_inbound();
    let mut rx2 = mock.subscribe_inbound();

    let data = ReceivedData {
        destination: AddressHash::new([1u8; 16]),
        data: PacketDataBuffer::new_from_slice(b"test"),
        payload_mode: ReceivedPayloadMode::FullWire,
        ratchet_used: false,
        context: None,
        request_id: None,
        hops: None,
        interface: None,
    };
    mock.inject_inbound(data);

    let r1 = rx1.recv().await.unwrap();
    let r2 = rx2.recv().await.unwrap();
    assert_eq!(r1.destination, AddressHash::new([1u8; 16]));
    assert_eq!(r2.destination, AddressHash::new([1u8; 16]));
}

// ============================================================
// MockTransport-specific: lifecycle fan-out
// ============================================================

#[tokio::test]
async fn mock_lifecycle_fanout_reaches_multiple_subscribers() {
    let mock = MockTransport::new_default();
    let mut rx1 = mock.subscribe_lifecycle();
    let mut rx2 = mock.subscribe_lifecycle();

    mock.inject_lifecycle(TransportLifecycleEvent::Reconnected);

    assert_eq!(
        rx1.recv().await.unwrap(),
        TransportLifecycleEvent::Reconnected
    );
    assert_eq!(
        rx2.recv().await.unwrap(),
        TransportLifecycleEvent::Reconnected
    );
}

// ============================================================
// NullTransport-specific: sends always fail with Unavailable
// ============================================================

#[tokio::test]
async fn null_all_sends_fail_with_unavailable() {
    let null = NullTransport::new();
    let dest = AddressHash::new([0xDD; 16]);

    let r1 = null.send_raw(dest, b"test").await;
    assert!(matches!(r1, Err(TransportError::Unavailable)));

    // send_via_link also fails
    let identity = rns_core::identity::PrivateIdentity::new_from_name("peer");
    let desc = rns_core::destination::DestinationDesc {
        identity: *identity.as_identity(),
        address_hash: dest,
        name: rns_core::destination::DestinationName::new("lxmf", "delivery"),
    };
    let r2 = null.send_via_link(desc, b"test", Duration::from_secs(1)).await;
    assert!(matches!(r2, Err(TransportError::Unavailable)));
}

// ============================================================
// MockTransport-specific: queued results with fallback
// ============================================================

#[tokio::test]
async fn mock_queued_send_raw_exhausts_then_defaults() {
    let mock = MockTransport::new_default();
    let dest = AddressHash::new([0xEE; 16]);

    // Queue one failure
    mock.queue_send_raw(Err(TransportError::SendFailed("queued failure".into())));

    // First call gets queued result
    let r1 = mock.send_raw(dest, b"a").await;
    assert!(matches!(r1, Err(TransportError::SendFailed(_))));

    // Second call gets default (SentDirect)
    let r2 = mock.send_raw(dest, b"b").await;
    assert!(matches!(r2, Ok(SendPacketOutcome::SentDirect)));
}
