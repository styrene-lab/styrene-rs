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
use styrene_ipc::types::{
    DaemonEvent, DeviceInfo, MessageEventKind, MessageInfo,
};
use tokio::sync::broadcast;

/// Default capacity for the event broadcast channel.
const DEFAULT_CHANNEL_CAPACITY: usize = 256;

/// Default capacity for the activity ring (backfill buffer).
const DEFAULT_RING_CAPACITY: usize = 200;

/// Service managing event publication and subscription.
pub struct EventService {
    /// Broadcast sender for live event streaming (internal RpcEvent).
    tx: broadcast::Sender<RpcEvent>,
    /// Broadcast sender for typed DaemonEvent (for IPC consumers).
    daemon_tx: broadcast::Sender<DaemonEvent>,
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
        let (daemon_tx, _) = broadcast::channel(channel_capacity);
        Self {
            tx,
            daemon_tx,
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

    /// Subscribe to typed DaemonEvents (for IPC consumers).
    pub fn subscribe_daemon_events(&self) -> broadcast::Receiver<DaemonEvent> {
        self.daemon_tx.subscribe()
    }

    /// Subscribe to message events only.
    pub fn subscribe_messages(&self, peer_filter: &[String]) -> broadcast::Receiver<DaemonEvent> {
        // For now, return unfiltered. Filtering can be added in the connection layer.
        let _ = peer_filter; // TODO: per-peer filtering
        self.daemon_tx.subscribe()
    }

    /// Subscribe to device events only.
    pub fn subscribe_devices(&self) -> broadcast::Receiver<DaemonEvent> {
        // Returns all DaemonEvents — connection layer filters to Device variants.
        self.daemon_tx.subscribe()
    }

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
        // Also emit typed DaemonEvent
        let mut msg = MessageInfo::default();
        msg.id = record.id.clone();
        msg.source_hash = record.source.clone();
        msg.content = record.content.clone();
        msg.timestamp = record.timestamp;
        let _ = self.daemon_tx.send(DaemonEvent::Message {
            kind: MessageEventKind::New,
            message: msg,
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
        let kind = match status {
            "delivered" => MessageEventKind::Delivered,
            s if s.starts_with("failed") => MessageEventKind::Failed,
            _ => MessageEventKind::StatusChanged,
        };
        let mut msg = MessageInfo::default();
        msg.id = message_id.to_string();
        let _ = self.daemon_tx.send(DaemonEvent::Message {
            kind,
            message: msg,
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
        let mut dev = DeviceInfo::default();
        dev.destination_hash = peer_hash.to_string();
        let _ = self.daemon_tx.send(DaemonEvent::Device { device: dev });
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

    #[tokio::test]
    async fn emit_message_new_sends_daemon_event() {
        let svc = EventService::new();
        let mut rx = svc.subscribe_daemon_events();

        let record = crate::storage::messages::MessageRecord {
            id: "msg1".into(),
            source: "src_hash".into(),
            destination: "dst_hash".into(),
            title: String::new(),
            content: "hello".into(),
            timestamp: 1000,
            direction: "in".into(),
            fields: None,
            receipt_status: None,
            read: false,
        };
        svc.emit_message_new(&record);

        let event = rx.recv().await.unwrap();
        match event {
            DaemonEvent::Message { kind, message } => {
                assert_eq!(kind, MessageEventKind::New);
                assert_eq!(message.id, "msg1");
                assert_eq!(message.content, "hello");
            }
            _ => panic!("expected Message event"),
        }
    }

    #[tokio::test]
    async fn emit_device_update_sends_daemon_event() {
        let svc = EventService::new();
        let mut rx = svc.subscribe_devices();

        svc.emit_device_update("abcdef01");

        let event = rx.recv().await.unwrap();
        match event {
            DaemonEvent::Device { device } => {
                assert_eq!(device.destination_hash, "abcdef01");
            }
            _ => panic!("expected Device event"),
        }
    }

    #[test]
    fn no_subscribers_doesnt_panic() {
        let svc = EventService::new();
        // publish with zero subscribers should not panic
        svc.publish(make_event("orphan"));
        assert_eq!(svc.ring_len(), 1);
    }
}
