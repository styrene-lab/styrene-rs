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
use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use styrene_ipc::types::{
    DaemonEvent, DeviceInfo, LinkActivity, LinkEvent, LinkEventKind, LinkSnapshot,
    MessageEventKind, MessageInfo, ObservationMetadata, ObservationSource, ResourceDirection,
    ResourceTransferInfo, ResourceTransferState, RouteEventInfo, RouteEventKind,
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
    active_links: Mutex<BTreeMap<String, LinkEvent>>,
    link_history: Mutex<VecDeque<LinkEvent>>,
    link_history_capacity: usize,
    resources: Mutex<BTreeMap<String, ResourceTransferInfo>>,
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
            active_links: Mutex::new(BTreeMap::new()),
            link_history: Mutex::new(VecDeque::with_capacity(ring_capacity)),
            link_history_capacity: ring_capacity,
            resources: Mutex::new(BTreeMap::new()),
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

    pub fn emit_messaging_operation(&self, outcome: styrene_ipc::types::MessagingOperationOutcome) {
        let _ = self.daemon_tx.send(DaemonEvent::MessagingOperation { outcome: Box::new(outcome) });
    }

    pub fn emit_standard_propagation_changed(&self, observed_at: i64) {
        let _ = self.daemon_tx.send(DaemonEvent::StandardPropagationChanged { observed_at });
    }

    /// Emit a new inbound message event.
    pub fn emit_message_new(
        &self,
        record: &MessageRecord,
        canonical: Option<&crate::storage::messages::CanonicalInboundRecord>,
    ) {
        self.publish(RpcEvent {
            event_type: "message_received".into(),
            payload: serde_json::json!({
                "id": record.id,
                "source": record.source,
                "destination": record.destination,
                "content": record.content,
                "timestamp": record.timestamp,
                "lxmf_timestamp": canonical.map(|value| value.timestamp),
                "kind": "new",
            }),
        });
        // Also emit typed DaemonEvent
        let mut msg = MessageInfo::default();
        msg.id = record.id.clone();
        msg.source_hash = record.source.clone();
        msg.destination_hash = record.destination.clone();
        msg.content = record.content.clone();
        msg.timestamp = record.timestamp;
        if let Some(canonical) = canonical {
            msg.lxmf_timestamp = Some(canonical.timestamp);
            msg.authentication_state = match canonical.authentication_state.as_str() {
                "verified" => styrene_ipc::types::MessageAuthenticationState::Verified,
                "invalid" => styrene_ipc::types::MessageAuthenticationState::Invalid,
                "unknown_identity" => {
                    styrene_ipc::types::MessageAuthenticationState::UnknownIdentity
                }
                "not_applicable" => styrene_ipc::types::MessageAuthenticationState::NotApplicable,
                _ => styrene_ipc::types::MessageAuthenticationState::Unknown,
            };
            msg.stamp_state = match canonical.stamp_state.as_str() {
                "verified" => styrene_ipc::types::MessageStampState::Verified,
                "invalid" => styrene_ipc::types::MessageStampState::Invalid,
                "not_applicable" => styrene_ipc::types::MessageStampState::NotApplicable,
                _ => styrene_ipc::types::MessageStampState::Unknown,
            };
            msg.stamp_value = canonical.stamp_value;
            msg.stamp_cost = canonical.stamp_target;
        }
        let _ =
            self.daemon_tx.send(DaemonEvent::Message { kind: MessageEventKind::New, message: msg });
    }

    pub fn emit_message_authentication_changed(
        &self,
        record: &MessageRecord,
        canonical: &crate::storage::messages::CanonicalInboundRecord,
    ) {
        self.publish(RpcEvent {
            event_type: "message_authentication_changed".into(),
            payload: serde_json::json!({
                "id": record.id,
                "authentication_state": canonical.authentication_state,
                "stamp_state": canonical.stamp_state,
                "stamp_value": canonical.stamp_value,
            }),
        });
        let mut message = MessageInfo::default();
        message.id = record.id.clone();
        message.source_hash = record.source.clone();
        message.destination_hash = record.destination.clone();
        message.content = record.content.clone();
        message.timestamp = record.timestamp;
        message.lxmf_timestamp = Some(canonical.timestamp);
        message.authentication_state = match canonical.authentication_state.as_str() {
            "verified" => styrene_ipc::types::MessageAuthenticationState::Verified,
            "invalid" => styrene_ipc::types::MessageAuthenticationState::Invalid,
            "unknown_identity" => styrene_ipc::types::MessageAuthenticationState::UnknownIdentity,
            "not_applicable" => styrene_ipc::types::MessageAuthenticationState::NotApplicable,
            _ => styrene_ipc::types::MessageAuthenticationState::Unknown,
        };
        message.stamp_state = match canonical.stamp_state.as_str() {
            "verified" => styrene_ipc::types::MessageStampState::Verified,
            "invalid" => styrene_ipc::types::MessageStampState::Invalid,
            "not_applicable" => styrene_ipc::types::MessageStampState::NotApplicable,
            _ => styrene_ipc::types::MessageStampState::Unknown,
        };
        message.stamp_value = canonical.stamp_value;
        message.stamp_cost = canonical.stamp_target;
        let _ = self
            .daemon_tx
            .send(DaemonEvent::Message { kind: MessageEventKind::StatusChanged, message });
    }

    pub fn emit_reconciliation_required(&self, reason: &str) {
        self.publish(RpcEvent {
            event_type: "reconciliation_required".into(),
            payload: serde_json::json!({ "dropped": 1, "reason": reason }),
        });
        let _ = self.daemon_tx.send(DaemonEvent::ReconcileRequired { dropped: 1 });
    }

    /// Emit a structured inbound drop outcome for observability. Drop events
    /// intentionally remain internal `RpcEvent`s until the typed IPC schema
    /// gains a compatible variant.
    pub fn emit_inbound_drop(
        &self,
        path: &str,
        reason: &str,
        message_id: Option<&str>,
        destination: Option<&str>,
        detail: Option<&str>,
    ) {
        self.publish(RpcEvent {
            event_type: "inbound_dropped".into(),
            payload: serde_json::json!({
                "path": path,
                "reason": reason,
                "message_id": message_id,
                "destination": destination,
                "detail": detail,
            }),
        });
    }

    /// Emit a message status change event.
    pub fn emit_message_status(
        &self,
        message_id: &str,
        status: &str,
        lifecycle_state: styrene_ipc::types::MessageLifecycleState,
        terminal_detail: Option<&str>,
        kind: MessageEventKind,
    ) {
        self.publish(RpcEvent {
            event_type: "message_status".into(),
            payload: serde_json::json!({
                "id": message_id,
                "status": status,
                "kind": "status_changed",
            }),
        });
        let mut msg = MessageInfo::default();
        msg.id = message_id.to_string();
        msg.status = status.to_string();
        msg.lifecycle_state = lifecycle_state;
        msg.terminal_detail = terminal_detail.map(str::to_owned);
        let _ = self.daemon_tx.send(DaemonEvent::Message { kind, message: msg });
    }

    pub fn emit_message_inspection_changed(&self, message_id: &str) {
        let mut message = MessageInfo::default();
        message.id = message_id.to_string();
        let _ = self
            .daemon_tx
            .send(DaemonEvent::Message { kind: MessageEventKind::StatusChanged, message });
    }

    /// Emit a link lifecycle event (activated, closed, RTT updated).
    pub fn emit_link_event(&self, mut event: LinkEvent) {
        let terminal = matches!(event.kind, LinkEventKind::Teardown | LinkEventKind::Timeout);
        if terminal {
            self.active_links.lock().unwrap().remove(&event.link_id);
            event.activity = LinkActivity::Historical;
        } else {
            let mut active = self.active_links.lock().unwrap();
            if let Some(previous) = active.get(&event.link_id) {
                event.identified |= previous.identified;
                event.interface = event.interface.or_else(|| previous.interface.clone());
                event.rtt_ms = event.rtt_ms.or(previous.rtt_ms);
            }
            event.activity = LinkActivity::Active;
            active.insert(event.link_id.clone(), event.clone());
        }
        {
            let mut history = self.link_history.lock().unwrap();
            if self.link_history_capacity == 0 {
                history.clear();
            } else if history.len() >= self.link_history_capacity {
                history.pop_front();
            }
            if self.link_history_capacity > 0 {
                let mut historical = event.clone();
                historical.activity = LinkActivity::Historical;
                history.push_back(historical);
            }
        }
        self.publish(RpcEvent {
            event_type: match event.status.as_str() {
                "active" => "link_activated".into(),
                "closed" => "link_closed".into(),
                "rtt_updated" => "link_rtt_updated".into(),
                other => format!("link_{other}"),
            },
            payload: serde_json::json!({
                "link_id": event.link_id,
                "peer_hash": event.peer_hash,
                "status": event.status,
                "rtt_ms": event.rtt_ms,
            }),
        });
        let _ = self.daemon_tx.send(DaemonEvent::Link { event });
    }

    pub fn link_snapshot(&self) -> LinkSnapshot {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or(0);
        let refresh = |mut event: LinkEvent| {
            if let (Some(observed_at), Some(threshold)) =
                (event.observation.observed_at, event.observation.freshness_threshold_secs)
            {
                event.observation.age_secs = Some(now.saturating_sub(observed_at).max(0) as u64);
                event.observation.stale =
                    event.observation.age_secs.is_some_and(|age| age > threshold);
            }
            event
        };
        let mut snapshot = LinkSnapshot::default();
        snapshot.active =
            self.active_links.lock().unwrap().values().cloned().map(refresh).collect();
        snapshot.history = self.link_history.lock().unwrap().iter().cloned().map(refresh).collect();
        snapshot
    }

    /// Reconcile event-derived state with the authoritative transport projection.
    pub fn reconcile_links(&self, active: Vec<LinkEvent>, terminal: Vec<LinkEvent>) {
        let mut links = self.active_links.lock().unwrap();
        links.clear();
        links.extend(active.into_iter().map(|event| (event.link_id.clone(), event)));
        drop(links);

        let mut history = self.link_history.lock().unwrap();
        for mut event in terminal {
            event.activity = LinkActivity::Historical;
            history.retain(|existing| {
                existing.link_id != event.link_id
                    || !matches!(existing.kind, LinkEventKind::Teardown | LinkEventKind::Timeout)
            });
            if self.link_history_capacity == 0 {
                continue;
            }
            if history.len() >= self.link_history_capacity {
                history.pop_front();
            }
            history.push_back(event);
        }
    }

    /// Subscribe to link events only (backed by the same daemon_tx channel).
    pub fn subscribe_links(&self) -> broadcast::Receiver<DaemonEvent> {
        self.daemon_tx.subscribe()
    }

    pub fn emit_route_event(&self, event: RouteEventInfo) {
        let kind = match event.kind {
            RouteEventKind::Discovered => "discovered",
            RouteEventKind::Lost => "lost",
            RouteEventKind::Rediscovered => "rediscovered",
            _ => "unknown",
        };
        self.publish(RpcEvent {
            event_type: format!("route_{kind}"),
            payload: serde_json::json!({
                "destination_hash": event.route.destination_hash,
                "kind": kind,
                "loss_reason": event.loss_reason,
                "observed_at": event.observation.observed_at,
            }),
        });
        let _ = self.daemon_tx.send(DaemonEvent::Route { event });
    }

    pub fn subscribe_routes(&self) -> broadcast::Receiver<DaemonEvent> {
        self.daemon_tx.subscribe()
    }

    pub fn emit_network_operation(&self, operation: styrene_ipc::types::NetworkOperationInfo) {
        let _ = self.daemon_tx.send(DaemonEvent::NetworkOperation { operation });
    }

    pub fn subscribe_network_operations(&self) -> broadcast::Receiver<DaemonEvent> {
        self.daemon_tx.subscribe()
    }

    pub fn emit_resource_event(&self, event: &rns_core::transport::resource::ResourceEvent) {
        use rns_core::transport::resource::{ResourceEventKind, ResourceFailure};

        let resource_hash = hex::encode(event.hash.to_bytes());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or(0);
        let mut resources = self.resources.lock().unwrap();
        let transfer = resources.entry(resource_hash.clone()).or_insert_with(|| {
            let mut transfer = ResourceTransferInfo::default();
            transfer.resource_hash = resource_hash;
            transfer.link_id = event.link_id.to_string();
            transfer.observation = ObservationMetadata::at(
                ObservationSource::TransportResourceState,
                Some(now),
                now,
                300,
            );
            transfer
        });
        transfer.observation.observed_at = Some(now);
        transfer.observation.age_secs = Some(0);
        transfer.observation.stale = false;
        match &event.kind {
            ResourceEventKind::Progress(progress) => {
                if transfer.direction == ResourceDirection::Unknown {
                    transfer.direction = ResourceDirection::Inbound;
                }
                transfer.state = ResourceTransferState::Transferring;
                transfer.received_bytes = progress.received_bytes;
                transfer.total_bytes = progress.total_bytes;
                transfer.received_parts = progress.received_parts.try_into().unwrap_or(u64::MAX);
                transfer.total_parts = progress.total_parts.try_into().unwrap_or(u64::MAX);
                transfer.progress = if progress.total_bytes == 0 {
                    0.0
                } else {
                    progress.received_bytes as f32 / progress.total_bytes as f32
                };
                transfer.cancellable = true;
            }
            ResourceEventKind::Complete(complete) => {
                transfer.direction = ResourceDirection::Inbound;
                transfer.state = ResourceTransferState::Completed;
                transfer.received_bytes = complete.transfer_size;
                transfer.total_bytes = complete.transfer_size;
                transfer.progress = 1.0;
                transfer.cancellable = false;
            }
            ResourceEventKind::OutboundComplete => {
                transfer.direction = ResourceDirection::Outbound;
                transfer.state = ResourceTransferState::Completed;
                transfer.received_bytes = transfer.total_bytes;
                transfer.progress = 1.0;
                transfer.cancellable = false;
            }
            ResourceEventKind::Failed(failure) => {
                transfer.state = match failure {
                    ResourceFailure::Cancelled => ResourceTransferState::Cancelled,
                    ResourceFailure::TimedOut => ResourceTransferState::TimedOut,
                    ResourceFailure::LinkClosed => ResourceTransferState::LinkClosed,
                    ResourceFailure::Integrity => ResourceTransferState::IntegrityFailed,
                };
                transfer.cancellable = false;
            }
        }
        let transfer = transfer.clone();
        drop(resources);
        let _ = self.daemon_tx.send(DaemonEvent::Resource { transfer });
    }

    pub fn emit_attachment_transfer(&self, transfer: styrene_ipc::types::AttachmentTransferInfo) {
        let _ = self.daemon_tx.send(DaemonEvent::AttachmentTransfer { transfer });
    }

    pub fn resource_transfers(&self) -> Vec<ResourceTransferInfo> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or(0);
        self.resources
            .lock()
            .unwrap()
            .values()
            .cloned()
            .map(|mut transfer| {
                if let Some(observed_at) = transfer.observation.observed_at {
                    let age = now.saturating_sub(observed_at).max(0) as u64;
                    transfer.observation.age_secs = Some(age);
                    transfer.observation.stale = transfer
                        .observation
                        .freshness_threshold_secs
                        .is_some_and(|threshold| age > threshold);
                }
                transfer
            })
            .collect()
    }

    pub fn subscribe_resources(&self) -> broadcast::Receiver<DaemonEvent> {
        self.daemon_tx.subscribe()
    }

    /// Emit a tunnel state change event.
    pub fn emit_tunnel_state(&self, peer_hash: &str, state: &str, backend: &str) {
        self.publish(RpcEvent {
            event_type: format!("tunnel_{state}"),
            payload: serde_json::json!({
                "peer_hash": peer_hash,
                "state": state,
                "backend": backend,
            }),
        });
        let _ = self.daemon_tx.send(DaemonEvent::TunnelStateChange {
            peer_hash: peer_hash.to_string(),
            state: state.to_string(),
            backend: backend.to_string(),
        });
    }

    /// Emit a device/peer update event (announce received or status change).
    pub fn emit_device_update(&self, peer_hash: &str) {
        let mut device = DeviceInfo::default();
        device.destination_hash = peer_hash.to_string();
        self.emit_device(device);
    }

    /// Emit a projected device update, including announce-derived capabilities.
    pub fn emit_device(&self, device: DeviceInfo) {
        self.publish(RpcEvent {
            event_type: "announce_received".into(),
            payload: serde_json::json!({
                "peer_hash": device.destination_hash,
            }),
        });
        let _ = self.daemon_tx.send(DaemonEvent::Device { device });
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
        RpcEvent { event_type: event_type.into(), payload: serde_json::json!({"test": true}) }
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
        svc.emit_message_new(&record, None);

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
    async fn attachment_message_event_omits_all_canonical_byte_evidence() {
        let svc = EventService::new();
        let mut rx = svc.subscribe_daemon_events();
        let marker = vec![0xde, 0xad, 0xbe, 0xef];
        let fields = rmpv::Value::Map(vec![(
            rmpv::Value::from(5),
            rmpv::Value::Array(vec![rmpv::Value::Array(vec![
                rmpv::Value::from("marker.bin"),
                rmpv::Value::Binary(marker.clone()),
            ])]),
        )]);
        let fields_msgpack = rmp_serde::to_vec(&fields).unwrap();
        let record = MessageRecord {
            id: "attachment-event".into(),
            source: "source".into(),
            destination: "destination".into(),
            title: String::new(),
            content: "metadata only".into(),
            timestamp: 1,
            direction: "in".into(),
            fields: Some(serde_json::json!({"5": [{"name": "marker.bin", "size": 4}]})),
            receipt_status: None,
            read: false,
        };
        let canonical = crate::storage::messages::CanonicalInboundRecord {
            message_id: record.id.clone(),
            source: [1; 16],
            destination: [2; 16],
            title: marker.clone(),
            content: marker.clone(),
            timestamp: 1.0,
            fields_msgpack: Some(fields_msgpack),
            signature: Some(vec![0xde; 64]),
            stamp: Some(marker.clone()),
            wire: marker.clone(),
            authentication_state: "verified".into(),
            stamp_state: "verified".into(),
            stamp_value: Some(1),
            stamp_target: Some(1),
        };
        svc.emit_message_new(&record, Some(&canonical));
        let DaemonEvent::Message { message, .. } = rx.recv().await.unwrap() else {
            panic!("expected message event");
        };
        assert!(message.canonical_title.is_none());
        assert!(message.canonical_content.is_none());
        assert!(message.canonical_fields_msgpack.is_none());
        assert!(message.canonical_signature.is_none());
        assert!(message.canonical_stamp.is_none());
        assert!(message.canonical_wire.is_none());
        let encoded = rmp_serde::to_vec(&message).unwrap();
        assert!(!encoded.windows(marker.len()).any(|window| window == marker));
    }

    #[tokio::test]
    async fn emit_message_status_preserves_status_in_daemon_event() {
        let svc = EventService::new();
        let mut rx = svc.subscribe_daemon_events();

        svc.emit_message_status(
            "msg-receipt",
            "delivered: packet-receipt",
            styrene_ipc::types::MessageLifecycleState::Delivered,
            Some("authenticated packet receipt"),
            MessageEventKind::Delivered,
        );

        match rx.recv().await.unwrap() {
            DaemonEvent::Message { kind, message } => {
                assert_eq!(kind, MessageEventKind::Delivered);
                assert_eq!(message.id, "msg-receipt");
                assert_eq!(message.status, "delivered: packet-receipt");
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
    fn emit_inbound_drop_records_structured_outcome() {
        let svc = EventService::new();
        svc.emit_inbound_drop("direct_packet", "duplicate", Some("msg-1"), Some("dest-1"), None);

        let ring = svc.activity_ring();
        assert_eq!(ring.len(), 1);
        assert_eq!(ring[0].event_type, "inbound_dropped");
        assert_eq!(ring[0].payload["path"], "direct_packet");
        assert_eq!(ring[0].payload["reason"], "duplicate");
        assert_eq!(ring[0].payload["message_id"], "msg-1");
        assert_eq!(ring[0].payload["destination"], "dest-1");
        assert!(ring[0].payload["detail"].is_null());
    }

    #[test]
    fn no_subscribers_doesnt_panic() {
        let svc = EventService::new();
        // publish with zero subscribers should not panic
        svc.publish(make_event("orphan"));
        assert_eq!(svc.ring_len(), 1);
    }

    #[test]
    fn closed_links_leave_active_state_but_remain_in_bounded_history() {
        let svc = EventService::with_capacity(16, 2);
        let established = LinkEvent::new("link-1", "peer-1", "active", Some(4.0));
        svc.emit_link_event(established);

        let mut activity = LinkEvent::new("link-1", "peer-1", "active", Some(5.0));
        activity.kind = LinkEventKind::Activity;
        svc.emit_link_event(activity);

        let mut closed = LinkEvent::new("link-1", "peer-1", "closed", Some(5.0));
        closed.kind = LinkEventKind::Timeout;
        svc.emit_link_event(closed);

        let snapshot = svc.link_snapshot();
        assert!(snapshot.active.is_empty(), "closed links must not remain topology edges");
        assert_eq!(snapshot.history.len(), 2);
        assert_eq!(snapshot.history[0].kind, LinkEventKind::Activity);
        assert_eq!(snapshot.history[1].kind, LinkEventKind::Timeout);
        assert!(snapshot.history.iter().all(|event| event.activity == LinkActivity::Historical));
    }
}
