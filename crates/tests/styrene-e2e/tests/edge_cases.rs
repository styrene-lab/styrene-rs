//! Edge cases and failure scenarios.
//!
//! Tests behaviour under non-happy-path conditions: unknown peers,
//! late joiners, announce updates, message to self.

use std::time::Duration;
use styrene_e2e::helpers::{await_identity_resolved, await_inbound_count, with_timeout, SETTLE};
use styrene_e2e::node::TestNodeBuilder;

#[tokio::test]
async fn send_to_unknown_peer_fails_with_status() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice-fail").tcp_server("127.0.0.1:0").build().await;

        // No bob exists — alice tries to send to a fabricated hash
        let fake_hash = "deadbeefdeadbeefdeadbeefdeadbeef";
        let result = alice.send_chat(fake_hash, "hello void").await;

        // send_chat should either return an error or persist with "failed" status.
        // The delivery pipeline polls resolve_identity for 12s, then fails.
        match result {
            Err(_) => {
                // Expected — delivery failed, check store has the failed message
                let store = alice.app_context.store().lock().expect("lock");
                let messages = store.list_messages(100, None).expect("list");
                if let Some(msg) = messages.iter().find(|m| m.destination == fake_hash) {
                    assert!(
                        msg.receipt_status
                            .as_deref()
                            .map(|s| s.contains("failed"))
                            .unwrap_or(false),
                        "message to unknown peer should have 'failed' receipt status, got {:?}",
                        msg.receipt_status
                    );
                }
            }
            Ok(msg_id) => {
                // If send_chat returned Ok, the message should still show failed
                let store = alice.app_context.store().lock().expect("lock");
                let msg = store
                    .get_message(&msg_id)
                    .expect("query")
                    .expect("message should be persisted");
                assert!(
                    msg.receipt_status.as_deref().map(|s| s.contains("failed")).unwrap_or(false),
                    "message to unknown peer should have 'failed' receipt status, got {:?}",
                    msg.receipt_status
                );
            }
        }
    })
    .await;
}

#[tokio::test]
async fn late_joiner_discovers_existing_node() {
    with_timeout(async {
        // Alice starts and announces first
        let alice = TestNodeBuilder::new("alice-early").tcp_server("127.0.0.1:0").build().await;

        tokio::time::sleep(SETTLE).await;
        alice.announce().await;

        // Wait a bit, then Bob connects
        tokio::time::sleep(Duration::from_millis(200)).await;

        let bob = TestNodeBuilder::new("bob-late")
            .tcp_client(alice.listen_addr.expect("listen addr"))
            .build()
            .await;

        tokio::time::sleep(SETTLE).await;

        // Bob announces — alice should discover bob
        bob.announce().await;
        await_identity_resolved(&alice.app_context, &bob.delivery_addr, Duration::from_secs(10))
            .await;

        // But can bob discover alice? Alice announced before bob connected.
        // Alice needs to re-announce, or bob needs to have received the
        // announce that was broadcast when bob's TCP connection was up.
        // Let's have alice re-announce and verify bob gets it.
        alice.announce().await;
        await_identity_resolved(&bob.app_context, &alice.delivery_addr, Duration::from_secs(10))
            .await;

        // Now verify they can actually exchange messages
        alice.send_chat(&bob.delivery_hash, "late joiner msg").await.expect("send to late joiner");
        let received = await_inbound_count(&bob.app_context, 1, Duration::from_secs(15)).await;
        assert_eq!(received[0].content, "late joiner msg");
    })
    .await;
}

