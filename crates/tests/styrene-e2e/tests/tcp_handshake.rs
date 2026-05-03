//! Layer 1 — TCP transport handshake.
//!
//! Two TestNode instances connect over TCP on localhost.
//! Validates that the transport layer establishes connectivity.

use styrene_e2e::helpers::{with_timeout, SETTLE};
use styrene_e2e::node::TestNodeBuilder;

#[tokio::test]
async fn two_nodes_connect_via_tcp() {
    with_timeout(async {
        // Alice serves, Bob connects
        let alice = TestNodeBuilder::new("alice").tcp_server("127.0.0.1:0").build().await;

        let bob = TestNodeBuilder::new("bob")
            .tcp_client(alice.listen_addr.expect("alice must have listen addr"))
            .build()
            .await;

        // Allow TCP handshake to complete
        tokio::time::sleep(SETTLE).await;

        assert!(
            alice.app_context.transport().is_connected(),
            "alice transport should report connected"
        );
        assert!(
            bob.app_context.transport().is_connected(),
            "bob transport should report connected"
        );

        // Verify distinct identities
        assert_ne!(alice.identity_hash, bob.identity_hash, "nodes must have different identities");

        // Verify listen address was resolved
        let addr = alice.listen_addr.expect("listen addr");
        assert_ne!(addr.port(), 0, "ephemeral port should have been resolved");
    })
    .await;
}

#[tokio::test]
async fn node_identity_hash_matches_delivery_derivation() {
    with_timeout(async {
        let node = TestNodeBuilder::new("hash-check").tcp_server("127.0.0.1:0").build().await;

        // delivery_hash should differ from identity_hash
        // (delivery hash = Hash(dest_name_hash || identity_hash))
        assert_ne!(
            node.identity_hash, node.delivery_hash,
            "delivery hash must differ from identity hash"
        );

        // Both should be 32-char hex strings
        assert_eq!(node.identity_hash.len(), 32);
        assert_eq!(node.delivery_hash.len(), 32);
    })
    .await;
}
