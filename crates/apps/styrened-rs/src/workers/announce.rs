//! Announce worker — subscribes to transport announce events,
//! processes them through DiscoveryService, and emits DaemonEvents.

use crate::services::{DiscoveryService, EventService};
use crate::transport::mesh_transport::MeshTransport;
use rns_core::transport::time::now_epoch_secs_i64;
use std::sync::Arc;
use tokio::task::JoinHandle;

/// Spawn the announce processing worker.
///
/// Subscribes to transport announce events and:
/// 1. Feeds announces to DiscoveryService (peer table + DB)
/// 2. Emits DaemonEvent::Device via EventService
///
/// Returns a JoinHandle for the spawned task.
pub fn spawn_announce_worker(
    transport: Arc<dyn MeshTransport>,
    discovery: Arc<DiscoveryService>,
    events: Arc<EventService>,
) -> JoinHandle<()> {
    let mut rx = transport.subscribe_announces();

    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let dest = event.destination.lock().await;
                    let peer_hash = hex::encode(dest.desc.address_hash.as_slice());
                    drop(dest); // release lock early

                    let timestamp = now_epoch_secs_i64();
                    let app_data = event.app_data.as_slice();

                    match discovery.accept_announce(peer_hash.clone(), timestamp, app_data) {
                        Ok(record) => {
                            eprintln!(
                                "[worker] announce from {} (name={:?}, seen={})",
                                peer_hash, record.name, record.seen_count
                            );
                            events.emit_device_update(&peer_hash);
                        }
                        Err(e) => {
                            eprintln!(
                                "[worker] announce processing error for {}: {e}",
                                peer_hash
                            );
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
