//! Assertive resource transfer completion over two live RNS transports.

use rns_core::transport::resource::ResourceEventKind;
use std::time::Duration;
use styrene_e2e::helpers::{await_identity_resolved, with_timeout, SETTLE};
use styrene_e2e::node::TestNodeBuilder;

#[tokio::test]
async fn large_payload_completes_with_integrity_progress_and_cleanup() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice-res").tcp_server("127.0.0.1:0").build().await;
        let bob = TestNodeBuilder::new("bob-res")
            .tcp_client(alice.listen_addr.expect("addr"))
            .build()
            .await;

        tokio::time::sleep(SETTLE).await;
        alice.announce().await;
        bob.announce().await;
        await_identity_resolved(&alice.app_context, &bob.delivery_addr, Duration::from_secs(10))
            .await;

        let mut sender_events = alice.app_context.transport().subscribe_resources();
        let mut receiver_events = bob.app_context.transport().subscribe_resources();
        let content =
            (0..4096).map(|index| char::from(b'A' + (index % 26) as u8)).collect::<String>();
        let message_id =
            alice.send_chat(&bob.delivery_hash, &content).await.expect("resource-backed send");

        let sender_hash = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let event = sender_events.recv().await.expect("sender resource event channel");
                match event.kind {
                    ResourceEventKind::OutboundComplete => break event.hash,
                    ResourceEventKind::Failed(reason) => {
                        panic!("sender resource failed: {reason:?}")
                    }
                    ResourceEventKind::Progress(_) | ResourceEventKind::Complete(_) => {}
                }
            }
        })
        .await
        .expect("sender resource completion deadline");

        let (receiver_hash, progress) = tokio::time::timeout(Duration::from_secs(10), async {
            let mut progress = Vec::new();
            loop {
                let event = receiver_events.recv().await.expect("receiver resource event channel");
                match event.kind {
                    ResourceEventKind::Progress(value) => progress.push(value),
                    ResourceEventKind::Complete(complete) => {
                        assert!(!complete.data.is_empty(), "completed resource payload is empty");
                        break (event.hash, progress);
                    }
                    ResourceEventKind::Failed(reason) => {
                        panic!("receiver resource failed: {reason:?}")
                    }
                    ResourceEventKind::OutboundComplete => {}
                }
            }
        })
        .await
        .expect("receiver resource completion deadline");

        assert_eq!(sender_hash, receiver_hash);
        assert!(!progress.is_empty(), "multi-part transfer emitted no progress");
        assert!(progress.windows(2).all(|pair| {
            pair[0].received_bytes <= pair[1].received_bytes
                && pair[0].received_parts <= pair[1].received_parts
        }));
        assert!(progress.iter().all(|value| {
            value.received_bytes <= value.total_bytes
                && value.received_parts <= value.total_parts
                && value.total_parts > 1
        }));

        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let state = alice
                    .app_context
                    .messaging()
                    .outbound_lifecycle(&message_id)
                    .expect("outbound lifecycle")
                    .expect("outbound route")
                    .0
                    .state;
                if state == "delivered" {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("verified resource completion delivery deadline");

        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let delivered = {
                    let store = bob.app_context.store().lock().expect("message store lock");
                    store
                        .list_messages(100, None)
                        .expect("list messages")
                        .into_iter()
                        .any(|message| message.direction == "in" && message.content == content)
                };
                if delivered {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("byte-identical message delivery deadline");

        assert_eq!(alice.transport.resource_state_counts().await.total(), 0);
        assert_eq!(bob.transport.resource_state_counts().await.total(), 0);
    })
    .await;
}
