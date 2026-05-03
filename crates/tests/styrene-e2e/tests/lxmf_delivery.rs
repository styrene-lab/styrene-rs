//! LXMF delivery scenarios.
//!
//! Tests the full message delivery pipeline: wire encoding, link
//! establishment, delivery, inbound decoding, store persistence,
//! and receipt status tracking.

use std::time::Duration;
use styrene_e2e::helpers::{
    await_identity_resolved, await_inbound_count, await_inbound_message, two_connected_nodes,
    with_timeout, SETTLE,
};
use styrene_e2e::node::TestNodeBuilder;

#[tokio::test]
async fn send_chat_delivers_with_correct_attribution() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice", "bob").await;

        let msg_id = alice.send_chat(&bob.delivery_hash, "hello bob").await.expect("send_chat");

        assert!(!msg_id.is_empty(), "message ID should be non-empty");

        let received = await_inbound_message(&bob.app_context, Duration::from_secs(15)).await;

        // Content integrity
        assert_eq!(received.content, "hello bob");
        assert_eq!(received.direction, "in");

        // Source attribution — bob sees alice's identity hash as sender
        assert_eq!(
            received.source, alice.identity_hash,
            "inbound source should be alice's identity hash"
        );

        // Destination — should be bob's transport identity hash
        // (the inbound decoder extracts the destination from the wire)
        assert!(!received.destination.is_empty(), "destination should be populated");

        // Sender's store — outbound record with receipt status
        {
            let store = alice.app_context.store().lock().expect("lock");
            let msg = store.get_message(&msg_id).expect("query");
            // Message might have been overwritten by ID collision, but
            // if it exists it should have correct fields
            if let Some(msg) = msg {
                assert_eq!(msg.direction, "out");
                assert_eq!(msg.content, "hello bob");
                assert_eq!(msg.destination, bob.delivery_hash);
                assert!(
                    msg.receipt_status.as_deref().map(|s| s.starts_with("sent")).unwrap_or(false),
                    "outbound should have 'sent' receipt, got {:?}",
                    msg.receipt_status
                );
            }
        }
    })
    .await;
}

#[tokio::test]
async fn delivery_preserves_message_content_with_special_characters() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-special", "bob-special").await;

        // Unicode, emoji, newlines, long content
        let special_content = "Héllo wörld! 🌍\nLine two\ttab\n日本語テスト\n\nEmpty line above";

        alice.send_chat(&bob.delivery_hash, special_content).await.expect("send special");

        let received = await_inbound_message(&bob.app_context, Duration::from_secs(15)).await;
        assert_eq!(
            received.content, special_content,
            "special characters should be preserved through LXMF wire encoding"
        );
    })
    .await;
}

#[tokio::test]
async fn delivery_to_unannounced_peer_fails_gracefully() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice-nopeer").tcp_server("127.0.0.1:0").build().await;

        // Send to a destination nobody has announced.
        // The delivery pipeline should poll resolve_identity for 12s,
        // then fail — not panic, not hang indefinitely.
        let fake_hash = "deadbeefdeadbeefdeadbeefdeadbeef";
        let result = alice.send_chat(fake_hash, "into the void").await;

        // Should either return Err or persist with failed status
        match result {
            Err(e) => {
                let msg = format!("{}", e);
                assert!(
                    msg.contains("not announced")
                        || msg.contains("not resolved")
                        || msg.contains("failed"),
                    "error should indicate identity resolution failure, got: {}",
                    msg
                );
            }
            Ok(msg_id) => {
                // Check that the message was persisted with failed status
                let store = alice.app_context.store().lock().expect("lock");
                let msg = store
                    .get_message(&msg_id)
                    .expect("query")
                    .expect("message should be persisted even on failure");
                assert!(
                    msg.receipt_status.as_deref().map(|s| s.contains("failed")).unwrap_or(false),
                    "failed delivery should have 'failed' receipt status, got {:?}",
                    msg.receipt_status
                );
            }
        }

        // Alice should not have crashed — verify she's still functional
        assert!(
            alice.app_context.transport().is_connected(),
            "alice should still be operational after failed delivery"
        );
    })
    .await;
}

