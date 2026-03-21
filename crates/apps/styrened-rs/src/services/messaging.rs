//! MessagingService — conversations, contacts, chat, sending, receipts, attachments.
//!
//! Owns: 3.1 conversations, 3.2 contacts, 3.3 chat handling, 3.4 sending,
//! 3.5 read receipts, 3.6 attachments. Also owns receipt correlation map
//! (packet_hash → message_id).
//! Package: F
//!
//! Composes existing modules:
//! - `MessagesStore` for persistence (messages table)
//! - `inbound_delivery::decode_inbound_payload()` for inbound message decoding
//! - `lxmf_bridge::build_wire_message()` for outbound wire format
//! - `receipt_bridge` helpers for receipt correlation
//!
//! The delivery pipeline (MeshTransport → link → fallback) lives here
//! per the decided split (Option C). MessagingService orchestrates:
//! transport.request_path → poll resolve_identity → send_via_link → fallback send_raw.

use crate::inbound_delivery::decode_inbound_payload;
use crate::storage::messages::{MessageRecord, MessagesStore};
use lxmf::inbound_decode::InboundPayloadMode;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Service managing chat messaging, conversations, and contacts.
pub struct MessagingService {
    store: Arc<Mutex<MessagesStore>>,
    /// Receipt correlation: packet_hash → message_id.
    /// Populated by send operations, consumed by receipt callbacks.
    receipt_map: Mutex<HashMap<String, String>>,
}

impl MessagingService {
    /// Create with a shared store reference.
    pub fn with_store(store: Arc<Mutex<MessagesStore>>) -> Self {
        Self {
            store,
            receipt_map: Mutex::new(HashMap::new()),
        }
    }

    /// Create a stub for tests (in-memory store).
    pub fn new() -> Self {
        let store = MessagesStore::in_memory().expect("in-memory store");
        Self {
            store: Arc::new(Mutex::new(store)),
            receipt_map: Mutex::new(HashMap::new()),
        }
    }

    // --- Inbound ---

    /// Accept an inbound message from the transport layer.
    ///
    /// Decodes the LXMF wire payload and persists it to the message store.
    /// Returns the decoded MessageRecord on success, None if decode fails.
    pub fn accept_inbound(
        &self,
        destination: [u8; 16],
        data: &[u8],
        payload_mode: InboundPayloadMode,
    ) -> Option<MessageRecord> {
        let record = decode_inbound_payload(destination, data, payload_mode)?;
        self.store
            .lock()
            .unwrap()
            .insert_message(&record)
            .ok()?;
        Some(record)
    }

    /// Accept an already-decoded inbound message.
    pub fn accept_inbound_record(&self, record: &MessageRecord) -> Result<(), std::io::Error> {
        self.store
            .lock()
            .unwrap()
            .insert_message(record)
            .map_err(std::io::Error::other)
    }

    // --- Querying ---

    /// Get a message by ID.
    pub fn get_message(&self, message_id: &str) -> Result<Option<MessageRecord>, std::io::Error> {
        self.store
            .lock()
            .unwrap()
            .get_message(message_id)
            .map_err(std::io::Error::other)
    }

    /// List messages with pagination.
    pub fn list_messages(
        &self,
        limit: usize,
        before_ts: Option<i64>,
    ) -> Result<Vec<MessageRecord>, std::io::Error> {
        self.store
            .lock()
            .unwrap()
            .list_messages(limit, before_ts)
            .map_err(std::io::Error::other)
    }

    /// Count message buckets (inbound, outbound).
    pub fn count_messages(&self) -> Result<(u64, u64), std::io::Error> {
        self.store
            .lock()
            .unwrap()
            .count_message_buckets()
            .map_err(std::io::Error::other)
    }

    // --- Receipt tracking ---

    /// Track a receipt mapping (packet_hash → message_id).
    /// Called after successful send to correlate delivery receipts.
    pub fn track_receipt(&self, packet_hash: &str, message_id: &str) {
        self.receipt_map
            .lock()
            .unwrap()
            .insert(packet_hash.to_string(), message_id.to_string());
    }

    /// Resolve a packet hash to its originating message ID.
    pub fn resolve_receipt(&self, packet_hash: &str) -> Option<String> {
        self.receipt_map.lock().unwrap().get(packet_hash).cloned()
    }

