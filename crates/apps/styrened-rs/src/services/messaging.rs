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
use crate::lxmf_bridge;
use crate::storage::messages::{MessageRecord, MessagesStore};
use crate::transport::mesh_transport::{MeshTransport, TransportError};
use lxmf::inbound_decode::InboundPayloadMode;
use rns_core::destination::{DestinationDesc, DestinationName};
use rns_core::hash::AddressHash;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Service managing chat messaging, conversations, and contacts.
pub struct MessagingService {
    store: Arc<Mutex<MessagesStore>>,
    /// Receipt correlation: packet_hash → message_id.
    /// Populated by send operations, consumed by receipt callbacks.
    receipt_map: Mutex<HashMap<String, String>>,
    /// Transport for outbound delivery (None in test mode).
    transport: Option<Arc<dyn MeshTransport>>,
    /// Signing key for LXMF wire messages (None in test mode).
    signer: Option<Arc<rns_core::identity::PrivateIdentity>>,
}

impl MessagingService {
    /// Create with a shared store reference.
    pub fn with_store(store: Arc<Mutex<MessagesStore>>) -> Self {
        Self {
            store,
            receipt_map: Mutex::new(HashMap::new()),
            transport: None,
            signer: None,
        }
    }

    /// Create with full delivery pipeline support.
    pub fn with_transport(
        store: Arc<Mutex<MessagesStore>>,
        transport: Arc<dyn MeshTransport>,
        signer: Arc<rns_core::identity::PrivateIdentity>,
    ) -> Self {
        Self {
            store,
            receipt_map: Mutex::new(HashMap::new()),
            transport: Some(transport),
            signer: Some(signer),
        }
    }

    /// Create a stub for tests (in-memory store).
    pub fn new() -> Self {
        let store = MessagesStore::in_memory().expect("in-memory store");
        Self {
            store: Arc::new(Mutex::new(store)),
            receipt_map: Mutex::new(HashMap::new()),
            transport: None,
            signer: None,
        }
    }

    // --- Outbound delivery ---

    /// Send a chat message via the delivery pipeline.
    ///
    /// Pipeline: build LXMF wire → persist → request_path → poll identity →
    /// send_via_link → track receipt. Returns the message ID on successful queue.
    pub async fn send_chat(
        &self,
        peer_hash: &str,
        content: &str,
        title: Option<&str>,
    ) -> Result<String, std::io::Error> {
        let transport = self.transport.as_ref().ok_or_else(|| {
            std::io::Error::other("transport not available — cannot send")
        })?;
        let signer = self.signer.as_ref().ok_or_else(|| {
            std::io::Error::other("signer not available — cannot send")
        })?;

        if !transport.is_connected() {
            return Err(std::io::Error::other("transport not connected"));
        }

        // Parse destination hash
        let dest_bytes: [u8; 16] = hex::decode(peer_hash)
            .map_err(|e| std::io::Error::other(format!("invalid peer hash: {e}")))?
            .try_into()
            .map_err(|_| std::io::Error::other("peer hash must be 16 bytes"))?;
        let dest_hash = AddressHash::new(dest_bytes);

        // Build LXMF wire message
        let source_hash = transport.identity_hash();
        let mut source_bytes = [0u8; 16];
        source_bytes.copy_from_slice(source_hash.as_slice());
        let payload = lxmf_bridge::build_wire_message(
            source_bytes,
            dest_bytes,
            title.unwrap_or(""),
            content,
            None,
            signer,
        )
        .map_err(|e| std::io::Error::other(format!("wire encode: {e}")))?;

        // Generate message ID and persist
        let msg_id = hex::encode(&payload[..8]); // First 8 bytes of wire as ID
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let record = MessageRecord {
            id: msg_id.clone(),
            source: hex::encode(source_hash.as_slice()),
            destination: peer_hash.to_string(),
            title: title.unwrap_or("").to_string(),
            content: content.to_string(),
            timestamp: now,
            direction: "out".to_string(),
            fields: None,
            receipt_status: Some("sending".to_string()),
            read: true, // Outgoing messages are always "read"
        };
        self.store
            .lock()
            .unwrap()
            .insert_message(&record)
            .map_err(std::io::Error::other)?;

        // Run delivery
        let transport = transport.clone();
        let store = self.store.clone();

        let delivery_result = Self::deliver(
            transport.as_ref(),
            dest_hash,
            &payload,
        )
        .await;

        match &delivery_result {
            Ok(packet_hash) => {
                // Track receipt
                self.receipt_map
                    .lock()
                    .unwrap()
                    .insert(packet_hash.clone(), msg_id.clone());
                // Update status
                let _ = store
                    .lock()
                    .unwrap()
                    .update_receipt_status(&msg_id, "sent: direct");
            }
            Err(e) => {
                let status = format!("failed: {e}");
                let _ = store
                    .lock()
                    .unwrap()
                    .update_receipt_status(&msg_id, &status);
            }
        }

        Ok(msg_id)
    }

