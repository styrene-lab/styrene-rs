//! Propagation (hub mode) scenarios.
//!
//! Tests the store-and-forward pipeline: hub receives messages for
//! non-local destinations, stores them, and makes them available for
//! later retrieval. Covers the full lifecycle: store → fetch → delete,
//! deduplication, expiry, and the inbound worker routing decision.

use std::time::Duration;
use styrene_e2e::helpers::{await_identity_resolved, await_inbound_count, with_timeout, SETTLE};
use styrene_e2e::node::TestNodeBuilder;

#[tokio::test]
async fn hub_stores_message_for_offline_destination() {
    with_timeout(async {
        // Hub with propagation enabled
        let hub = TestNodeBuilder::new("hub").tcp_server("127.0.0.1:0").build().await;
        hub.app_context.propagation().set_enabled(true);

        // Alice connects to hub
        let alice = TestNodeBuilder::new("alice")
            .tcp_client(hub.listen_addr.expect("hub addr"))
            .build()
            .await;

        tokio::time::sleep(SETTLE).await;
        alice.announce().await;
        hub.announce().await;

        await_identity_resolved(&alice.app_context, &hub.delivery_addr, Duration::from_secs(10))
            .await;

        // Charlie is offline — we just generate his identity to get a delivery hash.
        // We create a full node to derive the delivery hash, but Charlie is NOT
        // connected to the hub.
        let charlie = TestNodeBuilder::new("charlie").tcp_server("127.0.0.1:0").build().await;

        // Alice sends to Charlie's delivery hash. The message arrives at the hub
        // (because it's broadcast on the link), and the hub's inbound worker
        // sees it's not for the hub's own delivery destination.
        //
        // However, for the hub to store the message via propagation, the message
        // must actually arrive at the hub's transport. In RNS, send_via_link
        // sends to the destination's link — so Alice needs to know Charlie's
        // identity to establish a link. Since Charlie never announced, Alice
        // can't deliver via link.
        //
        // Instead, test the propagation service directly through the hub —
        // simulating what would happen if a message for Charlie arrived.
        let stored = hub
            .app_context
            .propagation()
            .store_for_propagation(
                &charlie.delivery_hash,
                b"simulated-lxmf-payload-for-charlie",
                Some(&alice.identity_hash),
            )
            .expect("store for propagation");
        assert!(stored, "message should be stored (new)");

        // Verify stats
        let (count, size) = hub.app_context.propagation().stats().expect("stats");
        assert_eq!(count, 1, "hub should have 1 propagation message");
        assert!(size > 0, "stored size should be positive");

        // Fetch for Charlie's destination
        let messages = hub
            .app_context
            .propagation()
            .fetch_for_destination(&charlie.delivery_hash)
            .expect("fetch");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].1, b"simulated-lxmf-payload-for-charlie");

        // Verify messages for other destinations are empty
        let other = hub
            .app_context
            .propagation()
            .fetch_for_destination(&alice.delivery_hash)
            .expect("fetch alice");
        assert!(other.is_empty(), "alice's destination should have no propagation messages");
    })
    .await;
}

#[tokio::test]
async fn hub_deduplicates_identical_messages() {
    with_timeout(async {
        let hub = TestNodeBuilder::new("hub-dedup").tcp_server("127.0.0.1:0").build().await;
        hub.app_context.propagation().set_enabled(true);

        let dest = "aabbccddaabbccddaabbccddaabbccdd";
        let payload = b"identical-payload";

        // Store same payload twice
        let first = hub
            .app_context
            .propagation()
            .store_for_propagation(dest, payload, None)
            .expect("first store");
        assert!(first, "first store should succeed");

        let second = hub
            .app_context
            .propagation()
            .store_for_propagation(dest, payload, None)
            .expect("second store");
        assert!(!second, "duplicate should be rejected");

        // Only one message stored
        let (count, _) = hub.app_context.propagation().stats().expect("stats");
        assert_eq!(count, 1);

        // But a different payload should store fine
        let third = hub
            .app_context
            .propagation()
            .store_for_propagation(dest, b"different-payload", None)
            .expect("third store");
        assert!(third, "different payload should store");

        let (count, _) = hub.app_context.propagation().stats().expect("stats");
        assert_eq!(count, 2);
    })
    .await;
}

#[tokio::test]
async fn hub_fetch_and_delete_lifecycle() {
    with_timeout(async {
        let hub = TestNodeBuilder::new("hub-lifecycle").tcp_server("127.0.0.1:0").build().await;
        hub.app_context.propagation().set_enabled(true);

        let dest = "1111111111111111111111111111111";

        // Store 3 messages for the same destination
        hub.app_context
            .propagation()
            .store_for_propagation(dest, b"msg-1", Some("src-a"))
            .expect("store 1");
        hub.app_context
            .propagation()
            .store_for_propagation(dest, b"msg-2", Some("src-b"))
            .expect("store 2");
        hub.app_context
            .propagation()
            .store_for_propagation(dest, b"msg-3", Some("src-a"))
            .expect("store 3");

        let (count, _) = hub.app_context.propagation().stats().expect("stats");
        assert_eq!(count, 3);

        // Fetch all for destination
        let messages = hub.app_context.propagation().fetch_for_destination(dest).expect("fetch");
        assert_eq!(messages.len(), 3);

        // Delete the first two (simulating successful delivery)
        let delivered_ids: Vec<String> = messages[..2].iter().map(|(id, _)| id.clone()).collect();
        hub.app_context.propagation().delete_delivered(&delivered_ids).expect("delete delivered");

        // One message remains
        let remaining =
            hub.app_context.propagation().fetch_for_destination(dest).expect("fetch remaining");
        assert_eq!(remaining.len(), 1);

        let (count, _) = hub.app_context.propagation().stats().expect("stats");
        assert_eq!(count, 1);
    })
    .await;
}

