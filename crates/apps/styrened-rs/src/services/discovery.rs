//! DiscoveryService — announce handling, path snapshots, device type detection.
//!
//! Owns: 2.1 announce handling, 2.3 path snapshots, device type detection.
//! Writes to MessagesStore announce table (conceptual "NodeStore").
//! Package: F
//!
//! Composes existing modules:
//! - `announce_names::parse_peer_name_from_app_data()` for name extraction
//! - `MessagesStore::insert_announce()` for persistence
//! - `MeshTransport::subscribe_announces()` for event source

use crate::announce_names::parse_peer_name_from_app_data;
use crate::storage::messages::{AnnounceRecord, MessagesStore};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// In-memory peer tracking (upserted on each announce).
#[derive(Debug, Clone)]
pub struct PeerRecord {
    pub peer: String,
    pub last_seen: i64,
    pub name: Option<String>,
    pub name_source: Option<String>,
    pub first_seen: i64,
    pub seen_count: u64,
}

/// Service managing device discovery via announces.
pub struct DiscoveryService {
    store: Arc<Mutex<MessagesStore>>,
    peers: Mutex<HashMap<String, PeerRecord>>,
}

impl DiscoveryService {
    /// Create with a shared store reference.
    pub fn with_store(store: Arc<Mutex<MessagesStore>>) -> Self {
        Self {
            store,
            peers: Mutex::new(HashMap::new()),
        }
    }

    /// Create a stub for tests (in-memory store).
    pub fn new() -> Self {
        let store = MessagesStore::in_memory().expect("in-memory store");
        Self {
            store: Arc::new(Mutex::new(store)),
            peers: Mutex::new(HashMap::new()),
        }
    }

    /// Process an announce from the mesh.
    ///
    /// Extracts peer name from app_data, upserts the in-memory peer record,
    /// persists to the announce table, and returns the resulting AnnounceRecord.
    pub fn accept_announce(
        &self,
        peer_hash: String,
        timestamp: i64,
        app_data: &[u8],
    ) -> Result<AnnounceRecord, std::io::Error> {
        let (name, name_source) = parse_peer_name_from_app_data(app_data)
            .map(|(n, s)| (Some(n), Some(s.to_string())))
            .unwrap_or((None, None));

        self.accept_announce_with_details(peer_hash, timestamp, name, name_source, None)
    }

    /// Process an announce with pre-parsed details.
    pub fn accept_announce_with_details(
        &self,
        peer_hash: String,
        timestamp: i64,
        name: Option<String>,
        name_source: Option<String>,
        app_data_hex: Option<String>,
    ) -> Result<AnnounceRecord, std::io::Error> {
        let peer = self.upsert_peer(&peer_hash, timestamp, name, name_source);

        let record = AnnounceRecord {
            id: format!(
                "announce-{}-{}-{}",
                peer.last_seen, peer.peer, peer.seen_count
            ),
            peer: peer.peer.clone(),
            timestamp: peer.last_seen,
            name: peer.name.clone(),
            name_source: peer.name_source.clone(),
            first_seen: peer.first_seen,
            seen_count: peer.seen_count,
            app_data_hex,
            capabilities: Vec::new(),
            rssi: None,
            snr: None,
            q: None,
            stamp_cost_flexibility: None,
            peering_cost: None,
        };

        self.store
            .lock()
            .unwrap()
            .insert_announce(&record)
            .map_err(std::io::Error::other)?;

        Ok(record)
    }

    /// Get all known peers (in-memory snapshot).
    pub fn peers(&self) -> Vec<PeerRecord> {
        self.peers.lock().unwrap().values().cloned().collect()
    }

    /// Get a specific peer by hash.
    pub fn peer(&self, hash: &str) -> Option<PeerRecord> {
        self.peers.lock().unwrap().get(hash).cloned()
    }

    /// Resolve a peer name to a hash. Case-insensitive prefix match.
    /// If `prefix` is provided, peer hash must start with it.
    pub fn resolve_name(&self, name: &str, prefix: Option<&str>) -> Option<String> {
        let name_lower = name.to_lowercase();
        let peers = self.peers.lock().unwrap();
        for peer in peers.values() {
            if let Some(ref peer_name) = peer.name {
                if peer_name.to_lowercase() == name_lower {
                    if let Some(pfx) = prefix {
                        if peer.peer.starts_with(pfx) {
                            return Some(peer.peer.clone());
                        }
                    } else {
                        return Some(peer.peer.clone());
                    }
                }
            }
        }
        None
    }

    /// Number of known peers.
    pub fn peer_count(&self) -> usize {
        self.peers.lock().unwrap().len()
    }

    /// List announces from the database.
    pub fn list_announces(&self, limit: usize) -> Result<Vec<AnnounceRecord>, std::io::Error> {
        self.store
            .lock()
            .unwrap()
            .list_announces(limit, None, None)
            .map_err(std::io::Error::other)
    }

