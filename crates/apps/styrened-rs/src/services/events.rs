//! EventService — event bus, notifications, activity ring.
//!
//! Owns: 5.1 EventBus, 5.2 notifications, 5.3 activity ring,
//! event fan-out to IPC/SSE.
//! Package: H
//!
//! Wraps the existing `broadcast::Sender<RpcEvent>` pattern from RpcDaemon
//! with a bounded activity ring for backfill on connect.

use crate::rpc::RpcEvent;
use crate::storage::messages::MessageRecord;
use std::collections::VecDeque;
use std::sync::Mutex;
use tokio::sync::broadcast;

/// Default capacity for the event broadcast channel.
const DEFAULT_CHANNEL_CAPACITY: usize = 256;

/// Default capacity for the activity ring (backfill buffer).
const DEFAULT_RING_CAPACITY: usize = 200;

/// Service managing event publication and subscription.
pub struct EventService {
    /// Broadcast sender for live event streaming.
    tx: broadcast::Sender<RpcEvent>,
    /// Activity ring — bounded deque of recent events for backfill.
    ring: Mutex<VecDeque<RpcEvent>>,
    ring_capacity: usize,
}

impl EventService {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CHANNEL_CAPACITY, DEFAULT_RING_CAPACITY)
    }

    pub fn with_capacity(channel_capacity: usize, ring_capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(channel_capacity);
        Self {
            tx,
            ring: Mutex::new(VecDeque::with_capacity(ring_capacity)),
            ring_capacity,
        }
    }

    /// Publish an event to all subscribers and append to the activity ring.
    pub fn publish(&self, event: RpcEvent) {
        // Append to ring first (always succeeds)
        {
            let mut ring = self.ring.lock().unwrap();
            if ring.len() >= self.ring_capacity {
                ring.pop_front();
            }
            ring.push_back(event.clone());
        }
        // Broadcast to live subscribers (ignore "no subscribers" error)
        let _ = self.tx.send(event);
    }

    /// Subscribe to live events.
    pub fn subscribe(&self) -> broadcast::Receiver<RpcEvent> {
        self.tx.subscribe()
    }

    /// Get the activity ring snapshot (for backfill on TUI connect).
    pub fn activity_ring(&self) -> Vec<RpcEvent> {
        self.ring.lock().unwrap().iter().cloned().collect()
    }

    /// Number of events in the activity ring.
    pub fn ring_len(&self) -> usize {
        self.ring.lock().unwrap().len()
    }

    /// Number of live subscribers.
    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }

    // --- Typed event emitters ---

    /// Emit a new inbound message event.
    pub fn emit_message_new(&self, record: &MessageRecord) {
        self.publish(RpcEvent {
            event_type: "message_received".into(),
            payload: serde_json::json!({
                "id": record.id,
                "source": record.source,
                "destination": record.destination,
                "content": record.content,
                "timestamp": record.timestamp,
                "kind": "new",
            }),
        });
    }

    /// Emit a message status change event.
    pub fn emit_message_status(&self, message_id: &str, status: &str) {
        self.publish(RpcEvent {
            event_type: "message_status".into(),
            payload: serde_json::json!({
                "id": message_id,
                "status": status,
                "kind": "status_changed",
            }),
        });
    }

    /// Emit a device/peer update event (announce received or status change).
    pub fn emit_device_update(&self, peer_hash: &str) {
        self.publish(RpcEvent {
            event_type: "announce_received".into(),
            payload: serde_json::json!({
                "peer_hash": peer_hash,
            }),
        });
    }
}

impl Default for EventService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_event(event_type: &str) -> RpcEvent {
        RpcEvent {
            event_type: event_type.into(),
            payload: serde_json::json!({"test": true}),
        }
    }

    #[test]
    fn publish_adds_to_ring() {
        let svc = EventService::new();
        svc.publish(make_event("announce_received"));
        svc.publish(make_event("inbound"));
        assert_eq!(svc.ring_len(), 2);
    }

    #[tokio::test]
    async fn publish_reaches_subscriber() {
        let svc = EventService::new();
        let mut rx = svc.subscribe();
        svc.publish(make_event("test_event"));

        let event = rx.recv().await.unwrap();
        assert_eq!(event.event_type, "test_event");
    }

    #[test]
    fn activity_ring_returns_snapshot() {
        let svc = EventService::new();
        svc.publish(make_event("a"));
        svc.publish(make_event("b"));
        svc.publish(make_event("c"));

        let ring = svc.activity_ring();
        assert_eq!(ring.len(), 3);
        assert_eq!(ring[0].event_type, "a");
        assert_eq!(ring[2].event_type, "c");
    }

    #[test]
    fn ring_evicts_oldest_when_full() {
        let svc = EventService::with_capacity(16, 3);
        svc.publish(make_event("a"));
        svc.publish(make_event("b"));
        svc.publish(make_event("c"));
        svc.publish(make_event("d")); // evicts "a"

        let ring = svc.activity_ring();
        assert_eq!(ring.len(), 3);
        assert_eq!(ring[0].event_type, "b");
        assert_eq!(ring[2].event_type, "d");
    }

    #[tokio::test]
    async fn multiple_subscribers_receive_same_event() {
        let svc = EventService::new();
        let mut rx1 = svc.subscribe();
        let mut rx2 = svc.subscribe();
        assert_eq!(svc.subscriber_count(), 2);

        svc.publish(make_event("fanout"));

        let e1 = rx1.recv().await.unwrap();
        let e2 = rx2.recv().await.unwrap();
        assert_eq!(e1.event_type, "fanout");
        assert_eq!(e2.event_type, "fanout");
    }

    #[test]
    fn no_subscribers_doesnt_panic() {
        let svc = EventService::new();
        // publish with zero subscribers should not panic
        svc.publish(make_event("orphan"));
        assert_eq!(svc.ring_len(), 1);
    }
}
