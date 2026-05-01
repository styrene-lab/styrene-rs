//! Layer 2 — Announce exchange and peer discovery.
//!
//! After TCP connection, nodes announce themselves and discover each other.

use std::time::Duration;
use styrene_e2e::helpers::{with_timeout, await_identity_resolved, SETTLE};
use styrene_e2e::node::TestNodeBuilder;

#[tokio::test]
async fn mutual_announce_exchange() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let bob = TestNodeBuilder::new("bob")
            .tcp_client(alice.listen_addr.expect("listen addr"))
            .build()
            .await;

        // Let TCP settle
        tokio::time::sleep(SETTLE).await;

        // Both announce
        alice.announce().await;
        bob.announce().await;

        // Wait for each to resolve the other's delivery destination identity
        await_identity_resolved(
            &alice.app_context,
            &bob.delivery_addr,
            Duration::from_secs(10),
        )
        .await;

        await_identity_resolved(
            &bob.app_context,
            &alice.delivery_addr,
            Duration::from_secs(10),
        )
        .await;

        // Verify identities resolved correctly
        let bob_identity = alice
            .app_context
            .transport()
            .resolve_identity(&bob.delivery_addr)
            .await
            .expect("alice should resolve bob's identity");

        let alice_identity = bob
            .app_context
            .transport()
            .resolve_identity(&alice.delivery_addr)
            .await
            .expect("bob should resolve alice's identity");

        // Public keys should match
        assert_eq!(
            bob_identity.public_key_bytes(),
            bob.identity.as_identity().public_key_bytes(),
            "resolved identity must match bob's actual public key"
        );
        assert_eq!(
            alice_identity.public_key_bytes(),
            alice.identity.as_identity().public_key_bytes(),
            "resolved identity must match alice's actual public key"
        );
    })
    .await;
}

#[tokio::test]
async fn announce_populates_node_store() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice-ns")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let bob = TestNodeBuilder::new("bob-ns")
            .tcp_client(alice.listen_addr.expect("listen addr"))
            .build()
            .await;

        tokio::time::sleep(SETTLE).await;

        bob.announce().await;

        // Wait for alice to resolve bob's identity (proves announce arrived)
        await_identity_resolved(
            &alice.app_context,
            &bob.delivery_addr,
            Duration::from_secs(10),
        )
        .await;

        // Check that the node store has bob registered
        let nodes = alice
            .app_context
            .node_store()
            .list(None)
            .expect("list nodes");
        assert!(
            !nodes.is_empty(),
            "node store should contain at least one peer after announce"
        );
    })
    .await;
}