    /// Upsert peer in the in-memory map.
    fn upsert_peer(
        &self,
        peer_hash: &str,
        timestamp: i64,
        name: Option<String>,
        name_source: Option<String>,
    ) -> PeerRecord {
        let mut peers = self.peers.lock().unwrap();

        if let Some(existing) = peers.get_mut(peer_hash) {
            existing.last_seen = timestamp;
            existing.seen_count = existing.seen_count.saturating_add(1);
            if let Some(n) = name {
                existing.name = Some(n);
                existing.name_source = name_source;
            }
            return existing.clone();
        }

        let record = PeerRecord {
            peer: peer_hash.to_string(),
            last_seen: timestamp,
            name,
            name_source,
            first_seen: timestamp,
            seen_count: 1,
        };
        peers.insert(peer_hash.to_string(), record.clone());
        record
    }
}

impl Default for DiscoveryService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_with_no_peers() {
        let svc = DiscoveryService::new();
        assert_eq!(svc.peer_count(), 0);
        assert!(svc.peers().is_empty());
    }

    #[test]
    fn accept_announce_creates_peer() {
        let svc = DiscoveryService::new();
        let result = svc.accept_announce_with_details(
            "abc123".into(),
            1000,
            Some("TestNode".into()),
            Some("delivery_app_data".into()),
            None,
        );
        assert!(result.is_ok());
        assert_eq!(svc.peer_count(), 1);
        let peer = svc.peer("abc123").unwrap();
        assert_eq!(peer.name, Some("TestNode".into()));
        assert_eq!(peer.seen_count, 1);
        assert_eq!(peer.first_seen, 1000);
    }

    #[test]
    fn repeated_announces_increment_seen_count() {
        let svc = DiscoveryService::new();
        svc.accept_announce_with_details("abc".into(), 1000, Some("Node".into()), None, None)
            .unwrap();
        svc.accept_announce_with_details("abc".into(), 2000, None, None, None)
            .unwrap();
        svc.accept_announce_with_details("abc".into(), 3000, None, None, None)
            .unwrap();

        let peer = svc.peer("abc").unwrap();
        assert_eq!(peer.seen_count, 3);
        assert_eq!(peer.first_seen, 1000);
        assert_eq!(peer.last_seen, 3000);
        assert_eq!(peer.name, Some("Node".into())); // preserved from first
    }

    #[test]
    fn name_updates_on_later_announce() {
        let svc = DiscoveryService::new();
        svc.accept_announce_with_details("peer1".into(), 1000, Some("OldName".into()), None, None)
            .unwrap();
        svc.accept_announce_with_details(
            "peer1".into(),
            2000,
            Some("NewName".into()),
            None,
            None,
        )
        .unwrap();

        let peer = svc.peer("peer1").unwrap();
        assert_eq!(peer.name, Some("NewName".into()));
    }

    #[test]
    fn announces_persisted_to_store() {
        let svc = DiscoveryService::new();
        svc.accept_announce_with_details("peer1".into(), 1000, Some("Node1".into()), None, None)
            .unwrap();
        svc.accept_announce_with_details("peer2".into(), 2000, Some("Node2".into()), None, None)
            .unwrap();

        let announces = svc.list_announces(10).unwrap();
        assert_eq!(announces.len(), 2);
    }

    #[test]
    fn resolve_name_finds_peer() {
        let svc = DiscoveryService::new();
        svc.accept_announce_with_details("abcdef01".into(), 1000, Some("Alpha".into()), None, None)
            .unwrap();
        svc.accept_announce_with_details("12345678".into(), 2000, Some("Beta".into()), None, None)
            .unwrap();

        assert_eq!(svc.resolve_name("Alpha", None), Some("abcdef01".into()));
        assert_eq!(svc.resolve_name("alpha", None), Some("abcdef01".into())); // case-insensitive
        assert_eq!(svc.resolve_name("Beta", None), Some("12345678".into()));
        assert_eq!(svc.resolve_name("Gamma", None), None); // not found
    }

    #[test]
    fn resolve_name_with_prefix() {
        let svc = DiscoveryService::new();
        svc.accept_announce_with_details("abcdef01".into(), 1000, Some("Node".into()), None, None)
            .unwrap();

        assert_eq!(svc.resolve_name("Node", Some("abc")), Some("abcdef01".into()));
        assert_eq!(svc.resolve_name("Node", Some("xyz")), None); // prefix mismatch
    }

    #[test]
    fn accept_announce_with_raw_app_data() {
        let svc = DiscoveryService::new();
        // Build a msgpack-encoded display name app_data
        let app_data = rmp_serde::to_vec(&rmpv::Value::Array(vec![
            rmpv::Value::Binary("MeshNode".as_bytes().to_vec()),
            rmpv::Value::Nil,
        ]))
        .unwrap();

        let result = svc.accept_announce("peer_hash".into(), 1000, &app_data);
        assert!(result.is_ok());
        let peer = svc.peer("peer_hash").unwrap();
        assert_eq!(peer.name, Some("MeshNode".into()));
    }
}
