//! Concurrent and stress scenarios.
//!
//! Tests that exercise simultaneous operations: bidirectional sends,
//! rapid-fire messages, and concurrent link establishment.

use std::time::Duration;
use styrene_e2e::helpers::{await_inbound_count, two_connected_nodes, with_timeout};

#[tokio::test]
async fn bidirectional_simultaneous_send() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-bidir", "bob-bidir").await;

        // Both send to each other concurrently
        let alice_send = {
            let delivery = bob.delivery_hash.clone();
            let ctx = alice.app_context.clone();
            tokio::spawn(
                async move { ctx.messaging().send_chat(&delivery, "from-alice", None).await },
            )
        };

        let bob_send = {
            let delivery = alice.delivery_hash.clone();
            let ctx = bob.app_context.clone();
            tokio::spawn(
                async move { ctx.messaging().send_chat(&delivery, "from-bob", None).await },
            )
        };

        // Both sends should succeed
        alice_send.await.expect("join alice").expect("alice send");
        bob_send.await.expect("join bob").expect("bob send");

        // Both should receive the other's message
        let alice_inbox = await_inbound_count(&alice.app_context, 1, Duration::from_secs(15)).await;
        assert_eq!(alice_inbox[0].content, "from-bob");

        let bob_inbox = await_inbound_count(&bob.app_context, 1, Duration::from_secs(15)).await;
        assert_eq!(bob_inbox[0].content, "from-alice");
    })
    .await;
}

#[tokio::test]
async fn rapid_fire_messages_all_delivered() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-rapid", "bob-rapid").await;

        // Alice sends 5 messages sequentially, waiting for delivery.
        let count = 5usize;
        let mut sent_ids = Vec::new();
        for i in 0..count {
            let msg = format!("rapid-{}", i);
            let id = alice
                .send_chat(&bob.delivery_hash, &msg)
                .await
                .unwrap_or_else(|e| panic!("send rapid-{} failed: {}", i, e));
            sent_ids.push(id);
            await_inbound_count(&bob.app_context, i + 1, Duration::from_secs(15)).await;
        }

        // All 5 messages should arrive at Bob with correct content
        let received = await_inbound_count(&bob.app_context, count, Duration::from_secs(5)).await;

        let contents: Vec<&str> = received.iter().map(|m| m.content.as_str()).collect();
        for i in 0..count {
            let expected = format!("rapid-{}", i);
            assert!(
                contents.contains(&expected.as_str()),
                "missing message '{}' in received: {:?}",
                expected,
                contents
            );
        }

        for msg in &received {
            assert_eq!(msg.direction, "in");
            assert_eq!(msg.source, alice.identity_hash);
        }

        // All message IDs should be unique (content-hash based)
        let unique: std::collections::HashSet<&str> = sent_ids.iter().map(|s| s.as_str()).collect();
        assert_eq!(unique.len(), count, "all message IDs should be unique");

        // Sender's store should have all outbound records
        let store = alice.app_context.store().lock().expect("lock");
        let all = store.list_messages(100, None).expect("list");
        let outbound_count = all.iter().filter(|m| m.direction == "out").count();
        assert_eq!(
            outbound_count, count,
            "sender should have all {} outbound records persisted",
            count
        );
    })
    .await;
}

#[tokio::test]
async fn interleaved_bidirectional_conversation() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-interleave", "bob-interleave").await;

        // Interleave: A→B, B→A, A→B, B→A, A→B
        // Each send waits for delivery before the next.
        alice.send_chat(&bob.delivery_hash, "a1").await.expect("a1");
        await_inbound_count(&bob.app_context, 1, Duration::from_secs(15)).await;

        bob.send_chat(&alice.delivery_hash, "b1").await.expect("b1");
        await_inbound_count(&alice.app_context, 1, Duration::from_secs(15)).await;

        alice.send_chat(&bob.delivery_hash, "a2").await.expect("a2");
        await_inbound_count(&bob.app_context, 2, Duration::from_secs(15)).await;

        bob.send_chat(&alice.delivery_hash, "b2").await.expect("b2");
        await_inbound_count(&alice.app_context, 2, Duration::from_secs(15)).await;

        alice.send_chat(&bob.delivery_hash, "a3").await.expect("a3");
        await_inbound_count(&bob.app_context, 3, Duration::from_secs(15)).await;

        // Verify Bob received all 3 from alice
        {
            let store = bob.app_context.store().lock().expect("lock");
            let all = store.list_messages(100, None).expect("list");
            let inbound: Vec<_> = all.iter().filter(|m| m.direction == "in").collect();
            assert_eq!(inbound.len(), 3, "bob should have 3 inbound");

            let in_contents: Vec<&str> = inbound.iter().map(|m| m.content.as_str()).collect();
            assert!(in_contents.contains(&"a1"));
            assert!(in_contents.contains(&"a2"));
            assert!(in_contents.contains(&"a3"));
        }

        // Verify Alice received both from bob
        {
            let store = alice.app_context.store().lock().expect("lock");
            let all = store.list_messages(100, None).expect("list");
            let inbound: Vec<_> = all.iter().filter(|m| m.direction == "in").collect();
            assert_eq!(inbound.len(), 2, "alice should have 2 inbound");

            let in_contents: Vec<&str> = inbound.iter().map(|m| m.content.as_str()).collect();
            assert!(in_contents.contains(&"b1"));
            assert!(in_contents.contains(&"b2"));
        }

        // Verify exact outbound counts
        {
            let store = bob.app_context.store().lock().expect("lock");
            let all = store.list_messages(100, None).expect("list");
            let outbound: Vec<_> = all.iter().filter(|m| m.direction == "out").collect();
            assert_eq!(outbound.len(), 2, "bob should have exactly 2 outbound records");
        }

        {
            let store = alice.app_context.store().lock().expect("lock");
            let all = store.list_messages(100, None).expect("list");
            let outbound: Vec<_> = all.iter().filter(|m| m.direction == "out").collect();
            assert_eq!(outbound.len(), 3, "alice should have exactly 3 outbound records");
        }
    })
    .await;
}
