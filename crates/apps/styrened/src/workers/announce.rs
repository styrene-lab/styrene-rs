//! Announce worker — subscribes to transport announce events,
//! processes them through DiscoveryService, and emits DaemonEvents.

use crate::services::{DiscoveryService, EventService};
use crate::transport::mesh_transport::MeshTransport;
use rns_core::destination::{DestinationName, NAME_HASH_LENGTH};
use rns_core::transport::time::now_epoch_secs_i64;
use std::sync::Arc;
use tokio::task::JoinHandle;

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
/// 2. Classifies by aspect (lxmf.delivery vs nomadnetwork.node)
/// 3. Emits DaemonEvent::Device via EventService
///
/// Returns a JoinHandle for the spawned task.
pub fn spawn_announce_worker(
    transport: Arc<dyn MeshTransport>,
    discovery: Arc<DiscoveryService>,
    events: Arc<EventService>,
) -> JoinHandle<()> {
    let mut rx = transport.subscribe_announces();

    // Pre-compute aspect name hashes for classification
    let nomadnet_hash = aspect_hash_prefix("nomadnetwork", "node");

    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let dest = event.destination.lock().await;
                    let peer_hash = hex::encode(dest.desc.address_hash.as_slice());
                    drop(dest);

                    // Classify by aspect
                    let is_page_host = event.name_hash == nomadnet_hash;

                    let timestamp = now_epoch_secs_i64();
                    let app_data = event.app_data.as_slice();

                    let device_type = if is_page_host { Some("page_host") } else { None };

                    match discovery.accept_announce_with_type(
                        peer_hash.clone(),
                        timestamp,
                        app_data,
                        device_type,
                    ) {
                        Ok(record) => {
                            let aspect =
                                if is_page_host { "nomadnetwork.node" } else { "lxmf.delivery" };
                            eprintln!(
                                "[worker] announce from {} (name={:?}, aspect={}, seen={})",
                                peer_hash, record.name, aspect, record.seen_count
                            );
                            events.emit_device_update(&peer_hash);
                        }
                        Err(e) => {
                            eprintln!("[worker] announce processing error for {}: {e}", peer_hash);
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    eprintln!("[worker] announce worker lagged, skipped {n} events");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    eprintln!("[worker] announce channel closed, worker stopping");
                    break;
                }
            }
        }
    })
}
