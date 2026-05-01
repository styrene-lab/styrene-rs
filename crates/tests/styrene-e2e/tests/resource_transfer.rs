//! Resource transfer debugging.
//!
//! Diagnoses why large messages (> LINK_PACKET_MDU) don't complete
//! between two TestNodes. Monitors resource events at the transport layer.

use std::time::Duration;
use styrene_e2e::helpers::{with_timeout, await_identity_resolved, SETTLE};
use styrene_e2e::node::TestNodeBuilder;
use rns_core::transport::resource::ResourceEventKind;

#[tokio::test]
async fn resource_events_fire_on_large_payload() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice-res")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let bob = TestNodeBuilder::new("bob-res")
            .tcp_client(alice.listen_addr.expect("addr"))
            .build()
            .await;

        tokio::time::sleep(SETTLE).await;
        alice.announce().await;
        bob.announce().await;

        await_identity_resolved(
            &alice.app_context,
            &bob.delivery_addr,
            Duration::from_secs(10),
        )
        .await;

        // Subscribe to resource events on BOTH sides
        let mut alice_resource_rx = alice.app_context.transport().subscribe_resources();
        let mut bob_resource_rx = bob.app_context.transport().subscribe_resources();

        // Send a message that exceeds single-packet size
        // LXMF overhead: dest(16) + source(16) + sig(64) + msgpack = ~120 bytes
        // LINK_PACKET_MDU ≈ 350 bytes → useful content ≈ 230 bytes
        // A 500-byte content will definitely trigger resource transfer
        let large = "X".repeat(500);
        eprintln!("[test] sending 500-byte content...");

        let send_result = alice
            .send_chat(&bob.delivery_hash, &large)
            .await;

        match &send_result {
            Ok(id) => eprintln!("[test] send_chat returned Ok({})", id),
            Err(e) => eprintln!("[test] send_chat returned Err({})", e),
        }

        // Wait and collect resource events
        eprintln!("[test] waiting for resource events...");
        tokio::time::sleep(Duration::from_secs(5)).await;

        // Check alice's resource events (sender side — should see OutboundComplete)
        let mut alice_events = Vec::new();
        while let Ok(event) = alice_resource_rx.try_recv() {
            let kind = match &event.kind {
                ResourceEventKind::Progress(p) => {
                    format!("Progress({}/{})", p.received_parts, p.total_parts)
                }
                ResourceEventKind::Complete(c) => {
                    format!("Complete(data_len={})", c.data.len())
                }
                ResourceEventKind::OutboundComplete => "OutboundComplete".to_string(),
            };
            eprintln!("[test] alice resource event: {} link={}", kind, event.link_id);
            alice_events.push(event);
        }

        // Check bob's resource events (receiver side — should see Progress → Complete)
        let mut bob_events = Vec::new();
        while let Ok(event) = bob_resource_rx.try_recv() {
            let kind = match &event.kind {
                ResourceEventKind::Progress(p) => {
                    format!("Progress({}/{})", p.received_parts, p.total_parts)
                }
                ResourceEventKind::Complete(c) => {
                    format!("Complete(data_len={})", c.data.len())
                }
                ResourceEventKind::OutboundComplete => "OutboundComplete".to_string(),
            };
            eprintln!("[test] bob resource event: {} link={}", kind, event.link_id);
            bob_events.push(event);
        }

        eprintln!(
            "[test] alice resource events: {}, bob resource events: {}",
            alice_events.len(),
            bob_events.len()
        );

        // Check if bob got any messages in the store
        {
            let store = bob.app_context.store().lock().expect("lock");
            let msgs = store.list_messages(100, None).expect("list");
            let inbound: Vec<_> = msgs.iter().filter(|m| m.direction == "in").collect();
            eprintln!("[test] bob inbound messages: {}", inbound.len());
            for msg in &inbound {
                eprintln!("[test]   content_len={} src={}", msg.content.len(), msg.source);
            }
        }

        // Report findings
        if bob_events.is_empty() {
            eprintln!(
                "[test] FINDING: no resource events on receiver side. \
                 The resource advertisement may not be reaching the receiver, \
                 or the resource protocol negotiation is not completing."
            );
        }

        let bob_complete = bob_events.iter().any(|e| matches!(e.kind, ResourceEventKind::Complete(_)));
        if bob_complete {
            eprintln!("[test] resource transfer COMPLETED on receiver side");
            // Verify the message was delivered
            let store = bob.app_context.store().lock().expect("lock");
            let msgs = store.list_messages(100, None).expect("list");
            let inbound: Vec<_> = msgs.iter().filter(|m| m.direction == "in").collect();
            assert!(
                !inbound.is_empty(),
                "resource completed but no message in store — inbound worker not processing resource events"
            );
        }

        // This test is diagnostic — it passes either way but logs what's happening
    })
    .await;
}
