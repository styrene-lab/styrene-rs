//! The Ratatui daemon layer over the shared IPC client, against a live
//! in-process daemon and IPC server.
//!
//! The TUI used to own socket framing and a private event reader. This test
//! connects through `styrene_tui::daemon::connect`, which now builds two
//! shared clients (commands and subscriptions), and proves that typed
//! queries answer and that pushed daemon events reach the TUI event stream.

use std::sync::Arc;
use std::time::Duration;

use styrene_e2e::helpers::{SETTLE, await_identity_resolved, await_inbound_message, with_timeout};
use styrene_e2e::node::{TestNode, TestNodeBuilder};
use styrene_ipc::traits::Daemon;
use styrene_tui::daemon::{self, TuiEvent};
use styrened::daemon_facade::DaemonFacade;

async fn start_ipc_server(node: &TestNode) -> (styrene_ipc_server::IpcServer, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let socket_path = dir.path().join("tui.sock");
    let facade = Arc::new(DaemonFacade::new(node.app_context.clone(), node.identity_hash.clone()))
        as Arc<dyn Daemon>;
    let config = styrene_ipc_server::IpcServerConfig {
        socket_path: socket_path.clone(),
        event_capacity: 64,
    };
    let mut server = styrene_ipc_server::IpcServer::new(facade, config);
    server.start().await.expect("start ipc server");
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

/// Wait for the first TUI event the predicate accepts, skipping the rest.
async fn await_tui_event(
    events: &mut tokio::sync::mpsc::Receiver<TuiEvent>,
    timeout: Duration,
    accept: impl Fn(&TuiEvent) -> bool,
) -> Option<TuiEvent> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match tokio::time::timeout(remaining, events.recv()).await {
            Ok(Some(event)) if accept(&event) => return Some(event),
            Ok(Some(_)) => continue,
            Ok(None) | Err(_) => return None,
        }
    }
}

#[tokio::test]
async fn tui_connects_queries_and_receives_pushed_events_through_the_shared_client() {
    with_timeout(async {
        let alice =
            TestNodeBuilder::new("alice-tui-client").tcp_server("127.0.0.1:0").build().await;
        let bob = TestNodeBuilder::new("bob-tui-client")
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

        // The TUI's own connect path: two shared clients, negotiated status,
        // and every subscription the TUI relies on.
        let mut connection = daemon::connect(Some(&socket_path)).await.expect("tui connect");
        let mut handle = connection.take_handle();
        let status = handle.status().await.expect("status through the shared client");
        assert!(status.connection_generation.is_some_and(|generation| generation != 0));
        let identity = handle.identity().await.expect("identity");
        assert_eq!(identity.identity_hash, bob.identity_hash);
        let _links = handle.links().await.expect("links answers through the shared client");

        // The initial snapshots the TUI queues before the event reader starts.
        let generation = await_tui_event(&mut connection.events, Duration::from_secs(5), |event| {
            matches!(event, TuiEvent::EventGeneration(_))
        })
        .await
        .expect("event connection generation");
        assert!(matches!(generation, TuiEvent::EventGeneration(value) if value != 0));

        // A pushed message event reaches the TUI stream through the client fanout.
        alice.send_chat(&bob.delivery_hash, "tui-shared-client").await.expect("send");
        await_inbound_message(&bob.app_context, Duration::from_secs(15)).await;
        let message = await_tui_event(&mut connection.events, Duration::from_secs(10), |event| {
            matches!(event, TuiEvent::Message(message) if message.content == "tui-shared-client")
        })
        .await
        .expect("pushed message reaches the TUI");
        let TuiEvent::Message(message) = message else { unreachable!() };
        assert_eq!(message.source_hash, alice.delivery_hash);
        assert!(!message.is_outgoing);

        // A typed query over the same command connection still answers afterwards.
        let (messages, _next, reset) =
            handle.message_page(&alice.delivery_hash, None).await.expect("message page");
        assert!(!reset);
        assert!(messages.iter().any(|entry| entry.content == "tui-shared-client"));
        assert!(handle.ping().await);
    })
    .await;
}
