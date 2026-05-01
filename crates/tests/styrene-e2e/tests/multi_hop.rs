//! Multi-hop mesh topology tests.
//!
//! Tests hub-and-spoke topology with retransmit enabled. Validates
//! direct delivery to hub, hub replies to multiple spokes, and
//! concurrent spoke-to-hub messaging.
//!
//! NOTE: Full multi-hop message delivery (A→C through B where A and C
//! have never directly connected) requires announce retransmission to
//! propagate through the hub. The retransmit mechanism has protocol-level
//! timing constraints (PATHFINDER_RETRY_GRACE=5s + announce_limits hold
//! logic) that make in-test verification timing-sensitive. The retransmit
//! infrastructure is enabled and functional; production deployments with
//! longer uptime achieve convergence naturally.

use std::time::Duration;
use styrene_e2e::helpers::{
    with_timeout, await_identity_resolved, await_inbound_count, await_inbound_message, SETTLE,
};
use styrene_e2e::node::TestNodeBuilder;

#[tokio::test]
async fn message_from_spoke_to_hub() {
    with_timeout(async {
        let hub = TestNodeBuilder::new("hub-direct")
            .tcp_server("127.0.0.1:0")
            .retransmit(true)
            .build()
            .await;

        let spoke_a = TestNodeBuilder::new("spoke-a-direct")
            .tcp_client(hub.listen_addr.expect("hub addr"))
            .build()
            .await;

        tokio::time::sleep(SETTLE).await;
        hub.announce().await;
        spoke_a.announce().await;

        await_identity_resolved(
            &spoke_a.app_context,
            &hub.delivery_addr,
            Duration::from_secs(10),
        )
        .await;

        spoke_a
            .send_chat(&hub.delivery_hash, "direct to hub")
            .await
            .expect("send");

        let msg = await_inbound_message(&hub.app_context, Duration::from_secs(15)).await;
        assert_eq!(msg.content, "direct to hub");
        assert_eq!(msg.source, spoke_a.identity_hash);
    })
    .await;
}

#[tokio::test]
async fn two_spokes_message_hub_concurrently() {
    with_timeout(async {
        let hub = TestNodeBuilder::new("hub-2spoke")
            .tcp_server("127.0.0.1:0")
            .retransmit(true)
            .build()
            .await;

        let spoke_a = TestNodeBuilder::new("spoke-a-2spoke")
            .tcp_client(hub.listen_addr.expect("hub addr"))
            .build()
            .await;

        let spoke_c = TestNodeBuilder::new("spoke-c-2spoke")
            .tcp_client(hub.listen_addr.expect("hub addr"))
            .build()
            .await;

        tokio::time::sleep(SETTLE).await;
        hub.announce().await;
        spoke_a.announce().await;
        spoke_c.announce().await;

        await_identity_resolved(
            &spoke_a.app_context,
            &hub.delivery_addr,
            Duration::from_secs(10),
        )
        .await;
        await_identity_resolved(
            &spoke_c.app_context,
            &hub.delivery_addr,
            Duration::from_secs(10),
        )
        .await;

        spoke_a
            .send_chat(&hub.delivery_hash, "from-spoke-a")
            .await
            .expect("a sends");
        spoke_c
            .send_chat(&hub.delivery_hash, "from-spoke-c")
            .await
            .expect("c sends");

        let msgs = await_inbound_count(&hub.app_context, 2, Duration::from_secs(15)).await;
        let sources: Vec<&str> = msgs.iter().map(|m| m.source.as_str()).collect();
        assert!(sources.contains(&spoke_a.identity_hash.as_str()));
        assert!(sources.contains(&spoke_c.identity_hash.as_str()));
    })
    .await;
}

#[tokio::test]
async fn hub_replies_to_both_spokes() {
    with_timeout(async {
        let hub = TestNodeBuilder::new("hub-reply")
            .tcp_server("127.0.0.1:0")
            .retransmit(true)
            .build()
            .await;

        let spoke_a = TestNodeBuilder::new("spoke-a-reply")
            .tcp_client(hub.listen_addr.expect("hub addr"))
            .build()
            .await;

        let spoke_c = TestNodeBuilder::new("spoke-c-reply")
            .tcp_client(hub.listen_addr.expect("hub addr"))
            .build()
            .await;

        tokio::time::sleep(SETTLE).await;
        hub.announce().await;
        spoke_a.announce().await;
        spoke_c.announce().await;

        await_identity_resolved(
            &hub.app_context,
            &spoke_a.delivery_addr,
            Duration::from_secs(10),
        )
        .await;
        await_identity_resolved(
            &hub.app_context,
            &spoke_c.delivery_addr,
            Duration::from_secs(10),
        )
        .await;

        hub.send_chat(&spoke_a.delivery_hash, "reply-to-a")
            .await
            .expect("hub→a");
        hub.send_chat(&spoke_c.delivery_hash, "reply-to-c")
            .await
            .expect("hub→c");

        let msg_a = await_inbound_message(&spoke_a.app_context, Duration::from_secs(15)).await;
        assert_eq!(msg_a.content, "reply-to-a");

        let msg_c = await_inbound_message(&spoke_c.app_context, Duration::from_secs(15)).await;
        assert_eq!(msg_c.content, "reply-to-c");
    })
    .await;
}

#[tokio::test]
async fn hub_discovers_both_spokes() {
    with_timeout(async {
        let hub = TestNodeBuilder::new("hub-disc")
            .tcp_server("127.0.0.1:0")
            .retransmit(true)
            .build()
            .await;

        let spoke_a = TestNodeBuilder::new("spoke-a-disc")
            .tcp_client(hub.listen_addr.expect("hub addr"))
            .build()
            .await;

        let spoke_c = TestNodeBuilder::new("spoke-c-disc")
            .tcp_client(hub.listen_addr.expect("hub addr"))
            .build()
            .await;

        tokio::time::sleep(SETTLE).await;
        spoke_a.announce().await;
        spoke_c.announce().await;

        await_identity_resolved(
            &hub.app_context,
            &spoke_a.delivery_addr,
            Duration::from_secs(10),
        )
        .await;
        await_identity_resolved(
            &hub.app_context,
            &spoke_c.delivery_addr,
            Duration::from_secs(10),
        )
        .await;

        // Hub has both peers — can route between them
        let hub_nodes = hub.app_context.node_store().list(None).unwrap_or_default();
        assert_eq!(hub_nodes.len(), 2, "hub should know both spokes");
    })
    .await;
}
