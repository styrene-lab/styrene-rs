//! Integration tests for NullTransport — verifying the null object pattern
//! works correctly from outside the crate.

use reticulum_daemon::transport::mesh_transport::{MeshTransport, TransportError};
use reticulum_daemon::transport::null_transport::NullTransport;
use rns_core::destination::{DestinationDesc, DestinationName};
use rns_core::hash::AddressHash;
use rns_core::identity::PrivateIdentity;
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
async fn null_transport_send_raw_returns_unavailable() {
    let transport = NullTransport::new();
    let dest = AddressHash::new([0xAA; 16]);
    let result = transport.send_raw(dest, b"test payload").await;
    assert!(matches!(result, Err(TransportError::Unavailable)));
}

#[tokio::test]
async fn null_transport_send_via_link_returns_unavailable() {
    let transport = NullTransport::new();
    let identity = PrivateIdentity::new_from_name("test-peer");
    let desc = DestinationDesc {
        identity: *identity.as_identity(),
        address_hash: AddressHash::new([0xBB; 16]),
        name: DestinationName::new("lxmf", "delivery"),
    };
    let result = transport
        .send_via_link(desc, b"test", Duration::from_secs(5))
        .await;
    assert!(matches!(result, Err(TransportError::Unavailable)));
}

#[tokio::test]
async fn null_transport_resolve_identity_returns_none() {
    let transport = NullTransport::new();
    let dest = AddressHash::new([0xCC; 16]);
    assert!(transport.resolve_identity(&dest).await.is_none());
}

#[test]
fn null_transport_is_not_connected() {
    let transport = NullTransport::new();
    assert!(!transport.is_connected());
}

#[test]
fn null_transport_hashes_are_zero() {
    let transport = NullTransport::new();
    let zero = AddressHash::new([0u8; 16]);
    assert_eq!(transport.identity_hash(), zero);
    assert_eq!(transport.destination_hash(), zero);
}

#[tokio::test]
async fn null_transport_shutdown_succeeds() {
    let transport = NullTransport::new();
    assert!(transport.shutdown().await.is_ok());
}

#[tokio::test]
async fn null_transport_as_dyn_mesh_transport() {
    // Verify NullTransport can be used as Arc<dyn MeshTransport>
    let transport: Arc<dyn MeshTransport> = Arc::new(NullTransport::new());
    assert!(!transport.is_connected());
    let result = transport.send_raw(AddressHash::new([1; 16]), b"test").await;
    assert!(matches!(result, Err(TransportError::Unavailable)));
}