    /// Low-level delivery: request path → resolve identity → send via link.
    async fn deliver(
        transport: &dyn MeshTransport,
        dest_hash: AddressHash,
        payload: &[u8],
    ) -> Result<String, TransportError> {
        // Step 1: Request path
        transport.request_path(&dest_hash).await;

        // Step 2: Poll for peer identity (12s timeout)
        let mut identity = None;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(12);
        while tokio::time::Instant::now() < deadline {
            if let Some(found) = transport.resolve_identity(&dest_hash).await {
                identity = Some(found);
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }

        let identity = identity.ok_or_else(|| {
            TransportError::SendFailed("peer not announced — identity not resolved".into())
        })?;

        // Step 3: Build destination descriptor
        let dest_desc = DestinationDesc {
            identity,
            address_hash: dest_hash,
            name: DestinationName::new("lxmf", "delivery"),
        };

        // Step 4: Send via link
        let result = transport
            .send_via_link(dest_desc, payload, Duration::from_secs(20))
            .await?;

        // Extract packet hash for receipt tracking
        match result {
            rns_core::transport::delivery::LinkSendResult::Packet(packet) => {
                Ok(hex::encode(packet.hash().to_bytes()))
            }
            rns_core::transport::delivery::LinkSendResult::Resource(hash) => {
                Ok(hex::encode(hash.to_bytes()))
            }
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

    // --- Conversations & contacts (new store methods) ---

    /// List messages for a specific peer with pagination.
    pub fn list_messages_for_peer(
        &self,
        peer_hash: &str,
        limit: usize,
        before_ts: Option<i64>,
    ) -> Result<Vec<MessageRecord>, std::io::Error> {
        self.store
            .lock()
            .unwrap()
            .list_messages_for_peer(peer_hash, limit, before_ts)
            .map_err(std::io::Error::other)
    }

    /// Mark all messages from a peer as read.
    pub fn mark_read(&self, peer_hash: &str) -> Result<u64, std::io::Error> {
        self.store
            .lock()
            .unwrap()
            .mark_read(peer_hash)
            .map_err(std::io::Error::other)
    }

    /// Delete all messages in a conversation with a peer.
    pub fn delete_conversation(&self, peer_hash: &str) -> Result<u64, std::io::Error> {
        self.store
            .lock()
            .unwrap()
            .delete_conversation(peer_hash)
            .map_err(std::io::Error::other)
    }

    /// Delete a single message by ID.
    pub fn delete_message(&self, message_id: &str) -> Result<bool, std::io::Error> {
        self.store
            .lock()
            .unwrap()
            .delete_message(message_id)
            .map_err(std::io::Error::other)
    }

    /// Search messages by content substring.
    pub fn search_messages(
        &self,
        query: &str,
        peer_hash: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MessageRecord>, std::io::Error> {
        self.store
            .lock()
            .unwrap()
            .search_messages(query, peer_hash, limit)
            .map_err(std::io::Error::other)
    }

    /// List conversation summaries.
    pub fn list_conversations(
        &self,
        unread_only: bool,
    ) -> Result<Vec<crate::storage::messages::ConversationSummary>, std::io::Error> {
        self.store
            .lock()
            .unwrap()
            .list_conversations(unread_only)
            .map_err(std::io::Error::other)
    }

    /// Set a contact (upsert).
    pub fn set_contact(
        &self,
        peer_hash: &str,
        alias: Option<&str>,
        notes: Option<&str>,
    ) -> Result<crate::storage::messages::ContactRecord, std::io::Error> {
        self.store
            .lock()
            .unwrap()
            .set_contact(peer_hash, alias, notes)
            .map_err(std::io::Error::other)
    }

    /// Remove a contact.
    pub fn remove_contact(&self, peer_hash: &str) -> Result<bool, std::io::Error> {
        self.store
            .lock()
            .unwrap()
            .remove_contact(peer_hash)
            .map_err(std::io::Error::other)
    }

    /// List all contacts.
    pub fn list_contacts(
        &self,
    ) -> Result<Vec<crate::storage::messages::ContactRecord>, std::io::Error> {
        self.store
            .lock()
            .unwrap()
            .list_contacts()
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
            read: false,
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

    #[tokio::test]
    async fn send_chat_without_transport_returns_error() {
        let svc = MessagingService::new();
        let result = svc.send_chat("abcdef0123456789", "hello", None).await;
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("transport not available"),
            "should fail when no transport"
        );
    }
}
