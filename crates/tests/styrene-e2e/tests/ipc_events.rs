//! IPC event streaming over Unix socket wire protocol.
//!
//! Verifies that events emitted by the daemon arrive as pushed frames
//! on the IPC socket when a client subscribes. This is the path the
//! TUI and CLI event streams depend on.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::UnixStream;

use styrene_e2e::helpers::{await_identity_resolved, await_inbound_message, with_timeout, SETTLE};
use styrene_e2e::node::TestNodeBuilder;
use styrene_ipc::traits::Daemon;
use styrene_ipc_server::wire::{self, MessageType, REQUEST_ID_SIZE};
use styrened::daemon_facade::DaemonFacade;

fn random_request_id() -> [u8; REQUEST_ID_SIZE] {
    let mut id = [0u8; 16];
    rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut id);
    id
}

fn empty_payload() -> HashMap<String, rmpv::Value> {
    HashMap::new()
}

async fn start_ipc_server(
    node: &styrene_e2e::node::TestNode,
) -> (styrene_ipc_server::IpcServer, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let socket_path = dir.path().join("test.sock");
    let facade = Arc::new(DaemonFacade::new(node.app_context.clone(), node.identity_hash.clone()))
        as Arc<dyn Daemon>;
    let config = styrene_ipc_server::IpcServerConfig {
        socket_path: socket_path.clone(),
        event_capacity: 64,
    };
    let mut server = styrene_ipc_server::IpcServer::new(facade, config);
    server.start().await.expect("start ipc server");

    // Bridge daemon events to the IPC server's event broadcast channel.
    // The EventService emits DaemonEvent via its own broadcast sender.
    // The IpcServer has a separate broadcast sender for pushing to clients.
    // This forwarder connects them.
    let event_tx = server.event_sender();
    let mut daemon_rx = node.app_context.events().subscribe_daemon_events();
    tokio::spawn(async move {
        loop {
            match daemon_rx.recv().await {
                Ok(event) => {
                    let _ = event_tx.send(event);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    std::mem::forget(dir);
    (server, socket_path)
}

/// Send a subscription request and read the Result confirmation.
async fn subscribe(stream: &mut UnixStream, topic: MessageType) {
    let req_id = random_request_id();
    let (mut read, mut write) = stream.split();
    wire::write_frame_async(&mut write, topic, &req_id, &empty_payload())
        .await
        .expect("write subscribe");
    let frame = wire::read_frame_async(&mut read).await.expect("read subscribe response");
    assert_eq!(frame.msg_type, MessageType::Result, "subscribe should return Result");
}

/// Read frames from the socket until we find an event of the given type,
/// or timeout. Skips response frames (non-zero request_id) and events
/// of other types.
async fn read_until_event(
    stream: &mut UnixStream,
    expected_type: MessageType,
    timeout: Duration,
) -> Option<wire::Frame> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }

        let (mut read, _) = stream.split();
        let result = tokio::time::timeout(remaining, wire::read_frame_async(&mut read)).await;

        match result {
            Ok(Ok(frame)) => {
                if frame.msg_type == expected_type {
                    return Some(frame);
                }
                // Skip non-matching frames (responses or other event types)
                continue;
            }
            Ok(Err(_)) => return None, // read error
            Err(_) => return None,     // timeout
        }
    }
}

/// Drain all available event frames from the socket (non-blocking after initial wait).
async fn drain_event_frames(stream: &mut UnixStream, wait: Duration) -> Vec<wire::Frame> {
    tokio::time::sleep(wait).await;
    let mut frames = Vec::new();
    loop {
        let (mut read, _) = stream.split();
        match tokio::time::timeout(Duration::from_millis(100), wire::read_frame_async(&mut read))
            .await
        {
            Ok(Ok(frame)) if frame.msg_type.is_event() => {
                frames.push(frame);
            }
            Ok(Ok(_)) => continue, // non-event frame, skip
            _ => break,            // timeout or error
        }
    }
    frames
}

// ── Tests ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn ipc_subscribe_messages_receives_new_event() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice-ipc-ev").tcp_server("127.0.0.1:0").build().await;
        let bob = TestNodeBuilder::new("bob-ipc-ev")
            .tcp_client(alice.listen_addr.expect("addr"))
            .build()
            .await;

        tokio::time::sleep(SETTLE).await;
        alice.announce().await;
        bob.announce().await;
        await_identity_resolved(&alice.app_context, &bob.delivery_addr, Duration::from_secs(10))
            .await;

        // Start IPC server on bob
        let (_server, socket_path) = start_ipc_server(&bob).await;
        tokio::time::sleep(SETTLE).await;

        // Connect and subscribe to messages
        let mut stream = UnixStream::connect(&socket_path).await.expect("connect");
        subscribe(&mut stream, MessageType::SubMessages).await;

        // Alice sends a message to bob
        alice.send_chat(&bob.delivery_hash, "ipc-event-msg").await.expect("send");
        await_inbound_message(&bob.app_context, Duration::from_secs(15)).await;

        // Read the pushed EventMessage frame
        let event =
            read_until_event(&mut stream, MessageType::EventMessage, Duration::from_secs(5))
                .await
                .expect("should receive EventMessage");

        // Verify zero request_id (pushed event, not response)
        assert_eq!(event.request_id, [0u8; 16], "event should have zero request_id");

        // Verify payload
        assert_eq!(event.payload.get("kind").and_then(|v| v.as_str()), Some("new"));
        assert_eq!(event.payload.get("content").and_then(|v| v.as_str()), Some("ipc-event-msg"));
        assert_eq!(
            event.payload.get("source_hash").and_then(|v| v.as_str()),
            Some(alice.identity_hash.as_str())
        );
    })
    .await;
}

