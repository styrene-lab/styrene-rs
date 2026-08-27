//! Announce worker — subscribes to transport announce events,
//! processes them through DiscoveryService, and emits DaemonEvents.

use crate::services::discovery::NATIVE_NOMADNET_HOST_DEVICE_TYPE;
use crate::services::{DiscoveryService, EventService};
use crate::transport::mesh_transport::MeshTransport;
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
                    } else {
                        discovery.accept_announce_with_type(
                            peer_hash.clone(),
                            timestamp,
                            app_data,
                            device_type,
                        )
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