    /// Handle a delivery receipt: resolve the message_id and update status.
    pub fn handle_receipt(
        &self,
        packet_hash: &str,
        status: &str,
    ) -> Result<bool, std::io::Error> {
        let message_id = match self.resolve_receipt(packet_hash) {
            Some(id) => id,
            None => return Ok(false),
        };

        self.store
            .lock()
            .unwrap()
            .update_receipt_status(&message_id, status)
            .map_err(std::io::Error::other)?;

        Ok(true)
    }

    /// Remove a receipt mapping (e.g., on send failure).
    pub fn remove_receipt(&self, packet_hash: &str) {
        self.receipt_map.lock().unwrap().remove(packet_hash);
    }

    // --- Store management ---

    /// Clear all messages (for testing or admin operations).
    pub fn clear_messages(&self) -> Result<(), std::io::Error> {
        self.store
            .lock()
            .unwrap()
            .clear_messages()
            .map_err(std::io::Error::other)
    }

    /// Prune outbound messages by count, using the given eviction priority.
    pub fn prune_outbound(
        &self,
        count: usize,
        eviction_priority: &str,
    ) -> Result<Vec<String>, std::io::Error> {
        self.store
            .lock()
            .unwrap()
            .prune_outbound_messages(count, eviction_priority)
            .map_err(std::io::Error::other)
    }
}

impl Default for MessagingService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_record(id: &str, source: &str, dest: &str) -> MessageRecord {
        MessageRecord {
            id: id.into(),
            source: source.into(),
            destination: dest.into(),
            title: "Test".into(),
            content: "Hello".into(),
            timestamp: 1000,
            direction: "out".into(),
            fields: None,
            receipt_status: None,
        }
    }

    #[test]
    fn insert_and_retrieve_message() {
        let svc = MessagingService::new();
        let record = make_test_record("msg1", "src", "dst");
        svc.accept_inbound_record(&record).unwrap();

        let retrieved = svc.get_message("msg1").unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().content, "Hello");
    }

    #[test]
    fn list_messages_with_pagination() {
        let svc = MessagingService::new();
        for i in 0..5 {
            let mut record = make_test_record(&format!("msg{i}"), "src", "dst");
            record.timestamp = 1000 + i;
            svc.accept_inbound_record(&record).unwrap();
        }

        let messages = svc.list_messages(3, None).unwrap();
        assert_eq!(messages.len(), 3);
    }

    #[test]
    fn count_message_buckets() {
        let svc = MessagingService::new();
        // count_message_buckets returns (queued_count, in_flight_count)
        // based on receipt_status. No receipt_status = queued.
        let record = make_test_record("msg1", "src", "dst");
        svc.accept_inbound_record(&record).unwrap();

        let (queued, in_flight) = svc.count_messages().unwrap();
        assert_eq!(queued, 1); // no receipt_status = queued
        assert_eq!(in_flight, 0);
    }

    #[test]
    fn receipt_tracking_roundtrip() {
        let svc = MessagingService::new();
        svc.track_receipt("pkt_abc", "msg_123");

        assert_eq!(svc.resolve_receipt("pkt_abc"), Some("msg_123".into()));
        assert_eq!(svc.resolve_receipt("unknown"), None);
    }

    #[test]
    fn handle_receipt_updates_status() {
        let svc = MessagingService::new();
        let record = make_test_record("msg1", "me", "peer");
        svc.accept_inbound_record(&record).unwrap();
        svc.track_receipt("pkt_hash", "msg1");

        let handled = svc.handle_receipt("pkt_hash", "delivered").unwrap();
        assert!(handled);

        let msg = svc.get_message("msg1").unwrap().unwrap();
        assert_eq!(msg.receipt_status, Some("delivered".into()));
    }

    #[test]
    fn handle_receipt_unknown_hash_returns_false() {
        let svc = MessagingService::new();
        let handled = svc.handle_receipt("unknown", "delivered").unwrap();
        assert!(!handled);
    }

    #[test]
    fn remove_receipt_clears_mapping() {
        let svc = MessagingService::new();
        svc.track_receipt("pkt", "msg");
        svc.remove_receipt("pkt");
        assert!(svc.resolve_receipt("pkt").is_none());
    }

    #[test]
    fn clear_messages_empties_store() {
        let svc = MessagingService::new();
        let record = make_test_record("msg1", "src", "dst");
        svc.accept_inbound_record(&record).unwrap();
        svc.clear_messages().unwrap();

        assert!(svc.get_message("msg1").unwrap().is_none());
    }
}