#[tokio::test]
async fn ipc_subscribe_devices_receives_announce_event() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice-ipc-dev").tcp_server("127.0.0.1:0").build().await;
        let bob = TestNodeBuilder::new("bob-ipc-dev")
            .tcp_client(alice.listen_addr.expect("addr"))
            .build()
            .await;

        tokio::time::sleep(SETTLE).await;

        // Start IPC server on alice, subscribe to devices
        let (_server, socket_path) = start_ipc_server(&alice).await;
        tokio::time::sleep(SETTLE).await;

        let mut stream = UnixStream::connect(&socket_path).await.expect("connect");
        subscribe(&mut stream, MessageType::SubDevices).await;

        // Bob announces — alice should push EventDevice
        bob.announce().await;

        let event =
            read_until_event(&mut stream, MessageType::EventDevice, Duration::from_secs(10))
                .await
                .expect("should receive EventDevice");

        assert_eq!(event.request_id, [0u8; 16]);
        assert_eq!(
            event.payload.get("destination_hash").and_then(|v| v.as_str()),
            Some(bob.delivery_hash.as_str())
        );
    })
    .await;
}

#[tokio::test]
async fn ipc_subscribe_links_receives_active_event() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice-ipc-link").tcp_server("127.0.0.1:0").build().await;
        let bob = TestNodeBuilder::new("bob-ipc-link")
            .tcp_client(alice.listen_addr.expect("addr"))
            .build()
            .await;

        tokio::time::sleep(SETTLE).await;
        alice.announce().await;
        bob.announce().await;
        await_identity_resolved(&alice.app_context, &bob.delivery_addr, Duration::from_secs(10))
            .await;

        // Subscribe to links on alice's IPC
        let (_server, socket_path) = start_ipc_server(&alice).await;
        tokio::time::sleep(SETTLE).await;

        let mut stream = UnixStream::connect(&socket_path).await.expect("connect");
        subscribe(&mut stream, MessageType::SubLinks).await;

        // Send a message — this establishes a link
        alice.send_chat(&bob.delivery_hash, "link-trigger").await.expect("send");
        await_inbound_message(&bob.app_context, Duration::from_secs(15)).await;

        // Read link event
        let event = read_until_event(&mut stream, MessageType::EventLink, Duration::from_secs(5))
            .await
            .expect("should receive EventLink");

        assert_eq!(event.request_id, [0u8; 16]);
        let status = event.payload.get("status").and_then(|v| v.as_str());
        assert_eq!(status, Some("active"), "link event should have status 'active'");

        let link_id = event.payload.get("link_id").and_then(|v| v.as_str());
        assert!(link_id.is_some() && !link_id.expect("link_id").is_empty());
    })
    .await;
}

