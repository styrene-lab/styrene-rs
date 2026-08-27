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
use rns_core::transport::destination_ext::link::LinkCloseReason;
use std::sync::Arc;
use styrene_ipc::types::{
    LinkEvent, LinkEventKind, LinkLifecycleReason, ObservationMetadata, ObservationSource,
};
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
        reconcile_transport_links(transport.as_ref(), events.as_ref()).await;
        loop {
            match rx.recv().await {
                Ok(TransportLifecycleEvent::LinkActivated {
                    link_id,
                    peer_hash,
                    interface,
                    rtt_ms,
                }) => {
                    let mut ev = LinkEvent::new(&link_id, &peer_hash, "active", Some(rtt_ms));
                    ev.interface = interface;
                    events.emit_link_event(ev);
                }
                Ok(TransportLifecycleEvent::LinkIdentified {
                    link_id,
                    peer_hash,
                    interface,
                    rtt_ms,
                    remote_identity_hash,
                }) => {
                    let mut ev = LinkEvent::new(&link_id, &peer_hash, "active", rtt_ms);
                    ev.interface = interface;
                    ev.kind = LinkEventKind::Identified;
                    ev.identified = true;
                    ev.remote_identity_hash = Some(remote_identity_hash);
                    events.emit_link_event(ev);
                }
                Ok(TransportLifecycleEvent::LinkActivity {
                    link_id,
                    peer_hash,
                    interface,
                    rtt_ms,
                }) => {
                    let mut ev = LinkEvent::new(&link_id, &peer_hash, "active", rtt_ms);
                    ev.interface = interface;
                    ev.kind = LinkEventKind::Activity;
                    events.emit_link_event(ev);
                }
                Ok(TransportLifecycleEvent::LinkClosed {
                    link_id,
                    peer_hash,
                    interface,
                    rtt_ms,
                    reason,
                }) => {
                    let mut ev = LinkEvent::new(&link_id, &peer_hash, "closed", rtt_ms);
                    ev.interface = interface;
                    ev.kind = match reason {
                        LinkCloseReason::StaleTimeout
                        | LinkCloseReason::EstablishmentTimeout
                        | LinkCloseReason::ChannelTimeout => LinkEventKind::Timeout,
                        _ => LinkEventKind::Teardown,
                    };
                    ev.reason = Some(match reason {
                        LinkCloseReason::Teardown => LinkLifecycleReason::LocalTeardown,
                        LinkCloseReason::StaleTimeout => LinkLifecycleReason::StaleTimeout,
                        LinkCloseReason::EstablishmentTimeout => {
                            LinkLifecycleReason::EstablishmentTimeout
                        }
                        LinkCloseReason::ChannelTimeout => LinkLifecycleReason::ChannelTimeout,
                        LinkCloseReason::SendFailure => LinkLifecycleReason::SendFailure,
                    });
                    events.emit_link_event(ev);
                }
                Ok(TransportLifecycleEvent::LinkRttUpdated {
                    link_id,
                    peer_hash,
                    interface,
                    rtt_ms,
                }) => {
                    let mut ev = LinkEvent::new(&link_id, &peer_hash, "rtt_updated", Some(rtt_ms));
                    ev.interface = interface;
                    ev.rtt_ms = Some(rtt_ms);
                    events.emit_link_event(ev);
                }
                Ok(TransportLifecycleEvent::LinkReconcileRequired) => {
                    reconcile_transport_links(transport.as_ref(), events.as_ref()).await;
                }
                // Connected/Disconnected/Reconnected — not link events, ignore
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    crate::daemon_diagnostic!("[link-worker] lagged, skipped {n} events");
                    reconcile_transport_links(transport.as_ref(), events.as_ref()).await;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    crate::daemon_diagnostic!("[link-worker] lifecycle channel closed, stopping");
                    break;
                }
            }
        }
    })
}

pub(crate) fn link_event_from_state(
    snapshot: rns_core::transport::destination_ext::link::LinkStateSnapshot,
) -> LinkEvent {
    let observed_at = snapshot
        .observed_at
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    let status = match snapshot.status {
        rns_core::transport::destination_ext::link::LinkStatus::Stale => "stale",
        rns_core::transport::destination_ext::link::LinkStatus::Closed => "closed",
        _ => "active",
    };
    let mut event = LinkEvent::new(
        hex::encode(snapshot.id.as_slice()),
        hex::encode(snapshot.address_hash.as_slice()),
        status,
        snapshot.rtt.map(|rtt| rtt.as_secs_f64() * 1000.0),
    );
    event.interface = snapshot.interface.map(|interface| hex::encode(interface.as_slice()));
    if let Some(reason) = snapshot.close_reason {
        event.kind = match reason {
            LinkCloseReason::StaleTimeout
            | LinkCloseReason::EstablishmentTimeout
            | LinkCloseReason::ChannelTimeout => LinkEventKind::Timeout,
            _ => LinkEventKind::Teardown,
        };
        event.reason = Some(match reason {
            LinkCloseReason::Teardown => LinkLifecycleReason::LocalTeardown,
            LinkCloseReason::StaleTimeout => LinkLifecycleReason::StaleTimeout,
            LinkCloseReason::EstablishmentTimeout => LinkLifecycleReason::EstablishmentTimeout,
            LinkCloseReason::ChannelTimeout => LinkLifecycleReason::ChannelTimeout,
            LinkCloseReason::SendFailure => LinkLifecycleReason::SendFailure,
        });
    } else {
        event.kind = LinkEventKind::Activity;
    }
    event.identified = snapshot.remote_identity.is_some();
    event.remote_identity_hash =
        snapshot.remote_identity.map(|identity| hex::encode(identity.as_slice()));
    event.timestamp = observed_at;
    event.observation = ObservationMetadata::at(
        ObservationSource::TransportLinkState,
        Some(observed_at),
        observed_at.saturating_add(snapshot.age.as_secs() as i64),
        300,
    );
    event
}

