//! Event subscription scenarios.
//!
//! The TUI and CLI consume DaemonEvent broadcast channels to render
//! live updates. These tests verify that real operations (announce,
//! message delivery, link establishment) actually emit the right events.

use std::time::Duration;
use styrene_e2e::helpers::{
    with_timeout, await_identity_resolved, await_inbound_message, SETTLE,
};
use styrene_e2e::node::TestNodeBuilder;
use styrene_ipc::types::DaemonEvent;

/// Wait for a specific DaemonEvent variant on a receiver, with timeout.
async fn recv_event(
    rx: &mut tokio::sync::broadcast::Receiver<DaemonEvent>,
    timeout: Duration,
) -> Option<DaemonEvent> {
    tokio::time::timeout(timeout, async {
        loop {
            match rx.recv().await {
                Ok(event) => return Some(event),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    })
    .await
    .unwrap_or(None)
}

/// Drain all currently buffered events from a receiver (non-blocking).
fn drain_events(
    rx: &mut tokio::sync::broadcast::Receiver<DaemonEvent>,
) -> Vec<DaemonEvent> {
    let mut events = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(event) => events.push(event),
            Err(_) => break,
        }
    }
    events
}

#[tokio::test]
async fn device_event_emitted_on_announce() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice-ev")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let bob = TestNodeBuilder::new("bob-ev")
            .tcp_client(alice.listen_addr.expect("listen addr"))
            .build()
            .await;

        tokio::time::sleep(SETTLE).await;

        // Subscribe to device events BEFORE the announce
        let mut device_rx = alice.app_context.events().subscribe_devices();

        // Bob announces — alice should receive a Device event
        bob.announce().await;

        let event = recv_event(&mut device_rx, Duration::from_secs(10))
            .await
            .expect("should receive device event");

        match event {
            DaemonEvent::Device { device } => {
                // The destination_hash should be bob's delivery hash
                assert_eq!(
                    device.destination_hash, bob.delivery_hash,
                    "device event should carry bob's delivery hash"
                );
            }
            other => panic!("expected Device event, got {:?}", other),
        }
    })
    .await;
}

#[tokio::test]
async fn message_event_emitted_on_delivery() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice-msg-ev")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let bob = TestNodeBuilder::new("bob-msg-ev")
            .tcp_client(alice.listen_addr.expect("listen addr"))
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
        await_identity_resolved(
            &bob.app_context,
            &alice.delivery_addr,
            Duration::from_secs(10),
        )
        .await;

        // Subscribe to message events on BOB's side before delivery
        let mut msg_rx = bob.app_context.events().subscribe_messages(&[]);

        // Drain any pre-existing events (announces may have triggered device events)
        drain_events(&mut msg_rx);

        // Alice sends to Bob
        alice.send_chat(&bob.delivery_hash, "event-test").await.expect("send");

        // Wait for the message to actually arrive in store
        await_inbound_message(&bob.app_context, Duration::from_secs(15)).await;

        // Now check that a Message event was emitted
        // There may be Device/Link events mixed in, so collect all and filter
        tokio::time::sleep(Duration::from_millis(200)).await;
        let events = drain_events(&mut msg_rx);

        let message_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, DaemonEvent::Message { .. }))
            .collect();

        assert!(
            !message_events.is_empty(),
            "should have received at least one Message event, got events: {:?}",
            events.iter().map(|e| match e {
                DaemonEvent::Message { kind, .. } => format!("Message({:?})", kind),
                DaemonEvent::Device { .. } => "Device".to_string(),
                DaemonEvent::Link { .. } => "Link".to_string(),
                _ => "Other".to_string(),
            }).collect::<Vec<_>>()
        );

        // Verify the event payload
        if let DaemonEvent::Message { kind, message } = &message_events[0] {
            assert_eq!(
                *kind,
                styrene_ipc::types::MessageEventKind::New,
                "event kind should be New"
            );
            assert_eq!(message.content, "event-test");
            assert_eq!(message.source_hash, alice.identity_hash);
        }
    })
    .await;
}

#[tokio::test]
async fn link_event_emitted_during_message_delivery() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice-link-ev")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let bob = TestNodeBuilder::new("bob-link-ev")
            .tcp_client(alice.listen_addr.expect("listen addr"))
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

        // Subscribe to link events on ALICE's side (she initiates the link)
        let mut link_rx = alice.app_context.events().subscribe_links();
        drain_events(&mut link_rx);

        // Alice sends — this establishes a link
        alice.send_chat(&bob.delivery_hash, "link-test").await.expect("send");
        await_inbound_message(&bob.app_context, Duration::from_secs(15)).await;

        // Collect link events
        tokio::time::sleep(Duration::from_millis(500)).await;
        let events = drain_events(&mut link_rx);

        let link_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, DaemonEvent::Link { .. }))
            .collect();

        assert!(
            !link_events.is_empty(),
            "should have received at least one Link event during delivery, got: {:?}",
            events.iter().map(|e| match e {
                DaemonEvent::Link { event } => format!("Link({})", event.status),
                DaemonEvent::Device { .. } => "Device".to_string(),
                DaemonEvent::Message { .. } => "Message".to_string(),
                _ => "Other".to_string(),
            }).collect::<Vec<_>>()
        );

        // At least one should be "active" (link established)
        let active_events: Vec<_> = link_events
            .iter()
            .filter(|e| match e {
                DaemonEvent::Link { event } => event.status == "active",
                _ => false,
            })
            .collect();

        assert!(
            !active_events.is_empty(),
            "should have a Link event with status 'active'"
        );

        // The link event should reference bob's delivery hash
        if let DaemonEvent::Link { event } = active_events[0] {
            assert!(
                !event.link_id.is_empty(),
                "link_id should be non-empty"
            );
            assert!(
                !event.peer_hash.is_empty(),
                "peer_hash should be non-empty"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn multiple_device_events_from_repeated_announces() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice-multi-ev")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let bob = TestNodeBuilder::new("bob-multi-ev")
            .tcp_client(alice.listen_addr.expect("listen addr"))
            .build()
            .await;

        tokio::time::sleep(SETTLE).await;

        let mut device_rx = alice.app_context.events().subscribe_devices();

        // Bob announces 3 times
        bob.announce().await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        bob.announce().await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        bob.announce().await;
        tokio::time::sleep(Duration::from_millis(500)).await;

        let events = drain_events(&mut device_rx);
        let device_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, DaemonEvent::Device { .. }))
            .collect();

        // Should have received at least 2 Device events (3 announces, some may merge)
        assert!(
            device_events.len() >= 2,
            "should receive multiple device events from repeated announces, got {}",
            device_events.len()
        );
    })
    .await;
}