#[tokio::test]
async fn announce_updates_node_store_display_name() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice-name").tcp_server("127.0.0.1:0").build().await;

        let bob = TestNodeBuilder::new("bob-name")
            .tcp_client(alice.listen_addr.expect("listen addr"))
            .build()
            .await;

        tokio::time::sleep(SETTLE).await;

        // First announce from bob
        bob.announce().await;
        await_identity_resolved(&alice.app_context, &bob.delivery_addr, Duration::from_secs(10))
            .await;

        // Check initial node store state.
        // The announce worker keys by the delivery destination hash,
        // not the identity hash — use bob.delivery_hash for lookups.
        let nodes = alice.app_context.node_store().list(None).expect("list");
        assert!(!nodes.is_empty(), "should have at least one node");
        let first_announce_count = nodes
            .iter()
            .find(|n| n.identity_hash == bob.delivery_hash)
            .map(|n| n.announce_count)
            .unwrap_or(0);
        assert!(first_announce_count >= 1, "announce_count should be at least 1");

        // Second announce from bob — announce_count should increment
        tokio::time::sleep(Duration::from_millis(100)).await;
        bob.announce().await;
        tokio::time::sleep(Duration::from_millis(500)).await;

        let nodes = alice.app_context.node_store().list(None).expect("list");
        let second_announce_count = nodes
            .iter()
            .find(|n| n.identity_hash == bob.delivery_hash)
            .map(|n| n.announce_count)
            .unwrap_or(0);
        assert!(
            second_announce_count > first_announce_count,
            "announce_count should increment on re-announce: {} -> {}",
            first_announce_count,
            second_announce_count
        );

        // Verify last_seen updated
        let node = nodes
            .iter()
            .find(|n| n.identity_hash == bob.delivery_hash)
            .expect("bob should be in node store");
        assert!(node.last_seen >= node.first_seen, "last_seen should be >= first_seen");
    })
    .await;
}

#[tokio::test]
async fn three_nodes_linear_topology() {
    with_timeout(async {
        // A ↔ B ↔ C (hub-and-spoke through B)
        let node_b = TestNodeBuilder::new("hub-b").tcp_server("127.0.0.1:0").build().await;

        let node_a = TestNodeBuilder::new("spoke-a")
            .tcp_client(node_b.listen_addr.expect("listen addr"))
            .build()
            .await;

        let node_c = TestNodeBuilder::new("spoke-c")
            .tcp_client(node_b.listen_addr.expect("listen addr"))
            .build()
            .await;

        tokio::time::sleep(SETTLE).await;

        // All three announce
        node_a.announce().await;
        node_b.announce().await;
        node_c.announce().await;

        // Wait for mutual discovery between A and B
        await_identity_resolved(
            &node_a.app_context,
            &node_b.delivery_addr,
            Duration::from_secs(10),
        )
        .await;
        await_identity_resolved(
            &node_b.app_context,
            &node_a.delivery_addr,
            Duration::from_secs(10),
        )
        .await;

        // Wait for mutual discovery between B and C
        await_identity_resolved(
            &node_b.app_context,
            &node_c.delivery_addr,
            Duration::from_secs(10),
        )
        .await;
        await_identity_resolved(
            &node_c.app_context,
            &node_b.delivery_addr,
            Duration::from_secs(10),
        )
        .await;

        // A → B should work (direct connection)
        node_a.send_chat(&node_b.delivery_hash, "a-to-b").await.expect("send a→b");
        let received = await_inbound_count(&node_b.app_context, 1, Duration::from_secs(15)).await;
        assert_eq!(received[0].content, "a-to-b");

        // C → B should work (direct connection)
        node_c.send_chat(&node_b.delivery_hash, "c-to-b").await.expect("send c→b");
        let received = await_inbound_count(&node_b.app_context, 2, Duration::from_secs(15)).await;
        let c_msgs: Vec<_> = received.iter().filter(|m| m.content == "c-to-b").collect();
        assert_eq!(c_msgs.len(), 1, "B should have received c-to-b");

        // Verify B's store has messages from both A and C with correct attribution
        {
            let store = node_b.app_context.store().lock().expect("lock");
            let all = store.list_messages(100, None).expect("list");
            let inbound: Vec<_> = all.iter().filter(|m| m.direction == "in").collect();

            let sources: Vec<&str> = inbound.iter().map(|m| m.source.as_str()).collect();
            assert!(
                sources.contains(&node_a.identity_hash.as_str()),
                "B should have message from A"
            );
            assert!(
                sources.contains(&node_c.identity_hash.as_str()),
                "B should have message from C"
            );
        }
    })
    .await;
}