async fn reconcile_transport_links(transport: &dyn MeshTransport, events: &EventService) {
    let lifecycle = transport.link_lifecycle_snapshot().await;
    let active = lifecycle.active.into_iter().map(link_event_from_state).collect();
    let history = lifecycle.history.into_iter().map(link_event_from_state).collect();
    events.reconcile_links(active, history);
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
            interface: Some("interface-1".into()),
            rtt_ms: 42.5,
        });

        let event = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .expect("timeout")
            .expect("channel closed");

        match event {
            styrene_ipc::types::DaemonEvent::Link { event: ev } => {
                assert_eq!(ev.link_id, "aabbccdd11223344");
                assert_eq!(ev.status, "active");
                assert_eq!(ev.interface.as_deref(), Some("interface-1"));
                assert_eq!(ev.kind, LinkEventKind::Established);
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
            interface: Some("interface-2".into()),
            rtt_ms: Some(10.0),
            reason: LinkCloseReason::StaleTimeout,
        });

        let event = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .expect("timeout")
            .expect("channel closed");

        match event {
            styrene_ipc::types::DaemonEvent::Link { event: ev } => {
                assert_eq!(ev.status, "closed");
                assert_eq!(ev.kind, LinkEventKind::Timeout);
                assert_eq!(ev.reason, Some(LinkLifecycleReason::StaleTimeout));
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
        let result = tokio::time::timeout(std::time::Duration::from_millis(50), rx.recv()).await;
        assert!(result.is_err(), "Connected should not emit a DaemonEvent");
    }

    #[tokio::test]
    async fn lifecycle_lag_reconciles_active_links_from_transport_state() {
        use rns_core::hash::AddressHash;
        use rns_core::transport::destination_ext::link::{LinkStateSnapshot, LinkStatus};

        let transport = Arc::new(MockTransport::new_default());
        let events = Arc::new(EventService::with_capacity(4, 8));
        transport.set_link_snapshots(vec![LinkStateSnapshot {
            id: AddressHash::new([1; 16]),
            address_hash: AddressHash::new([2; 16]),
            interface: Some(AddressHash::new([3; 16])),
            rtt: Some(std::time::Duration::from_millis(7)),
            status: LinkStatus::Active,
            remote_identity: Some(AddressHash::new([4; 16])),
            observed_at: std::time::SystemTime::now(),
            age: std::time::Duration::from_secs(1),
            close_reason: None,
        }]);
        transport.set_terminal_link_snapshots(vec![LinkStateSnapshot {
            id: AddressHash::new([5; 16]),
            address_hash: AddressHash::new([6; 16]),
            interface: Some(AddressHash::new([7; 16])),
            rtt: Some(std::time::Duration::from_millis(11)),
            status: LinkStatus::Closed,
            remote_identity: Some(AddressHash::new([8; 16])),
            observed_at: std::time::SystemTime::now(),
            age: std::time::Duration::from_secs(2),
            close_reason: Some(LinkCloseReason::SendFailure),
        }]);
        let _handle = spawn_link_worker(transport.clone(), events.clone());
        tokio::task::yield_now().await;

        events.emit_link_event(LinkEvent::new("stale-cache", "old-peer", "active", None));
        transport.inject_lifecycle(TransportLifecycleEvent::LinkClosed {
            link_id: hex::encode([5; 16]),
            peer_hash: hex::encode([6; 16]),
            interface: Some(hex::encode([7; 16])),
            rtt_ms: Some(11.0),
            reason: LinkCloseReason::SendFailure,
        });
        for _ in 0..32 {
            transport.inject_lifecycle(TransportLifecycleEvent::Connected);
        }

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                let snapshot = events.link_snapshot();
                if snapshot.active.len() == 1 && snapshot.active[0].link_id == hex::encode([1; 16])
                {
                    assert_eq!(snapshot.active[0].remote_identity_hash, Some(hex::encode([4; 16])));
                    assert!(snapshot.history.iter().any(|event| event.link_id == "stale-cache"));
                    let terminal_id = hex::encode([5; 16]);
                    assert!(!snapshot.active.iter().any(|event| event.link_id == terminal_id));
                    let terminal = snapshot
                        .history
                        .iter()
                        .filter(|event| event.link_id == terminal_id)
                        .collect::<Vec<_>>();
                    assert_eq!(terminal.len(), 1);
                    assert_eq!(terminal[0].reason, Some(LinkLifecycleReason::SendFailure));
                    assert_eq!(terminal[0].remote_identity_hash, Some(hex::encode([8; 16])));
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("lag reconciliation");
    }
}
