//! Announce worker — subscribes to transport announce events,
//! processes them through DiscoveryService, and emits DaemonEvents.

use crate::services::discovery::NATIVE_NOMADNET_HOST_DEVICE_TYPE;
use crate::services::{DiscoveryService, EventService};
use crate::transport::mesh_transport::{MeshTransport, TransportLifecycleEvent};
use rns_core::destination::{DestinationName, NAME_HASH_LENGTH};
use rns_core::transport::time::now_epoch_secs_i64;
use std::sync::Arc;
use tokio::task::JoinHandle;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnnounceProcessingMilestone {
    pub destination_hash: String,
    pub accepted: bool,
}

/// Compute the name hash prefix for a destination aspect.
fn aspect_hash_prefix(app: &str, aspect: &str) -> [u8; NAME_HASH_LENGTH] {
    let name = DestinationName::new(app, aspect);
    let mut prefix = [0u8; NAME_HASH_LENGTH];
    prefix.copy_from_slice(&name.hash.as_slice()[..NAME_HASH_LENGTH]);
    prefix
}

/// Spawn the announce processing worker.
///
/// Subscribes to transport announce events and:
/// 1. Feeds announces to DiscoveryService (peer table + DB)
/// 2. Classifies by aspect (lxmf.delivery, lxmf.propagation, or nomadnetwork.node)
/// 3. Emits DaemonEvent::Device via EventService
///
/// Returns a JoinHandle for the spawned task.
pub fn spawn_announce_worker(
    transport: Arc<dyn MeshTransport>,
    discovery: Arc<DiscoveryService>,
    events: Arc<EventService>,
) -> JoinHandle<()> {
    spawn_announce_worker_inner(transport, discovery, events, None)
}

pub fn spawn_announce_worker_with_milestones(
    transport: Arc<dyn MeshTransport>,
    discovery: Arc<DiscoveryService>,
    events: Arc<EventService>,
    milestones: tokio::sync::mpsc::UnboundedSender<AnnounceProcessingMilestone>,
) -> JoinHandle<()> {
    spawn_announce_worker_inner(transport, discovery, events, Some(milestones))
}

fn spawn_announce_worker_inner(
    transport: Arc<dyn MeshTransport>,
    discovery: Arc<DiscoveryService>,
    events: Arc<EventService>,
    milestones: Option<tokio::sync::mpsc::UnboundedSender<AnnounceProcessingMilestone>>,
) -> JoinHandle<()> {
    let mut rx = transport.subscribe_announces();

    // Pre-compute aspect name hashes for classification
    let nomadnet_hash = aspect_hash_prefix("nomadnetwork", "node");
    let delivery_hash = aspect_hash_prefix("lxmf", "delivery");
    let propagation_hash = aspect_hash_prefix("lxmf", "propagation");

    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let dest = event.destination.lock().await;
                    let peer_hash = hex::encode(dest.desc.address_hash.as_slice());
                    let mut identity_hash = [0u8; 16];
                    identity_hash.copy_from_slice(dest.desc.identity.address_hash.as_slice());
                    let mut propagation_destination = [0u8; 16];
                    propagation_destination.copy_from_slice(dest.desc.address_hash.as_slice());
                    drop(dest);

                    // Classify by aspect
                    let is_page_host = event.name_hash == nomadnet_hash;
                    let is_delivery = event.name_hash == delivery_hash;
                    let is_propagation_host = event.name_hash == propagation_hash;

                    let timestamp = now_epoch_secs_i64();
                    let app_data = event.app_data.as_slice();

                    let device_type =
                        if is_page_host { Some(NATIVE_NOMADNET_HOST_DEVICE_TYPE) } else { None };

                    let result = if is_propagation_host {
                        lxmf::propagation_announce::StandardPropagationAnnounce::parse(app_data)
                            .map_err(|_| {
                                std::io::Error::other("invalid standard propagation app data")
                            })
                            .and_then(|metadata| {
                                discovery.accept_standard_propagation_announce(
                                    peer_hash.clone(),
                                    identity_hash,
                                    propagation_destination,
                                    timestamp,
                                    &metadata,
                                )
                            })
                    } else if is_delivery {
                        discovery.accept_delivery_announce(peer_hash.clone(), timestamp, app_data)
                    } else if is_page_host {
                        discovery.accept_announce_with_type(
                            peer_hash.clone(),
                            timestamp,
                            app_data,
                            device_type,
                        )
                    } else {
                        Err(std::io::Error::other("unsupported announce aspect"))
                    };
                    match result {
                        Ok(record) => {
                            let aspect = if is_page_host {
                                "nomadnetwork.node"
                            } else if is_propagation_host {
                                "lxmf.propagation"
                            } else {
                                "lxmf.delivery"
                            };
                            crate::daemon_diagnostic!(
                                "[worker] announce from {} (name={:?}, aspect={}, seen={})",
                                peer_hash,
                                record.name,
                                aspect,
                                record.seen_count
                            );
                            if let Some(device) = discovery.device(&peer_hash) {
                                events.emit_device(device);
                            }
                            if is_propagation_host {
                                events.emit_standard_propagation_changed(timestamp);
                            }
                            if let Some(milestones) = &milestones {
                                let _ = milestones.send(AnnounceProcessingMilestone {
                                    destination_hash: peer_hash,
                                    accepted: true,
                                });
                            }
                        }
                        Err(e) => {
                            crate::daemon_diagnostic!(
                                "[worker] announce processing error for {}: {e}",
                                peer_hash
                            );
                            if let Some(milestones) = &milestones {
                                let _ = milestones.send(AnnounceProcessingMilestone {
                                    destination_hash: peer_hash,
                                    accepted: false,
                                });
                            }
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    crate::daemon_diagnostic!(
                        "[worker] announce worker lagged, skipped {n} events"
                    );
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    crate::daemon_diagnostic!("[worker] announce channel closed, worker stopping");
                    break;
                }
            }
        }
    })
}

