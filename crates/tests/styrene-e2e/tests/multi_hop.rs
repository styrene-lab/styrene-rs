//! Multi-hop mesh topology tests.
//!
//! Tests hub-and-spoke and routed A-to-B-to-C topologies.

use rns_core::hash::AddressHash;
use rns_core::transport::core_transport::path_table::{
    PathSnapshot, RouteEvent, RouteEventKind, RouteLossReason,
};
use rns_core::transport::iface::InterfaceState;
use std::time::Duration;
use styrene_e2e::helpers::{
    await_identity_resolved, await_inbound_count, await_inbound_message, with_timeout, SETTLE,
};
use styrene_e2e::node::TestNodeBuilder;

const MILESTONE_TIMEOUT: Duration = Duration::from_secs(15);

async fn await_connected_interface(node: &styrene_e2e::node::TestNode, interface: AddressHash) {
    let deadline = tokio::time::Instant::now() + MILESTONE_TIMEOUT;
    loop {
        if node.transport.interface_snapshots().await.iter().any(|snapshot| {
            snapshot.hash == interface && snapshot.state == InterfaceState::Connected
        }) {
            return;
        }
        assert!(tokio::time::Instant::now() < deadline, "interface {interface} did not connect");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn await_exact_route(
    node: &styrene_e2e::node::TestNode,
    destination: AddressHash,
    hops: u8,
    next_hop: AddressHash,
    interface: Option<AddressHash>,
) -> PathSnapshot {
    let deadline = tokio::time::Instant::now() + MILESTONE_TIMEOUT;
    loop {
        if let Some(route) = node.transport.path_snapshot(&destination).await {
            if route.hops == hops
                && route.received_from == next_hop
                && interface.is_none_or(|expected| route.iface == expected)
            {
                return route;
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "route to {destination} did not reach {hops} hops through {next_hop}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn await_route_absent(node: &styrene_e2e::node::TestNode, destination: AddressHash) {
    let deadline = tokio::time::Instant::now() + MILESTONE_TIMEOUT;
    loop {
        if node.transport.path_snapshot(&destination).await.is_none() {
            return;
        }
        assert!(tokio::time::Instant::now() < deadline, "stale route to {destination} remained");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn await_route_event(
    events: &mut tokio::sync::broadcast::Receiver<RouteEvent>,
    destination: AddressHash,
    kind: RouteEventKind,
) -> RouteEvent {
    tokio::time::timeout(MILESTONE_TIMEOUT, async {
        loop {
            match events.recv().await {
                Ok(event) if event.route.destination == destination && event.kind == kind => {
                    return event;
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    panic!("route event stream closed")
                }
            }
        }
    })
    .await
    .expect("route event milestone timed out")
}

async fn await_packet_delivery_receipt(node: &styrene_e2e::node::TestNode, message_id: &str) {
    let deadline = tokio::time::Instant::now() + MILESTONE_TIMEOUT;
    loop {
        let status = node
            .app_context
            .store()
            .lock()
            .expect("message store")
            .get_message(message_id)
            .expect("message lookup")
            .and_then(|message| message.receipt_status);
        if status.as_deref() == Some("delivered: packet-receipt") {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "message {message_id} did not receive its packet delivery receipt; status={status:?}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn await_failed_receipt(node: &styrene_e2e::node::TestNode, message_id: &str) {
    let deadline = tokio::time::Instant::now() + MILESTONE_TIMEOUT;
    loop {
        let status = node
            .app_context
            .store()
            .lock()
            .expect("message store")
            .get_message(message_id)
            .expect("message lookup")
            .and_then(|message| message.receipt_status);
        if status.as_deref().is_some_and(|value| value.starts_with("failed:")) {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "message {message_id} did not reach failed state; status={status:?}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn await_inbound_content(
    node: &styrene_e2e::node::TestNode,
    content: &str,
) -> styrened::storage::messages::MessageRecord {
    let deadline = tokio::time::Instant::now() + MILESTONE_TIMEOUT;
    loop {
        let message = node
            .app_context
            .store()
            .lock()
            .expect("message store")
            .list_messages(100, None)
            .expect("message list")
            .into_iter()
            .find(|message| message.direction == "in" && message.content == content);
        if let Some(message) = message {
            return message;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "inbound message with content {content:?} did not arrive"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn assert_no_inbound_content(node: &styrene_e2e::node::TestNode, content: &str) {
    let messages = node
        .app_context
        .store()
        .lock()
        .expect("message store")
        .list_messages(100, None)
        .expect("message list");
    assert!(
        messages.iter().all(|message| message.direction != "in" || message.content != content),
        "cancelled-route message reached C"
    );
}

async fn await_intermediate_links(node: &styrene_e2e::node::TestNode, expected: usize) {
    let deadline = tokio::time::Instant::now() + MILESTONE_TIMEOUT;
    loop {
        let actual = node.transport.intermediate_link_count_for_test().await;
        if actual == expected {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "expected {expected} intermediate links, found {actual}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn assert_active_link_route(
    node: &styrene_e2e::node::TestNode,
    destination: AddressHash,
    interface: AddressHash,
) {
    let snapshot = node.transport.link_lifecycle_snapshot().await;
    assert!(
        snapshot
            .active
            .iter()
            .any(|link| { link.address_hash == destination && link.interface == Some(interface) }),
        "proved link to {destination} must use interface {interface}; active={:?}",
        snapshot.active
    );
}

async fn await_announce(
    events: &mut tokio::sync::broadcast::Receiver<
        rns_core::transport::core_transport::AnnounceEvent,
    >,
    destination: AddressHash,
) {
    tokio::time::timeout(MILESTONE_TIMEOUT, async {
        loop {
            match events.recv().await {
                Ok(event) if event.destination.lock().await.desc.address_hash == destination => {
                    return;
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    panic!("announce event stream closed")
                }
            }
        }
    })
    .await
    .expect("announce milestone timed out");
}

#[tokio::test]
async fn routed_delivery_recovers_through_replacement_interface() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let b = TestNodeBuilder::new("route-b")
            .tcp_server("127.0.0.1:0")
            .retransmit(true)
            .build()
            .await;
        let b_addr = b.listen_addr.expect("B listener");
        let a = TestNodeBuilder::new("route-a").tcp_client(b_addr).build().await;
        let c = TestNodeBuilder::new("route-c").tcp_client(b_addr).build().await;

        let a_iface = a.transport.interface_snapshots().await[0].hash;
        let c_iface = c.transport.interface_snapshots().await[0].hash;
        await_connected_interface(&a, a_iface).await;
        await_connected_interface(&c, c_iface).await;

        let mut a_route_events = a.transport.route_events().await;
        a.announce().await;
        c.announce().await;
        let initial_b_route =
            await_exact_route(&b, a.delivery_addr, 1, a.delivery_addr, None).await;
        await_exact_route(&b, c.delivery_addr, 1, c.delivery_addr, None).await;

        a.app_context.transport().request_path(&c.delivery_addr).await;
        c.app_context.transport().request_path(&a.delivery_addr).await;
        let b_identity = *b.identity.address_hash();
        let initial_a_route =
            await_exact_route(&a, c.delivery_addr, 2, b_identity, Some(a_iface)).await;
        await_exact_route(&c, a.delivery_addr, 2, b_identity, Some(c_iface)).await;

        let first_id = a.send_chat(&c.delivery_hash, "before route loss").await.expect("A to C");
        let first = await_inbound_content(&c, "before route loss").await;
        assert_eq!(first.content, "before route loss");
        assert_eq!(first.source, a.identity_hash);
        await_packet_delivery_receipt(&a, &first_id).await;
        assert_active_link_route(&a, c.delivery_addr, initial_a_route.iface).await;
        await_intermediate_links(&b, 1).await;

        a.cancel_interface(&initial_a_route.iface).await;
        let stale_route = a
            .transport
            .path_snapshot(&c.delivery_addr)
            .await
            .expect("route remains until scheduled cull");
        assert_eq!(stale_route.iface, initial_a_route.iface);
        assert!(
            !a.transport
                .iface_manager()
                .lock()
                .await
                .active_interface_hashes()
                .contains(&stale_route.iface),
            "cancelled route interface must not be current"
        );
        let stale_id = a
            .send_chat(&c.delivery_hash, "must not cross stale route")
            .await
            .expect("normal routed send records its terminal outcome");
        await_failed_receipt(&a, &stale_id).await;

        let lost =
            await_route_event(&mut a_route_events, c.delivery_addr, RouteEventKind::Lost).await;
        assert_eq!(lost.loss_reason, Some(RouteLossReason::InterfaceUnavailable));
        assert_eq!(lost.route.iface, initial_a_route.iface);
        await_route_absent(&a, c.delivery_addr).await;
        assert_no_inbound_content(&c, "must not cross stale route");
        await_intermediate_links(&b, 0).await;
        assert!(
            a.transport.link_lifecycle_snapshot().await.active.is_empty(),
            "local link bound to the lost interface must be removed"
        );

        let replacement_iface = a.attach_tcp_client(b_addr).await;
        assert_ne!(replacement_iface, initial_a_route.iface);
        await_connected_interface(&a, replacement_iface).await;
        a.announce().await;
        let replacement_b_route =
            await_exact_route(&b, a.delivery_addr, 1, a.delivery_addr, None).await;
        assert_ne!(replacement_b_route.iface, initial_b_route.iface);

        let mut b_announces = b.transport.recv_announces().await;
        c.announce().await;
        await_announce(&mut b_announces, c.delivery_addr).await;
        a.app_context.transport().request_path(&c.delivery_addr).await;
        let rediscovered =
            await_route_event(&mut a_route_events, c.delivery_addr, RouteEventKind::Rediscovered)
                .await;
        assert_eq!(rediscovered.route.hops, 2);
        assert_eq!(rediscovered.route.received_from, b_identity);
        assert_eq!(rediscovered.route.iface, replacement_iface);
        await_exact_route(&a, c.delivery_addr, 2, b_identity, Some(replacement_iface)).await;

        let second_id = a.send_chat(&c.delivery_hash, "after rediscovery").await.expect("A to C");
        let second = await_inbound_content(&c, "after rediscovery").await;
        assert_eq!(second.content, "after rediscovery");
        assert_eq!(second.source, a.identity_hash);
        await_packet_delivery_receipt(&a, &second_id).await;
        assert_active_link_route(&a, c.delivery_addr, replacement_iface).await;

        c.shutdown().await;
        a.shutdown().await;
        b.shutdown().await;
    })
    .await
    .expect("routed recovery scenario timed out after 60s");
}

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

        await_identity_resolved(&spoke_a.app_context, &hub.delivery_addr, Duration::from_secs(10))
            .await;

        spoke_a.send_chat(&hub.delivery_hash, "direct to hub").await.expect("send");

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

        await_identity_resolved(&spoke_a.app_context, &hub.delivery_addr, Duration::from_secs(10))
            .await;
        await_identity_resolved(&spoke_c.app_context, &hub.delivery_addr, Duration::from_secs(10))
            .await;

        spoke_a.send_chat(&hub.delivery_hash, "from-spoke-a").await.expect("a sends");
        spoke_c.send_chat(&hub.delivery_hash, "from-spoke-c").await.expect("c sends");

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

        await_identity_resolved(&hub.app_context, &spoke_a.delivery_addr, Duration::from_secs(10))
            .await;
        await_identity_resolved(&hub.app_context, &spoke_c.delivery_addr, Duration::from_secs(10))
            .await;

        hub.send_chat(&spoke_a.delivery_hash, "reply-to-a").await.expect("hub→a");
        hub.send_chat(&spoke_c.delivery_hash, "reply-to-c").await.expect("hub→c");

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

        await_identity_resolved(&hub.app_context, &spoke_a.delivery_addr, Duration::from_secs(10))
            .await;
        await_identity_resolved(&hub.app_context, &spoke_c.delivery_addr, Duration::from_secs(10))
            .await;

        // Hub has both peers — can route between them
        let hub_nodes = hub.app_context.node_store().list(None).unwrap_or_default();
        assert_eq!(hub_nodes.len(), 2, "hub should know both spokes");
    })
    .await;
}