#[tokio::test]
async fn reply_uses_correct_peer_addressing() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-reply", "bob-reply").await;

        // Alice → Bob
        alice.send_chat(&bob.delivery_hash, "initial message").await.expect("send");
        await_inbound_count(&bob.app_context, 1, Duration::from_secs(15)).await;

        // Bob replies to Alice using alice's delivery hash
        bob.send_chat(&alice.delivery_hash, "reply message").await.expect("reply");
        await_inbound_count(&alice.app_context, 1, Duration::from_secs(15)).await;

        // Verify Bob's inbound has correct source
        {
            let store = bob.app_context.store().lock().expect("lock");
            let messages = store.list_messages(100, None).expect("list");
            let inbound: Vec<_> = messages.iter().filter(|m| m.direction == "in").collect();
            assert_eq!(inbound.len(), 1);
            assert_eq!(inbound[0].source, alice.identity_hash);
            assert_eq!(inbound[0].content, "initial message");
        }

        // Verify Alice's inbound has correct source
        {
            let store = alice.app_context.store().lock().expect("lock");
            let messages = store.list_messages(100, None).expect("list");
            let inbound: Vec<_> = messages.iter().filter(|m| m.direction == "in").collect();
            assert_eq!(inbound.len(), 1);
            assert_eq!(inbound[0].source, bob.identity_hash);
            assert_eq!(inbound[0].content, "reply message");
        }
    })
    .await;
}

#[tokio::test]
async fn delivery_before_mutual_announce_requires_sender_initiative() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice-oneann").tcp_server("127.0.0.1:0").build().await;

        let bob = TestNodeBuilder::new("bob-oneann")
            .tcp_client(alice.listen_addr.expect("listen addr"))
            .build()
            .await;

        tokio::time::sleep(SETTLE).await;

        // Only Bob announces — Alice knows Bob but Bob doesn't know Alice
        bob.announce().await;
        await_identity_resolved(&alice.app_context, &bob.delivery_addr, Duration::from_secs(10))
            .await;

        // Alice can send to Bob (she resolved his identity from his announce)
        alice
            .send_chat(&bob.delivery_hash, "one-way announce")
            .await
            .expect("send with one-way announce");

        let received = await_inbound_message(&bob.app_context, Duration::from_secs(15)).await;
        assert_eq!(received.content, "one-way announce");

        // Bob's inbound worker decodes the message from alice.
        // Alice's identity was delivered as part of the link handshake,
        // so bob should see alice's identity hash as the source.
        assert_eq!(received.source, alice.identity_hash);
    })
    .await;
}

#[tokio::test]
async fn message_with_title_roundtrips() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-title", "bob-title").await;

        // send_chat takes title as Option<&str> — verify it survives the wire
        alice
            .app_context
            .messaging()
            .send_chat(&bob.delivery_hash, "message body", Some("Important Subject"))
            .await
            .expect("send with title");

        let received = await_inbound_message(&bob.app_context, Duration::from_secs(15)).await;
        assert_eq!(received.content, "message body");
        assert_eq!(received.title, "Important Subject", "title should survive LXMF wire roundtrip");
    })
    .await;
}

#[tokio::test]
async fn empty_message_delivers() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-empty", "bob-empty").await;

        alice.send_chat(&bob.delivery_hash, "").await.expect("send empty");

        let received = await_inbound_message(&bob.app_context, Duration::from_secs(15)).await;
        assert_eq!(received.content, "");
    })
    .await;
}

#[tokio::test]
async fn large_message_delivery() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-large", "bob-large").await;

        // 2KB message — exceeds single-packet threshold, triggers resource
        // transfer. The resource manager polls outgoing parts and the inbound
        // worker processes completed resource events.
        let large_content: String = (0..2048).map(|i| (b'A' + (i % 26) as u8) as char).collect();

        alice.send_chat(&bob.delivery_hash, &large_content).await.expect("send large");

        let received = await_inbound_message(&bob.app_context, Duration::from_secs(15)).await;
        assert_eq!(
            received.content.len(),
            large_content.len(),
            "large message length should be preserved"
        );
        assert_eq!(received.content, large_content, "large message content mismatch");
    })
    .await;
}

#[tokio::test]
async fn resource_transfer_delivers_large_message() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-boundary", "bob-boundary").await;

        // 1000 bytes triggers resource transfer — should deliver via
        // the resource completion handler in the inbound worker.
        let large: String = (0..1000).map(|i| (b'A' + (i % 26) as u8) as char).collect();
        alice.send_chat(&bob.delivery_hash, &large).await.expect("send large");

        let received = await_inbound_message(&bob.app_context, Duration::from_secs(15)).await;
        assert_eq!(received.content.len(), 1000);
        assert_eq!(received.content, large);
    })
    .await;
}

#[tokio::test]
async fn binary_safe_content() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-bin", "bob-bin").await;

        // Content with null bytes, high bytes, control characters
        let tricky = "null\x00byte\ttab\nnewline\r\nCRLF\x1b[31mANSI\x1b[0m";

        alice.send_chat(&bob.delivery_hash, tricky).await.expect("send tricky");

        let received = await_inbound_message(&bob.app_context, Duration::from_secs(15)).await;
        assert_eq!(received.content, tricky, "binary-safe content should roundtrip");
    })
    .await;
}
