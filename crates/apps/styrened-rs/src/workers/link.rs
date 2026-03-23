//! Link worker — subscribes to transport lifecycle link events and emits
//! typed DaemonEvent::Link through EventService.
//!
//! Handles:
//!   TransportLifecycleEvent::LinkActivated → DaemonEvent::Link { status: "active" }
//!   TransportLifecycleEvent::LinkClosed    → DaemonEvent::Link { status: "closed" }
//!   TransportLifecycleEvent::LinkRttUpdated → DaemonEvent::Link { status: "rtt_updated" }
//!
//! This is the single bridge point between the RNS transport layer and the IPC
//! event stream for link telemetry. The TUI subscribes to EventLink frames that
//! originate here.

use crate::services::EventService;
use crate::transport::mesh_transport::{MeshTransport, TransportLifecycleEvent};
use std::sync::Arc;
use styrene_ipc::types::LinkEvent;
use tokio::task::JoinHandle;

/// Spawn the link telemetry worker.
///
/// Subscribes to `MeshTransport::subscribe_lifecycle()` and forwards
/// `LinkActivated`, `LinkClosed`, and `LinkRttUpdated` events to `EventService`.
pub fn spawn_link_worker(
    transport: Arc<dyn MeshTransport>,
    events: Arc<EventService>,
) -> JoinHandle<()> {
    let mut rx = transport.subscribe_lifecycle();

    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(TransportLifecycleEvent::LinkActivated { link_id, peer_hash, rtt_ms }) => {
                    let ev = LinkEvent::new(&link_id, &peer_hash, "active", Some(rtt_ms));
                    events.emit_link_event(ev);
                }
                Ok(TransportLifecycleEvent::LinkClosed { link_id, peer_hash }) => {
                    let ev = LinkEvent::new(&link_id, &peer_hash, "closed", None);
                    events.emit_link_event(ev);
                }
                Ok(TransportLifecycleEvent::LinkRttUpdated { link_id, peer_hash, rtt_ms }) => {
                    let mut ev = LinkEvent::new(&link_id, &peer_hash, "rtt_updated", Some(rtt_ms));
                    ev.rtt_ms = Some(rtt_ms);
                    events.emit_link_event(ev);
                }
                // Connected/Disconnected/Reconnected — not link events, ignore
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    eprintln!("[link-worker] lagged, skipped {n} events");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    eprintln!("[link-worker] lifecycle channel closed, stopping");
                    break;
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::mock_transport::MockTransport;

    #[tokio::test]
    async fn link_activated_event_reaches_event_service() {
        let transport = Arc::new(MockTransport::new_default());
        let events = Arc::new(EventService::new());
        let mut rx = events.subscribe_links();

        let _handle = spawn_link_worker(transport.clone(), events.clone());

        transport.inject_lifecycle(TransportLifecycleEvent::LinkActivated {
            link_id: "aabbccdd11223344".into(),
            peer_hash: "deadbeef".repeat(4),
            rtt_ms: 42.5,
        });

        let event = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            rx.recv(),
        )
        .await
        .expect("timeout")
        .expect("channel closed");

        match event {
            styrene_ipc::types::DaemonEvent::Link { event: ev } => {
                assert_eq!(ev.link_id, "aabbccdd11223344");
                assert_eq!(ev.status, "active");
                assert_eq!(ev.rtt_ms, Some(42.5));
            }
            _ => panic!("expected Link event, got {event:?}"),
        }
    }

    #[tokio::test]
    async fn link_closed_event_reaches_event_service() {
        let transport = Arc::new(MockTransport::new_default());
        let events = Arc::new(EventService::new());
        let mut rx = events.subscribe_links();

        let _handle = spawn_link_worker(transport.clone(), events.clone());

        transport.inject_lifecycle(TransportLifecycleEvent::LinkClosed {
            link_id: "closelink".into(),
            peer_hash: "peerXXXX".into(),
        });

        let event = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            rx.recv(),
        )
        .await
        .expect("timeout")
        .expect("channel closed");

        match event {
            styrene_ipc::types::DaemonEvent::Link { event: ev } => {
                assert_eq!(ev.status, "closed");
            }
            _ => panic!("expected Link event"),
        }
    }

    #[tokio::test]
    async fn connected_lifecycle_events_are_ignored() {
        let transport = Arc::new(MockTransport::new_default());
        let events = Arc::new(EventService::new());
        let mut rx = events.subscribe_daemon_events();

        let _handle = spawn_link_worker(transport.clone(), events.clone());

        transport.inject_lifecycle(TransportLifecycleEvent::Connected);

        // No event should be emitted for Connected
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            rx.recv(),
        )
        .await;
        assert!(result.is_err(), "Connected should not emit a DaemonEvent");
    }
}
