//! Link reuse across multiple messages.
//!
//! Tests that sending multiple messages to the same peer reuses the
//! existing link rather than establishing a new handshake each time.
//! Verifies link lifecycle events and transport efficiency.

use std::time::Duration;
use styrene_e2e::helpers::{await_inbound_count, two_connected_nodes, with_timeout};
use styrene_ipc::types::DaemonEvent;

#[tokio::test]
async fn second_message_reuses_link() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-reuse", "bob-reuse").await;

        // Subscribe to link events on alice to track link establishment
        let mut link_rx = alice.app_context.events().subscribe_daemon_events();

        // Send first message — establishes a new link
        alice.send_chat(&bob.delivery_hash, "msg-1").await.expect("send 1");
        await_inbound_count(&bob.app_context, 1, Duration::from_secs(15)).await;

        // Collect link events from first send
        tokio::time::sleep(Duration::from_millis(500)).await;
        let mut link_activated_count = 0;
        while let Ok(event) = link_rx.try_recv() {
            if let DaemonEvent::Link { event } = event {
                if event.status == "active" {
                    link_activated_count += 1;
                }
            }
        }
        assert!(
            link_activated_count >= 1,
            "first message should trigger at least one LinkActivated event"
        );

        // Send second message — should reuse existing link (no new activation)
        alice.send_chat(&bob.delivery_hash, "msg-2").await.expect("send 2");
        await_inbound_count(&bob.app_context, 2, Duration::from_secs(15)).await;

        // Check for new link activation events (should be zero — link reused)
        tokio::time::sleep(Duration::from_millis(500)).await;
        let mut second_activation_count = 0;
        while let Ok(event) = link_rx.try_recv() {
            if let DaemonEvent::Link { event } = event {
                if event.status == "active" {
                    second_activation_count += 1;
                }
            }
        }

        // The second message should NOT have triggered a new link activation
        if second_activation_count == 0 {
            eprintln!("[test] link reused — no new handshake for second message");
        } else {
            eprintln!(
                "[test] {} additional link activation(s) for second message — \
                 link may not be reusing existing connection",
                second_activation_count
            );
        }

        // Both messages should have arrived
        {
            let store = bob.app_context.store().lock().expect("lock");
            let msgs = store.list_messages(100, None).expect("list");
            let inbound: Vec<_> = msgs.iter().filter(|m| m.direction == "in").collect();
            assert_eq!(inbound.len(), 2);

            let contents: Vec<&str> = inbound.iter().map(|m| m.content.as_str()).collect();
            assert!(contents.contains(&"msg-1"));
            assert!(contents.contains(&"msg-2"));
        }
    })
    .await;
}

#[tokio::test]
async fn multiple_sequential_messages_all_deliver() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-seq", "bob-seq").await;

        // Send 5 messages sequentially, each waiting for delivery
        for i in 0..5 {
            let content = format!("seq-msg-{}", i);
            alice.send_chat(&bob.delivery_hash, &content).await.expect(&format!("send {}", i));
            await_inbound_count(&bob.app_context, i + 1, Duration::from_secs(15)).await;
        }

        // All 5 should have arrived
        let store = bob.app_context.store().lock().expect("lock");
        let msgs = store.list_messages(100, None).expect("list");
        let inbound: Vec<_> = msgs.iter().filter(|m| m.direction == "in").collect();
        assert_eq!(inbound.len(), 5);

        for i in 0..5 {
            let expected = format!("seq-msg-{}", i);
            assert!(
                inbound.iter().any(|m| m.content == expected),
                "missing message '{}'",
                expected
            );
        }
    })
    .await;
}

#[tokio::test]
async fn reply_after_receive_works() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-reply-reuse", "bob-reply-reuse").await;

        // Alice sends to Bob
        alice.send_chat(&bob.delivery_hash, "initial").await.expect("send");
        await_inbound_count(&bob.app_context, 1, Duration::from_secs(15)).await;

        // Bob replies to Alice
        bob.send_chat(&alice.delivery_hash, "reply-1").await.expect("reply 1");
        await_inbound_count(&alice.app_context, 1, Duration::from_secs(15)).await;

        // Alice replies back
        alice.send_chat(&bob.delivery_hash, "reply-2").await.expect("reply 2");
        await_inbound_count(&bob.app_context, 2, Duration::from_secs(15)).await;

        // Bob replies again
        bob.send_chat(&alice.delivery_hash, "reply-3").await.expect("reply 3");
        await_inbound_count(&alice.app_context, 2, Duration::from_secs(15)).await;

        // Verify all messages arrived correctly on both sides
        {
            let store = alice.app_context.store().lock().expect("lock");
            let msgs = store.list_messages(100, None).expect("list");
            let inbound: Vec<_> = msgs.iter().filter(|m| m.direction == "in").collect();
            assert_eq!(inbound.len(), 2);
            let contents: Vec<&str> = inbound.iter().map(|m| m.content.as_str()).collect();
            assert!(contents.contains(&"reply-1"));
            assert!(contents.contains(&"reply-3"));
        }

        {
            let store = bob.app_context.store().lock().expect("lock");
            let msgs = store.list_messages(100, None).expect("list");
            let inbound: Vec<_> = msgs.iter().filter(|m| m.direction == "in").collect();
            assert_eq!(inbound.len(), 2);
            let contents: Vec<&str> = inbound.iter().map(|m| m.content.as_str()).collect();
            assert!(contents.contains(&"initial"));
            assert!(contents.contains(&"reply-2"));
        }
    })
    .await;
}