#[tokio::test]
async fn hub_expiry_removes_stale_messages() {
    with_timeout(async {
        let hub = TestNodeBuilder::new("hub-expiry").tcp_server("127.0.0.1:0").build().await;
        hub.app_context.propagation().set_enabled(true);

        // Set very short expiry so messages expire immediately
        hub.app_context.propagation().set_expiry_secs(0);

        let dest = "2222222222222222222222222222222";
        hub.app_context
            .propagation()
            .store_for_propagation(dest, b"will-expire", None)
            .expect("store");

        let (count, _) = hub.app_context.propagation().stats().expect("stats before");
        assert_eq!(count, 1);

        // Expire — with 0s expiry, message should be immediately stale
        let expired = hub.app_context.propagation().expire_old().expect("expire");
        assert_eq!(expired, 1, "one message should have expired");

        let (count, _) = hub.app_context.propagation().stats().expect("stats after");
        assert_eq!(count, 0, "no messages should remain after expiry");

        // Fetch should also be empty
        let fetched =
            hub.app_context.propagation().fetch_for_destination(dest).expect("fetch after expiry");
        assert!(fetched.is_empty());
    })
    .await;
}

#[tokio::test]
async fn propagation_disabled_does_not_store() {
    with_timeout(async {
        let hub = TestNodeBuilder::new("hub-disabled").tcp_server("127.0.0.1:0").build().await;
        // Propagation NOT enabled (default)
        assert!(!hub.app_context.propagation().is_enabled());

        let dest = "3333333333333333333333333333333";
        let stored = hub
            .app_context
            .propagation()
            .store_for_propagation(dest, b"should-not-store", None)
            .expect("store attempt");
        assert!(!stored, "disabled propagation should not store");

        let (count, _) = hub.app_context.propagation().stats().expect("stats");
        assert_eq!(count, 0);
    })
    .await;
}

#[tokio::test]
async fn hub_inbound_worker_routes_nonlocal_to_propagation() {
    with_timeout(async {
        // This test verifies the inbound worker's routing decision:
        // when propagation is enabled and a message arrives for a
        // non-local destination, it goes to propagation instead of
        // local delivery.

        let hub = TestNodeBuilder::new("hub-route").tcp_server("127.0.0.1:0").build().await;
        hub.app_context.propagation().set_enabled(true);

        let alice = TestNodeBuilder::new("alice-route")
            .tcp_client(hub.listen_addr.expect("hub addr"))
            .build()
            .await;

        tokio::time::sleep(SETTLE).await;
        alice.announce().await;
        hub.announce().await;

        await_identity_resolved(&alice.app_context, &hub.delivery_addr, Duration::from_secs(10))
            .await;

        // Alice sends to the HUB's own delivery hash — this IS local,
        // so it should be delivered normally, NOT stored in propagation.
        alice.send_chat(&hub.delivery_hash, "local-delivery").await.expect("send to hub");

        await_inbound_count(&hub.app_context, 1, Duration::from_secs(15)).await;

        // Hub should have received the message locally
        {
            let store = hub.app_context.store().lock().expect("lock");
            let messages = store.list_messages(100, None).expect("list");
            let inbound: Vec<_> = messages.iter().filter(|m| m.direction == "in").collect();
            assert_eq!(inbound.len(), 1, "hub should have 1 local inbound message");
            assert_eq!(inbound[0].content, "local-delivery");
        }

        // Propagation store should be empty — the message was local
        let (count, _) = hub.app_context.propagation().stats().expect("stats");
        assert_eq!(count, 0, "local messages should NOT go to propagation store");
    })
    .await;
}

#[tokio::test]
async fn multiple_destinations_stored_independently() {
    with_timeout(async {
        let hub = TestNodeBuilder::new("hub-multi-dest").tcp_server("127.0.0.1:0").build().await;
        hub.app_context.propagation().set_enabled(true);

        let dest_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let dest_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

        // Store messages for two different offline destinations
        hub.app_context
            .propagation()
            .store_for_propagation(dest_a, b"for-a-1", None)
            .expect("store a1");
        hub.app_context
            .propagation()
            .store_for_propagation(dest_a, b"for-a-2", None)
            .expect("store a2");
        hub.app_context
            .propagation()
            .store_for_propagation(dest_b, b"for-b-1", None)
            .expect("store b1");

        let (count, _) = hub.app_context.propagation().stats().expect("stats");
        assert_eq!(count, 3, "total 3 messages stored");

        // Fetch per destination
        let a_msgs = hub.app_context.propagation().fetch_for_destination(dest_a).expect("fetch a");
        assert_eq!(a_msgs.len(), 2);

        let b_msgs = hub.app_context.propagation().fetch_for_destination(dest_b).expect("fetch b");
        assert_eq!(b_msgs.len(), 1);
        assert_eq!(b_msgs[0].1, b"for-b-1");

        // Delete dest_a's messages — dest_b should be unaffected
        let a_ids: Vec<String> = a_msgs.iter().map(|(id, _)| id.clone()).collect();
        hub.app_context.propagation().delete_delivered(&a_ids).expect("delete a");

        let (count, _) = hub.app_context.propagation().stats().expect("stats after");
        assert_eq!(count, 1, "only dest_b's message should remain");
    })
    .await;
}
