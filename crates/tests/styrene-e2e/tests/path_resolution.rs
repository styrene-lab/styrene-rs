//! Path resolution scenarios.
//!
//! Tests that the path table correctly populates from announces,
//! that paths are bidirectional, that unknown destinations have no path,
//! and that paths are reachable through multi-hop topologies.

use rns_core::hash::AddressHash;
use std::time::Duration;
use styrene_e2e::helpers::{await_identity_resolved, await_path, with_timeout, SETTLE};
use styrene_e2e::node::TestNodeBuilder;

#[tokio::test]
async fn bidirectional_path_resolution() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice").tcp_server("127.0.0.1:0").build().await;

        let bob = TestNodeBuilder::new("bob")
            .tcp_client(alice.listen_addr.expect("listen addr"))
            .build()
            .await;

        tokio::time::sleep(SETTLE).await;
        alice.announce().await;
        bob.announce().await;

        // Wait for mutual discovery
        await_identity_resolved(&bob.app_context, &alice.delivery_addr, Duration::from_secs(10))
            .await;
        await_identity_resolved(&alice.app_context, &bob.delivery_addr, Duration::from_secs(10))
            .await;

        // Bob → Alice path
        bob.app_context.transport().request_path(&alice.delivery_addr).await;
        await_path(&bob.app_context, &alice.delivery_addr, Duration::from_secs(10)).await;

        let (hops_b2a, next_hop_b2a) = bob
            .app_context
            .transport()
            .query_path(&alice.delivery_addr)
            .await
            .expect("bob should have path to alice");

        // Alice → Bob path
        alice.app_context.transport().request_path(&bob.delivery_addr).await;
        await_path(&alice.app_context, &bob.delivery_addr, Duration::from_secs(10)).await;

        let (hops_a2b, next_hop_a2b) = alice
            .app_context
            .transport()
            .query_path(&bob.delivery_addr)
            .await
            .expect("alice should have path to bob");

        // Both should be direct (low hop count)
        assert!(hops_b2a <= 2, "bob→alice should be direct, got {} hops", hops_b2a);
        assert!(hops_a2b <= 2, "alice→bob should be direct, got {} hops", hops_a2b);

        // Next-hop interface addresses should be non-zero (real interfaces)
        assert_ne!(next_hop_b2a.as_slice(), &[0u8; 16], "next-hop interface should be non-zero");
        assert_ne!(next_hop_a2b.as_slice(), &[0u8; 16], "next-hop interface should be non-zero");
    })
    .await;
}

#[tokio::test]
async fn no_path_for_unknown_destination() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice-nopath").tcp_server("127.0.0.1:0").build().await;

        tokio::time::sleep(SETTLE).await;

        // Query path for a fabricated destination — should be None
        let fake_dest = AddressHash::new([0xDE; 16]);
        let path = alice.app_context.transport().query_path(&fake_dest).await;
        assert!(path.is_none(), "should have no path to unknown destination");

        // Identity resolution should also fail
        let identity = alice.app_context.transport().resolve_identity(&fake_dest).await;
        assert!(identity.is_none(), "should not resolve identity for unknown destination");
    })
    .await;
}

#[tokio::test]
async fn path_through_hub_node() {
    with_timeout(async {
        // A ↔ Hub ↔ C — both spokes connect to hub
        let hub = TestNodeBuilder::new("hub").tcp_server("127.0.0.1:0").build().await;

        let spoke_a = TestNodeBuilder::new("spoke-a")
            .tcp_client(hub.listen_addr.expect("hub addr"))
            .build()
            .await;

        let spoke_c = TestNodeBuilder::new("spoke-c")
            .tcp_client(hub.listen_addr.expect("hub addr"))
            .build()
            .await;

        tokio::time::sleep(SETTLE).await;
        spoke_a.announce().await;
        hub.announce().await;
        spoke_c.announce().await;

        // Hub should discover both spokes
        await_identity_resolved(&hub.app_context, &spoke_a.delivery_addr, Duration::from_secs(10))
            .await;
        await_identity_resolved(&hub.app_context, &spoke_c.delivery_addr, Duration::from_secs(10))
            .await;

        // Hub should have paths to both spokes
        hub.app_context.transport().request_path(&spoke_a.delivery_addr).await;
        hub.app_context.transport().request_path(&spoke_c.delivery_addr).await;

        await_path(&hub.app_context, &spoke_a.delivery_addr, Duration::from_secs(10)).await;
        await_path(&hub.app_context, &spoke_c.delivery_addr, Duration::from_secs(10)).await;

        // Hub's paths to the two spokes should use different interfaces
        // (each spoke connected on a separate accepted socket)
        let (_, iface_a) = hub
            .app_context
            .transport()
            .query_path(&spoke_a.delivery_addr)
            .await
            .expect("hub→spoke_a path");
        let (_, iface_c) = hub
            .app_context
            .transport()
            .query_path(&spoke_c.delivery_addr)
            .await
            .expect("hub→spoke_c path");

        assert_ne!(
            iface_a.as_slice(),
            iface_c.as_slice(),
            "paths to different spokes should traverse different interfaces"
        );
    })
    .await;
}

#[tokio::test]
async fn identity_resolution_returns_correct_public_key() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice-resolve").tcp_server("127.0.0.1:0").build().await;

        let bob = TestNodeBuilder::new("bob-resolve")
            .tcp_client(alice.listen_addr.expect("listen addr"))
            .build()
            .await;

        tokio::time::sleep(SETTLE).await;
        bob.announce().await;

        await_identity_resolved(&alice.app_context, &bob.delivery_addr, Duration::from_secs(10))
            .await;

        // Resolved identity's public key should match bob's actual keys
        let resolved = alice
            .app_context
            .transport()
            .resolve_identity(&bob.delivery_addr)
            .await
            .expect("should resolve");

        assert_eq!(
            resolved.public_key_bytes(),
            bob.identity.as_identity().public_key_bytes(),
            "DH public key mismatch"
        );
        assert_eq!(
            resolved.verifying_key_bytes(),
            bob.identity.as_identity().verifying_key_bytes(),
            "Ed25519 verifying key mismatch"
        );
    })
    .await;
}
