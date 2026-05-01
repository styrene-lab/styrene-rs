//! Auto-reply integration e2e.
//!
//! Tests that when Bob has auto-reply enabled and Alice sends him a message,
//! Alice receives the automatic reply. Full pipeline:
//! inbound message → auto-reply check → outbound delivery.

use std::time::Duration;

use styrene_e2e::helpers::{
    with_timeout, await_identity_resolved, await_inbound_count, two_connected_nodes, SETTLE,
};
use styrene_e2e::node::TestNodeBuilder;

#[tokio::test]
async fn auto_reply_sends_response_to_sender() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-ar", "bob-ar").await;

        // Enable auto-reply on Bob with 0s cooldown for testing
        bob.app_context.auto_reply().set_config(
            styrened::services::auto_reply::AutoReplyConfig {
                mode: styrened::services::AutoReplyMode::All,
                message: "I'm currently away from the mesh.".to_string(),
                cooldown: Duration::from_secs(0),
            },
        );

        // Alice sends to Bob
        alice
            .send_chat(&bob.delivery_hash, "hey bob, you there?")
            .await
            .expect("send");

        // Bob should receive Alice's message
        await_inbound_count(&bob.app_context, 1, Duration::from_secs(15)).await;

        // Alice should receive the auto-reply from Bob
        let auto_replies = await_inbound_count(
            &alice.app_context,
            1,
            Duration::from_secs(15),
        )
        .await;

        assert_eq!(
            auto_replies[0].content,
            "I'm currently away from the mesh.",
            "auto-reply content should match configured message"
        );
        assert_eq!(
            auto_replies[0].source, bob.identity_hash,
            "auto-reply should come from bob"
        );
    })
    .await;
}

#[tokio::test]
async fn auto_reply_respects_cooldown() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-cd", "bob-cd").await;

        // Enable auto-reply with a long cooldown
        bob.app_context.auto_reply().set_config(
            styrened::services::auto_reply::AutoReplyConfig {
                mode: styrened::services::AutoReplyMode::All,
                message: "Away".to_string(),
                cooldown: Duration::from_secs(3600), // 1 hour
            },
        );

        // Alice sends first message — should trigger auto-reply
        alice
            .send_chat(&bob.delivery_hash, "first")
            .await
            .expect("send first");
        await_inbound_count(&bob.app_context, 1, Duration::from_secs(15)).await;
        await_inbound_count(&alice.app_context, 1, Duration::from_secs(15)).await;

        // Alice sends second message — should NOT trigger auto-reply (cooldown)
        alice
            .send_chat(&bob.delivery_hash, "second")
            .await
            .expect("send second");
        await_inbound_count(&bob.app_context, 2, Duration::from_secs(15)).await;

        // Wait a bit for any potential auto-reply
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Alice should still have exactly 1 inbound (the first auto-reply only)
        let store = alice.app_context.store().lock().expect("lock");
        let messages = store.list_messages(100, None).expect("list");
        let inbound: Vec<_> = messages.iter().filter(|m| m.direction == "in").collect();
        assert_eq!(
            inbound.len(),
            1,
            "second message should NOT trigger auto-reply due to cooldown"
        );
    })
    .await;
}

#[tokio::test]
async fn auto_reply_disabled_sends_nothing() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-off", "bob-off").await;

        // Auto-reply disabled (default)
        assert!(!bob.app_context.auto_reply().is_enabled());

        alice
            .send_chat(&bob.delivery_hash, "hello")
            .await
            .expect("send");
        await_inbound_count(&bob.app_context, 1, Duration::from_secs(15)).await;

        // Wait and verify alice gets no auto-reply
        tokio::time::sleep(Duration::from_secs(2)).await;

        let store = alice.app_context.store().lock().expect("lock");
        let messages = store.list_messages(100, None).expect("list");
        let inbound: Vec<_> = messages.iter().filter(|m| m.direction == "in").collect();
        assert!(
            inbound.is_empty(),
            "disabled auto-reply should send nothing"
        );
    })
    .await;
}

