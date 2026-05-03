//! Conversation flow — multi-message exchange with ordering and attribution.
//!
//! Alice sends 3 messages, Bob replies 2, Alice sends 1 more.
//! Verifies: all messages arrive on the receiver side with correct
//! source/destination attribution, direction flags, and content.
//!
//! NOTE: Outbound record counts on the sender side may be lower than
//! the actual send count due to a known message ID collision bug
//! (IDs derived from first 8 bytes of LXMF wire can collide within
//! the same second, and INSERT OR REPLACE overwrites earlier records).

use std::time::Duration;
use styrene_e2e::helpers::{await_inbound_count, two_connected_nodes, with_timeout};

#[tokio::test]
async fn multi_message_conversation() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice", "bob").await;

        // Alice → Bob: 3 messages in sequence, waiting for delivery
        alice.send_chat(&bob.delivery_hash, "msg-1-from-alice").await.expect("send 1");
        await_inbound_count(&bob.app_context, 1, Duration::from_secs(15)).await;

        alice.send_chat(&bob.delivery_hash, "msg-2-from-alice").await.expect("send 2");
        await_inbound_count(&bob.app_context, 2, Duration::from_secs(15)).await;

        alice.send_chat(&bob.delivery_hash, "msg-3-from-alice").await.expect("send 3");
        await_inbound_count(&bob.app_context, 3, Duration::from_secs(15)).await;

        // Bob → Alice: 2 replies
        bob.send_chat(&alice.delivery_hash, "reply-1-from-bob").await.expect("reply 1");
        await_inbound_count(&alice.app_context, 1, Duration::from_secs(15)).await;

        bob.send_chat(&alice.delivery_hash, "reply-2-from-bob").await.expect("reply 2");
        await_inbound_count(&alice.app_context, 2, Duration::from_secs(15)).await;

        // Alice → Bob: 1 more
        alice.send_chat(&bob.delivery_hash, "msg-4-from-alice").await.expect("send 4");
        await_inbound_count(&bob.app_context, 4, Duration::from_secs(15)).await;

        // ── Verify Bob's inbound ───────────────────────────────────────
        {
            let store = bob.app_context.store().lock().expect("lock");
            let all = store.list_messages(100, None).expect("list");

            let inbound: Vec<_> = all.iter().filter(|m| m.direction == "in").collect();
            assert_eq!(inbound.len(), 4, "bob should have 4 inbound messages");

            // All inbound came from alice's identity hash
            for msg in &inbound {
                assert_eq!(
                    msg.source, alice.identity_hash,
                    "inbound source should be alice's identity hash, got {}",
                    msg.source
                );
            }

            // Verify all 4 inbound contents present
            let inbound_contents: Vec<&str> = inbound.iter().map(|m| m.content.as_str()).collect();
            assert!(inbound_contents.contains(&"msg-1-from-alice"));
            assert!(inbound_contents.contains(&"msg-2-from-alice"));
            assert!(inbound_contents.contains(&"msg-3-from-alice"));
            assert!(inbound_contents.contains(&"msg-4-from-alice"));
        }

        // ── Verify Alice's inbound ─────────────────────────────────────
        {
            let store = alice.app_context.store().lock().expect("lock");
            let all = store.list_messages(100, None).expect("list");

            let inbound: Vec<_> = all.iter().filter(|m| m.direction == "in").collect();
            assert_eq!(inbound.len(), 2, "alice should have 2 inbound messages");

            // All inbound came from bob
            for msg in &inbound {
                assert_eq!(msg.source, bob.identity_hash);
            }

            let inbound_contents: Vec<&str> = inbound.iter().map(|m| m.content.as_str()).collect();
            assert!(inbound_contents.contains(&"reply-1-from-bob"));
            assert!(inbound_contents.contains(&"reply-2-from-bob"));
        }

        // ── Verify outbound records with exact counts ──────────────────
        {
            let store = alice.app_context.store().lock().expect("lock");
            let all = store.list_messages(100, None).expect("list");
            let outbound: Vec<_> = all.iter().filter(|m| m.direction == "out").collect();
            assert_eq!(outbound.len(), 4, "alice should have exactly 4 outbound records");
            for msg in &outbound {
                assert_eq!(msg.destination, bob.delivery_hash);
                assert!(msg.receipt_status.is_some(), "outbound should have receipt status");
            }
        }

        {
            let store = bob.app_context.store().lock().expect("lock");
            let all = store.list_messages(100, None).expect("list");
            let outbound: Vec<_> = all.iter().filter(|m| m.direction == "out").collect();
            assert_eq!(outbound.len(), 2, "bob should have exactly 2 outbound records");
            for msg in &outbound {
                assert_eq!(msg.destination, alice.delivery_hash);
            }
        }
    })
    .await;
}

#[tokio::test]
async fn message_ids_are_unique_across_conversation() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-uid", "bob-uid").await;

        // Send 3 messages with delivery waits between each.
        // The ID collision bug means IDs derived from wire bytes may
        // repeat within the same second. This test documents that.
        let id1 = alice.send_chat(&bob.delivery_hash, "first").await.expect("send 1");
        await_inbound_count(&bob.app_context, 1, Duration::from_secs(15)).await;

        let id2 = alice.send_chat(&bob.delivery_hash, "second").await.expect("send 2");
        await_inbound_count(&bob.app_context, 2, Duration::from_secs(15)).await;

        let id3 = alice.send_chat(&bob.delivery_hash, "third").await.expect("send 3");
        await_inbound_count(&bob.app_context, 3, Duration::from_secs(15)).await;

        // All 3 should arrive at bob regardless of ID uniqueness
        let received = await_inbound_count(&bob.app_context, 3, Duration::from_secs(5)).await;
        let contents: Vec<&str> = received.iter().map(|m| m.content.as_str()).collect();
        assert!(contents.contains(&"first"));
        assert!(contents.contains(&"second"));
        assert!(contents.contains(&"third"));

        // All IDs must be unique (content-hash based)
        assert_ne!(id1, id2, "message IDs must be unique");
        assert_ne!(id2, id3, "message IDs must be unique");
        assert_ne!(id1, id3, "message IDs must be unique");
    })
    .await;
}