#[tokio::test]
async fn ipc_unsubscribe_stops_events() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice-unsub").tcp_server("127.0.0.1:0").build().await;
        let bob = TestNodeBuilder::new("bob-unsub")
            .tcp_client(alice.listen_addr.expect("addr"))
            .build()
            .await;

        tokio::time::sleep(SETTLE).await;

        let (_server, socket_path) = start_ipc_server(&alice).await;
        tokio::time::sleep(SETTLE).await;

        let mut stream = UnixStream::connect(&socket_path).await.expect("connect");

        // Subscribe to devices
        subscribe(&mut stream, MessageType::SubDevices).await;

        // Verify event arrives
        bob.announce().await;
        let event =
            read_until_event(&mut stream, MessageType::EventDevice, Duration::from_secs(10)).await;
        assert!(event.is_some(), "should receive event while subscribed");

        // Unsubscribe
        let req_id = random_request_id();
        let (mut read, mut write) = stream.split();
        wire::write_frame_async(&mut write, MessageType::Unsub, &req_id, &empty_payload())
            .await
            .expect("write unsub");
        let frame = wire::read_frame_async(&mut read).await.expect("read unsub response");
        assert_eq!(frame.msg_type, MessageType::Result);

        // Trigger another announce
        bob.announce().await;
        tokio::time::sleep(Duration::from_secs(1)).await;

        // Should NOT receive any more events
        let late_event =
            read_until_event(&mut stream, MessageType::EventDevice, Duration::from_secs(2)).await;
        assert!(late_event.is_none(), "should NOT receive events after unsubscribe");
    })
    .await;
}

#[tokio::test]
async fn ipc_multiple_subscriptions_on_same_connection() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice-multi-sub").tcp_server("127.0.0.1:0").build().await;
        let bob = TestNodeBuilder::new("bob-multi-sub")
            .tcp_client(alice.listen_addr.expect("addr"))
            .build()
            .await;

        tokio::time::sleep(SETTLE).await;
        alice.announce().await;
        bob.announce().await;
        await_identity_resolved(&alice.app_context, &bob.delivery_addr, Duration::from_secs(10))
            .await;

        let (_server, socket_path) = start_ipc_server(&bob).await;
        tokio::time::sleep(SETTLE).await;

        let mut stream = UnixStream::connect(&socket_path).await.expect("connect");

        // Subscribe to both messages AND devices
        subscribe(&mut stream, MessageType::SubMessages).await;
        subscribe(&mut stream, MessageType::SubDevices).await;

        // Trigger both: alice announces (device event) and sends message (message event)
        alice.announce().await;
        tokio::time::sleep(Duration::from_millis(500)).await;

        alice.send_chat(&bob.delivery_hash, "multi-sub-test").await.expect("send");
        await_inbound_message(&bob.app_context, Duration::from_secs(15)).await;

        // Collect all events
        let events = drain_event_frames(&mut stream, Duration::from_secs(1)).await;

        let device_events: Vec<_> =
            events.iter().filter(|f| f.msg_type == MessageType::EventDevice).collect();
        let message_events: Vec<_> =
            events.iter().filter(|f| f.msg_type == MessageType::EventMessage).collect();

        assert!(!device_events.is_empty(), "should receive at least one device event");
        assert!(!message_events.is_empty(), "should receive at least one message event");
    })
    .await;
}