#[tokio::test]
async fn auto_reply_does_not_loop_between_two_nodes() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-loop", "bob-loop").await;

        // BOTH nodes have auto-reply enabled — potential infinite loop
        let ar_config = styrened::services::auto_reply::AutoReplyConfig {
            mode: styrened::services::AutoReplyMode::All,
            message: "I'm away".to_string(),
            cooldown: Duration::from_secs(0), // no cooldown — maximally dangerous
        };
        alice.app_context.auto_reply().set_config(ar_config.clone());
        bob.app_context.auto_reply().set_config(ar_config);

        // Alice sends the triggering message
        alice
            .send_chat(&bob.delivery_hash, "trigger")
            .await
            .expect("send trigger");

        // Bob receives trigger → auto-replies → Alice receives auto-reply
        await_inbound_count(&bob.app_context, 1, Duration::from_secs(15)).await;
        await_inbound_count(&alice.app_context, 1, Duration::from_secs(15)).await;

        // Now the question: does Alice's auto-reply to Bob's auto-reply
        // cause another auto-reply from Bob, ad infinitum?
        // Wait 3 seconds and check message counts are bounded.
        tokio::time::sleep(Duration::from_secs(3)).await;

        let alice_inbound = {
            let store = alice.app_context.store().lock().expect("lock");
            store
                .list_messages(100, None)
                .expect("list")
                .into_iter()
                .filter(|m| m.direction == "in")
                .count()
        };

        let bob_inbound = {
            let store = bob.app_context.store().lock().expect("lock");
            store
                .list_messages(100, None)
                .expect("list")
                .into_iter()
                .filter(|m| m.direction == "in")
                .count()
        };

        // With loop prevention: alice should have 1 inbound (bob's auto-reply),
        // bob should have 1 inbound (alice's trigger). No further messages.
        // Without loop prevention: counts would grow unboundedly.
        assert!(
            alice_inbound <= 3,
            "auto-reply should not loop — alice has {} inbound messages",
            alice_inbound
        );
        assert!(
            bob_inbound <= 3,
            "auto-reply should not loop — bob has {} inbound messages",
            bob_inbound
        );

        // Ideal: exactly 1 inbound each (no loop at all)
        // Acceptable: 2-3 if one round of mutual reply happens before cooldown kicks in
        // Unacceptable: >5 (runaway loop)
    })
    .await;
}

#[tokio::test]
async fn auto_reply_independent_cooldown_per_peer() {
    with_timeout(async {
        // Bob has auto-reply enabled. Alice and Charlie both message him.
        // Both should get replies (independent cooldowns).
        let bob = TestNodeBuilder::new("bob-multi")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let alice = TestNodeBuilder::new("alice-multi")
            .tcp_client(bob.listen_addr.expect("addr"))
            .build()
            .await;

        let charlie = TestNodeBuilder::new("charlie-multi")
            .tcp_client(bob.listen_addr.expect("addr"))
            .build()
            .await;

        tokio::time::sleep(SETTLE).await;
        alice.announce().await;
        bob.announce().await;
        charlie.announce().await;

        await_identity_resolved(
            &bob.app_context,
            &alice.delivery_addr,
            Duration::from_secs(10),
        )
        .await;
        await_identity_resolved(
            &bob.app_context,
            &charlie.delivery_addr,
            Duration::from_secs(10),
        )
        .await;
        await_identity_resolved(
            &alice.app_context,
            &bob.delivery_addr,
            Duration::from_secs(10),
        )
        .await;
        await_identity_resolved(
            &charlie.app_context,
            &bob.delivery_addr,
            Duration::from_secs(10),
        )
        .await;

        bob.app_context.auto_reply().set_config(
            styrened::services::auto_reply::AutoReplyConfig {
                mode: styrened::services::AutoReplyMode::All,
                message: "Away from keyboard".to_string(),
                cooldown: Duration::from_secs(3600),
            },
        );

        // Alice and Charlie both message Bob
        alice
            .send_chat(&bob.delivery_hash, "hi from alice")
            .await
            .expect("alice sends");
        charlie
            .send_chat(&bob.delivery_hash, "hi from charlie")
            .await
            .expect("charlie sends");

        // Bob should receive both
        await_inbound_count(&bob.app_context, 2, Duration::from_secs(15)).await;

        // Both should receive auto-replies
        await_inbound_count(&alice.app_context, 1, Duration::from_secs(15)).await;
        await_inbound_count(&charlie.app_context, 1, Duration::from_secs(15)).await;

        // Verify content
        {
            let store = alice.app_context.store().lock().expect("lock");
            let msgs = store.list_messages(100, None).expect("list");
            let inbound: Vec<_> = msgs.iter().filter(|m| m.direction == "in").collect();
            assert_eq!(inbound.len(), 1);
            assert_eq!(inbound[0].content, "Away from keyboard");
        }
        {
            let store = charlie.app_context.store().lock().expect("lock");
            let msgs = store.list_messages(100, None).expect("list");
            let inbound: Vec<_> = msgs.iter().filter(|m| m.direction == "in").collect();
            assert_eq!(inbound.len(), 1);
            assert_eq!(inbound[0].content, "Away from keyboard");
        }
    })
    .await;
}
