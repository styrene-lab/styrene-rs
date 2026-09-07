//! Session-scoped deferred dispatch. Store only IDs, never another copy of the payload.
use super::*;
use std::collections::HashMap;
use tokio::time::{Duration, Instant};

pub(super) const CAPACITY: usize = 64;
const RETENTION: Duration = Duration::from_secs(30);

pub(super) struct HeldMessage {
    id: String,
    source: rns_core::hash::AddressHash,
    deadline: Instant,
}

pub(super) fn hold(
    tx: &tokio::sync::mpsc::Sender<HeldMessage>,
    messaging: &MessagingService,
    record: &crate::storage::messages::MessageRecord,
) {
    // Invalid signatures and stamps do not become retry candidates.
    let Ok(Some(canonical)) = messaging.canonical_inbound(&record.id) else {
        return;
    };
    if canonical.authentication_state != "unknown_identity" {
        return;
    }
    if tx
        .try_send(HeldMessage {
            id: record.id.clone(),
            source: rns_core::hash::AddressHash::new(canonical.source),
            deadline: Instant::now() + RETENTION,
        })
        .is_err()
    {
        crate::daemon_diagnostic!("[worker] deferred inbound capacity reached: {}", record.id);
    }
}

pub(super) fn spawn(
    mut rx: tokio::sync::mpsc::Receiver<HeldMessage>,
    transport: Arc<dyn MeshTransport>,
    messaging: Arc<MessagingService>,
    protocol: Arc<ProtocolService>,
    auto_reply: Option<Arc<AutoReplyService>>,
    response_tx: tokio::sync::mpsc::Sender<ResponseRequest>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut pending = HashMap::<String, HeldMessage>::new();
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                item = rx.recv() => {
                    let Some(item) = item else { break; };
                    if pending.len() < CAPACITY {
                        pending.entry(item.id.clone()).or_insert(item);
                    } else {
                        crate::daemon_diagnostic!("[worker] deferred inbound capacity reached: {}", item.id);
                    }
                }
                _ = tick.tick() => {
                    let ids: Vec<_> = pending.keys().cloned().collect();
                    for id in ids {
                        let Some(item) = pending.get(&id) else { continue; };
                        if Instant::now() >= item.deadline {
                            pending.remove(&id);
                            crate::daemon_diagnostic!("[worker] deferred inbound identity timed out: {id}");
                            continue;
                        }
                        let Ok(Some(identity)) = tokio::time::timeout(
                            Duration::from_millis(100), transport.resolve_identity(&item.source),
                        ).await else { continue; };
                        if Instant::now() >= item.deadline {
                            pending.remove(&id);
                            continue;
                        }
                        // Remove before any dispatch. Duplicates never enter this queue:
                        // only the winning canonical insert is eligible to call hold().
                        pending.remove(&id);
                        match messaging.revalidate_unknown_message(&id, &identity) {
                            Ok(_) if messaging.inbound_is_dispatchable(&id).unwrap_or(false) => {
                                if let Ok(Some(record)) = messaging.get_message(&id) {
                                    crate::daemon_diagnostic!("[worker] deferred inbound verified, dispatching: {id}");
                                    protocol.dispatch_inbound(&record).await;
                                    maybe_enqueue_response(&record, auto_reply.as_ref(), &response_tx);
                                }
                            }
                            Ok(_) => crate::daemon_diagnostic!("[worker] deferred inbound remains untrusted: {id}"),
                            Err(error) => crate::daemon_diagnostic!("[worker] deferred inbound verification failed: {id}: {error}"),
                        }
                    }
                }
            }
        }
    })
}