/// Announce the local delivery destination whenever the transport becomes
/// connected. A peer that only ever hears the node's announce at first start
/// loses it on its own restart, and every later message from this node then
/// arrives unverified. Announcing on every connect and reconnect keeps the
/// node resolvable by the peers it actually talks to.
pub fn spawn_connect_announce_worker(
    transport: Arc<dyn MeshTransport>,
) -> tokio::task::JoinHandle<()> {
    const SETTLE: std::time::Duration = std::time::Duration::from_millis(750);
    const POLL: std::time::Duration = std::time::Duration::from_secs(1);
    tokio::spawn(async move {
        // The adapter reports Disconnected on shutdown but never Connected, so
        // the worker watches the connected state itself and treats every
        // false-to-true transition as a (re)connect, alongside any lifecycle
        // event that does arrive.
        let mut lifecycle = transport.subscribe_lifecycle();
        let mut was_connected = false;
        let mut poll = tokio::time::interval(POLL);
        loop {
            tokio::select! {
                _ = poll.tick() => {
                    let connected = transport.is_connected();
                    if connected && !was_connected {
                        tokio::time::sleep(SETTLE).await;
                        announce_on_connect(transport.as_ref(), "connect").await;
                    }
                    was_connected = connected;
                }
                event = lifecycle.recv() => match event {
                    Ok(TransportLifecycleEvent::Connected | TransportLifecycleEvent::Reconnected) => {
                        tokio::time::sleep(SETTLE).await;
                        announce_on_connect(transport.as_ref(), "reconnect").await;
                        was_connected = true;
                    }
                    Ok(TransportLifecycleEvent::Disconnected) => was_connected = false,
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    })
}

async fn announce_on_connect(transport: &dyn MeshTransport, reason: &str) {
    match transport.dispatch_announce(None).await {
        Ok(()) => crate::daemon_diagnostic!("[worker] announced on transport {reason}"),
        Err(error) => {
            crate::daemon_diagnostic!("[worker] announce on transport {reason} rejected: {error}");
        }
    }
}

#[cfg(test)]
mod connect_announce_tests {
    use super::*;
    use crate::transport::mock_transport::{MockCall, MockTransport};
    use std::time::Duration;

    fn announce_count(transport: &MockTransport) -> usize {
        transport.calls().iter().filter(|call| matches!(call, MockCall::Announce { .. })).count()
    }

    #[tokio::test]
    async fn announces_on_start_and_on_every_reconnect() {
        let transport = Arc::new(MockTransport::new_default());
        transport.set_connected(true);
        let worker = spawn_connect_announce_worker(transport.clone());
        tokio::time::sleep(Duration::from_millis(2_200)).await;
        assert_eq!(announce_count(&transport), 1, "announces once when already connected");

        transport.inject_lifecycle(TransportLifecycleEvent::Reconnected);
        tokio::time::sleep(Duration::from_millis(1_100)).await;
        assert_eq!(announce_count(&transport), 2, "announces again after a reconnect");

        transport.inject_lifecycle(TransportLifecycleEvent::Disconnected);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(announce_count(&transport), 2, "a disconnect does not announce");
        worker.abort();
    }

    #[tokio::test]
    async fn does_not_announce_while_disconnected_at_start() {
        let transport = Arc::new(MockTransport::new_default());
        transport.set_connected(false);
        let worker = spawn_connect_announce_worker(transport.clone());
        tokio::time::sleep(Duration::from_millis(1_100)).await;
        assert_eq!(announce_count(&transport), 0);
        transport.set_connected(true);
        tokio::time::sleep(Duration::from_millis(2_200)).await;
        assert_eq!(
            announce_count(&transport),
            1,
            "announces once the transport connects, with no lifecycle event at all"
        );
        worker.abort();
    }
}
