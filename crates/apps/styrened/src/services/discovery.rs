//! DiscoveryService — announce handling, path snapshots, device type detection.
//!
//! Owns: 2.1 announce handling, 2.3 path snapshots, device type detection.
//! Package: F
//!
//! Composes:
//! - `NodeStore` (styrene-services) for persistent peer registry
//! - `MessagesStore` for announce table (legacy compat, written in parallel)
//! - `announce_names::parse_peer_name_from_app_data()` for name extraction

use crate::announce_names::parse_peer_name_from_app_data;
use crate::storage::messages::{AnnounceRecord, MessagesStore};
use crate::storage::standard_propagation::StandardPropagationPeer;
use std::sync::{Arc, Mutex};
use styrene_ipc::types::{DeviceInfo, DiscoveredCapability};
use styrene_services::node_store::{AnnounceRoute, Node, NodeStore};

pub const NATIVE_NOMADNET_HOST_DEVICE_TYPE: &str = "native_nomadnet_host";
pub const LXMF_DELIVERY_DEVICE_TYPE: &str = "lxmf_delivery";
pub const STANDARD_LXMF_PROPAGATION_ACTIVE_DEVICE_TYPE: &str =
    "standard_lxmf_propagation_host_active";
pub const STANDARD_LXMF_PROPAGATION_INACTIVE_DEVICE_TYPE: &str =
    "standard_lxmf_propagation_host_inactive";
const LEGACY_NOMADNET_HOST_DEVICE_TYPE: &str = "page_host";

/// Service managing device discovery via announces.
///
/// Peer state is persisted in `NodeStore` (SQLite) and survives daemon
/// restarts. The legacy `MessagesStore` announce table is written in
/// parallel for backward compatibility with the RPC daemon.
pub struct DiscoveryService {
    store: Arc<Mutex<MessagesStore>>,
    node_store: Arc<NodeStore>,
}

impl DiscoveryService {
    /// Create with shared store references.
    pub fn with_stores(store: Arc<Mutex<MessagesStore>>, node_store: Arc<NodeStore>) -> Self {
        Self { store, node_store }
    }

    /// Create with a shared store reference and an in-memory NodeStore.
    pub fn with_store(store: Arc<Mutex<MessagesStore>>) -> Self {
        let node_store = Arc::new(NodeStore::in_memory().expect("in-memory node store"));
        Self { store, node_store }
    }

    /// Create a stub for tests (in-memory stores).
    pub fn new() -> Self {
        let store = MessagesStore::in_memory().expect("in-memory store");
        let node_store = Arc::new(NodeStore::in_memory().expect("in-memory node store"));
        Self { store: Arc::new(Mutex::new(store)), node_store }
    }

    /// Access the underlying NodeStore.
    pub fn node_store(&self) -> &NodeStore {
        &self.node_store
    }

    /// Process an announce from the mesh.
    ///
    /// Extracts peer name from app_data, persists to both NodeStore and
    /// the legacy announce table.
    pub fn accept_announce(
        &self,
        peer_hash: String,
        timestamp: i64,
        app_data: &[u8],
    ) -> Result<AnnounceRecord, std::io::Error> {
        let (name, name_source) = parse_peer_name_from_app_data(app_data)
            .map(|(n, s)| (Some(n), Some(s.to_string())))
            .unwrap_or((None, None));

        let metadata = lxmf::announce::delivery_announce_metadata(app_data);
        let stamp_cost = metadata.as_ref().and_then(|value| value.stamp_cost);
        let record = self.accept_announce_with_details(
            peer_hash.clone(),
            timestamp,
            name,
            name_source,
            None,
        )?;
        let record = AnnounceRecord {
            stamp_cost,
            capabilities: metadata
                .map(|value| {
                    value
                        .supported_functionality
                        .into_iter()
                        .map(|function| format!("lxmf_function:{function}"))
                        .collect()
                })
                .unwrap_or_default(),
            ..record
        };
        if stamp_cost.is_some() || !record.capabilities.is_empty() {
            let store = self
                .store
                .lock()
                .map_err(|_| std::io::Error::other("messages store lock poisoned"))?;
            store.insert_announce(&record).map_err(std::io::Error::other)?;
        }
        Ok(record)
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
        // Persist to NodeStore (primary, survives restart)
        let node = self
            .node_store
            .accept_announce(
                &peer_hash,
                timestamp,
                name.as_deref(),
                name_source.as_deref(),
                None, // device_type
                None, // signal_quality
            )
            .map_err(|e| std::io::Error::other(e.to_string()))?;

        // Build AnnounceRecord for legacy compat
        let record = AnnounceRecord {
            id: format!(
                "announce-{}-{}-{}",
                node.last_seen, node.identity_hash, node.announce_count
            ),
            peer: node.identity_hash.clone(),
            timestamp: node.last_seen,
            name: node.display_name.clone(),
            name_source: node.name_source.clone(),
            first_seen: node.first_seen,
            seen_count: node.announce_count,
            app_data_hex,
            capabilities: Vec::new(),
            rssi: None,
            snr: None,
            q: None,
            stamp_cost: None,
            stamp_cost_flexibility: None,
            peering_cost: None,
        };

        // Write to legacy announce table (secondary)
        self.store
            .lock()
            .map_err(|_| std::io::Error::other("messages store lock poisoned"))?
            .insert_announce(&record)
            .map_err(std::io::Error::other)?;

        Ok(record)
    }

    /// Process an announce with aspect-derived device type.
    pub fn accept_announce_with_type(
        &self,
        peer_hash: String,
        timestamp: i64,
        app_data: &[u8],
        device_type: Option<&str>,
    ) -> Result<AnnounceRecord, std::io::Error> {
        self.accept_announce_with_route(
            peer_hash,
            timestamp,
            app_data,
            device_type,
            &AnnounceRoute::default(),
        )
    }

    /// Process an announce reception, recording the announcing identity and the
    /// route the announce arrived over alongside the aspect-derived type.
    pub fn accept_announce_with_route(
        &self,
        peer_hash: String,
        timestamp: i64,
        app_data: &[u8],
        device_type: Option<&str>,
        route: &AnnounceRoute,
    ) -> Result<AnnounceRecord, std::io::Error> {
        let (name, name_source) = parse_peer_name_from_app_data(app_data)
            .map(|(n, s)| (Some(n), Some(s.to_string())))
            .unwrap_or((None, None));

        // Persist to NodeStore with device_type from aspect classification
        let node = self
            .node_store
            .accept_announce_with_route(
                &peer_hash,
                timestamp,
                name.as_deref(),
                name_source.as_deref(),
                device_type,
                None,
                route,
            )
            .map_err(|e| std::io::Error::other(e.to_string()))?;

        let metadata = if device_type.is_none() || device_type == Some(LXMF_DELIVERY_DEVICE_TYPE) {
            lxmf::announce::delivery_announce_metadata(app_data)
        } else {
            None
        };
        let stamp_cost = metadata.as_ref().and_then(|value| value.stamp_cost);
        let record = AnnounceRecord {
            id: format!(
                "announce-{}-{}-{}",
                node.last_seen, node.identity_hash, node.announce_count
            ),
            peer: node.identity_hash.clone(),
            timestamp: node.last_seen,
            name: node.display_name.clone(),
            name_source: node.name_source.clone(),
            first_seen: node.first_seen,
            seen_count: node.announce_count,
            app_data_hex: None,
            capabilities: metadata
                .map(|value| {
                    value
                        .supported_functionality
                        .into_iter()
                        .map(|function| format!("lxmf_function:{function}"))
                        .collect()
                })
                .unwrap_or_default(),
            rssi: None,
            snr: None,
            q: None,
            stamp_cost,
            stamp_cost_flexibility: None,
            peering_cost: None,
        };

        let store =
            self.store.lock().map_err(|_| std::io::Error::other("messages store lock poisoned"))?;
        store.insert_announce(&record).map_err(std::io::Error::other)?;
        Ok(record)
    }

    pub fn accept_delivery_announce(
        &self,
        destination_hash: String,
        timestamp: i64,
        app_data: &[u8],
    ) -> Result<AnnounceRecord, std::io::Error> {
        self.accept_delivery_announce_with_route(
            destination_hash,
            timestamp,
            app_data,
            &AnnounceRoute::default(),
        )
    }

    pub fn accept_delivery_announce_with_route(
        &self,
        destination_hash: String,
        timestamp: i64,
        app_data: &[u8],
        route: &AnnounceRoute,
    ) -> Result<AnnounceRecord, std::io::Error> {
        lxmf::announce::delivery_announce_metadata(app_data)
            .ok_or_else(|| std::io::Error::other("invalid canonical LXMF delivery app data"))?;
        self.accept_announce_with_route(
            destination_hash,
            timestamp,
            app_data,
            Some(LXMF_DELIVERY_DEVICE_TYPE),
            route,
        )
    }

    pub fn accept_standard_propagation_announce(
        &self,
        peer_hash: String,
        identity_hash: [u8; 16],
        propagation_destination: [u8; 16],
        timestamp: i64,
        metadata: &lxmf::propagation_announce::StandardPropagationAnnounce,
    ) -> Result<AnnounceRecord, std::io::Error> {
        self.accept_standard_propagation_announce_with_route(
            peer_hash,
            identity_hash,
            propagation_destination,
            timestamp,
            metadata,
            &AnnounceRoute {
                identity_hash: Some(hex::encode(identity_hash)),
                ..AnnounceRoute::default()
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn accept_standard_propagation_announce_with_route(
        &self,
        peer_hash: String,
        identity_hash: [u8; 16],
        propagation_destination: [u8; 16],
        timestamp: i64,
        metadata: &lxmf::propagation_announce::StandardPropagationAnnounce,
        route: &AnnounceRoute,
    ) -> Result<AnnounceRecord, std::io::Error> {
        let device_type = if metadata.node_active {
            STANDARD_LXMF_PROPAGATION_ACTIVE_DEVICE_TYPE
        } else {
            STANDARD_LXMF_PROPAGATION_INACTIVE_DEVICE_TYPE
        };
        let mut record = self.accept_announce_with_details_and_type(
            peer_hash,
            timestamp,
            metadata.node_name.clone(),
            Some("standard_lxmf_propagation_metadata".into()),
            device_type,
            route,
        )?;
        record.stamp_cost = u32::try_from(metadata.stamp_cost).ok();
        record.stamp_cost_flexibility = u32::try_from(metadata.stamp_cost_flexibility).ok();
        record.peering_cost = u32::try_from(metadata.peering_cost).ok();
        record.capabilities = vec![format!(
            "standard_lxmf_propagation:{}",
            if metadata.node_active { "active" } else { "inactive" }
        )];
        let mut store =
            self.store.lock().map_err(|_| std::io::Error::other("messages store lock poisoned"))?;
        store.insert_announce(&record).map_err(std::io::Error::other)?;
        store
            .standard_propagation_upsert_peer(&StandardPropagationPeer {
                identity_hash,
                propagation_destination: Some(propagation_destination),
                configured: false,
                enabled: metadata.node_active,
                transfer_limit_kb: usize::try_from(metadata.transfer_limit_kb).ok(),
                sync_limit_kb: usize::try_from(metadata.sync_limit_kb).ok(),
                stamp_cost: u32::try_from(metadata.stamp_cost).ok(),
                stamp_flexibility: u32::try_from(metadata.stamp_cost_flexibility).ok(),
                peering_cost: u32::try_from(metadata.peering_cost).ok(),
                observed_at: timestamp.max(0),
            })
            .map_err(std::io::Error::other)?;
        Ok(record)
    }

    fn accept_announce_with_details_and_type(
        &self,
        peer_hash: String,
        timestamp: i64,
        name: Option<String>,
        name_source: Option<String>,
        device_type: &str,
        route: &AnnounceRoute,
    ) -> Result<AnnounceRecord, std::io::Error> {
        let node = self
            .node_store
            .accept_announce_with_route(
                &peer_hash,
                timestamp,
                name.as_deref(),
                name_source.as_deref(),
                Some(device_type),
                None,
                route,
            )
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(AnnounceRecord {
            id: format!(
                "announce-{}-{}-{}",
                node.last_seen, node.identity_hash, node.announce_count
            ),
            peer: node.identity_hash,
            timestamp: node.last_seen,
            name: node.display_name,
            name_source: node.name_source,
            first_seen: node.first_seen,
            seen_count: node.announce_count,
            app_data_hex: None,
            capabilities: Vec::new(),
            rssi: None,
            snr: None,
            q: None,
            stamp_cost: None,
            stamp_cost_flexibility: None,
            peering_cost: None,
        })
    }

    /// Get all known peers from persistent NodeStore.
    pub fn peers(&self) -> Vec<Node> {
        self.node_store.list(None).unwrap_or_default()
    }

    /// Project persisted announce evidence into the frontend discovery contract.
    pub fn devices(&self) -> Vec<DeviceInfo> {
        self.peers().into_iter().map(project_device).collect()
    }

    pub fn device(&self, hash: &str) -> Option<DeviceInfo> {
        self.peer(hash).map(project_device)
    }

    /// Get a specific peer by hash.
    pub fn peer(&self, hash: &str) -> Option<Node> {
        self.node_store.get(hash).ok().flatten()
    }

    /// Resolve a peer name to a hash. Case-insensitive match.
    /// If `prefix` is provided, peer hash must start with it.
    pub fn resolve_name(&self, name: &str, prefix: Option<&str>) -> Option<String> {
        let name_lower = name.to_lowercase();
        let nodes = self.node_store.list(None).unwrap_or_default();
        for node in &nodes {
            if let Some(ref display_name) = node.display_name
                && display_name.to_lowercase() == name_lower
            {
                if let Some(pfx) = prefix {
                    if node.identity_hash.starts_with(pfx) {
                        return Some(node.identity_hash.clone());
                    }
                } else {
                    return Some(node.identity_hash.clone());
                }
            }
        }
        None
    }

    /// Number of known peers.
    pub fn peer_count(&self) -> usize {
        self.node_store.count().unwrap_or(0) as usize
    }

    /// List announces from the legacy database.
    pub fn list_announces(&self, limit: usize) -> Result<Vec<AnnounceRecord>, std::io::Error> {
        self.store.lock().unwrap().list_announces(limit, None, None).map_err(std::io::Error::other)
    }

    /// Block a node.
    pub fn block_peer(&self, identity_hash: &str) -> Result<(), std::io::Error> {
        self.node_store
            .set_blocked(identity_hash, true)
            .map_err(|e| std::io::Error::other(e.to_string()))
    }

    /// Unblock a node.
    pub fn unblock_peer(&self, identity_hash: &str) -> Result<(), std::io::Error> {
        self.node_store
            .set_blocked(identity_hash, false)
            .map_err(|e| std::io::Error::other(e.to_string()))
    }

    /// Bookmark a node.
    pub fn bookmark_peer(&self, identity_hash: &str) -> Result<(), std::io::Error> {
        self.node_store
            .set_bookmarked(identity_hash, true)
            .map_err(|e| std::io::Error::other(e.to_string()))
    }
}

fn project_device(node: Node) -> DeviceInfo {
    let native_nomadnet_host = matches!(
        node.device_type.as_deref(),
        Some(NATIVE_NOMADNET_HOST_DEVICE_TYPE | LEGACY_NOMADNET_HOST_DEVICE_TYPE)
    );
    let mut device = DeviceInfo::default();
    // The discovery key is the announced destination hash. The identity behind
    // it is recorded only when an announce reception observed it, so legacy
    // rows project an empty identity hash.
    device.destination_hash = node.identity_hash;
    device.identity_hash = node.peer_identity_hash.unwrap_or_default();
    device.hops = node.last_hops;
    device.interface_kind = node.last_interface_kind;
    device.name = node.display_name.unwrap_or_default();
    device.device_type = if native_nomadnet_host {
        NATIVE_NOMADNET_HOST_DEVICE_TYPE.into()
    } else {
        node.device_type.unwrap_or_else(|| "unknown".into())
    };
    device.status = "announced".into();
    device.last_announce = Some(node.last_seen);
    device.announce_count = u32::try_from(node.announce_count).unwrap_or(u32::MAX);
    if native_nomadnet_host {
        device.discovered_capabilities.push(DiscoveredCapability::NativeNomadNetHost);
    }
    device.standard_lxmf_propagation_active = match device.device_type.as_str() {
        STANDARD_LXMF_PROPAGATION_ACTIVE_DEVICE_TYPE => Some(true),
        STANDARD_LXMF_PROPAGATION_INACTIVE_DEVICE_TYPE => Some(false),
        _ => None,
    };
    if device.standard_lxmf_propagation_active.is_some() {
        device.discovered_capabilities.push(DiscoveredCapability::StandardLxmfPropagationHost);
    }
    device
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
        assert_eq!(peer.display_name, Some("TestNode".into()));
        assert_eq!(peer.announce_count, 1);
        assert_eq!(peer.first_seen, 1000);
    }

    #[test]
    fn repeated_announces_increment_count() {
        let svc = DiscoveryService::new();
        svc.accept_announce_with_details("abc".into(), 1000, Some("Node".into()), None, None)
            .unwrap();
        svc.accept_announce_with_details("abc".into(), 2000, None, None, None).unwrap();
        svc.accept_announce_with_details("abc".into(), 3000, None, None, None).unwrap();

        let peer = svc.peer("abc").unwrap();
        assert_eq!(peer.announce_count, 3);
        assert_eq!(peer.first_seen, 1000);
        assert_eq!(peer.last_seen, 3000);
        assert_eq!(peer.display_name, Some("Node".into())); // preserved from first
    }

    #[test]
    fn name_updates_on_later_announce() {
        let svc = DiscoveryService::new();
        svc.accept_announce_with_details("peer1".into(), 1000, Some("OldName".into()), None, None)
            .unwrap();
        svc.accept_announce_with_details("peer1".into(), 2000, Some("NewName".into()), None, None)
            .unwrap();

        let peer = svc.peer("peer1").unwrap();
        assert_eq!(peer.display_name, Some("NewName".into()));
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
        assert_eq!(svc.resolve_name("alpha", None), Some("abcdef01".into()));
        assert_eq!(svc.resolve_name("Beta", None), Some("12345678".into()));
        assert_eq!(svc.resolve_name("Gamma", None), None);
    }

    #[test]
    fn resolve_name_with_prefix() {
        let svc = DiscoveryService::new();
        svc.accept_announce_with_details("abcdef01".into(), 1000, Some("Node".into()), None, None)
            .unwrap();

        assert_eq!(svc.resolve_name("Node", Some("abc")), Some("abcdef01".into()));
        assert_eq!(svc.resolve_name("Node", Some("xyz")), None);
    }

    #[test]
    fn accept_announce_with_raw_app_data() {
        let svc = DiscoveryService::new();
        let app_data = rmp_serde::to_vec(&rmpv::Value::Array(vec![
            rmpv::Value::Binary("MeshNode".as_bytes().to_vec()),
            rmpv::Value::Nil,
        ]))
        .unwrap();

        let result = svc.accept_announce("peer_hash".into(), 1000, &app_data);
        assert!(result.is_ok());
        let peer = svc.peer("peer_hash").unwrap();
        assert_eq!(peer.display_name, Some("MeshNode".into()));
    }

    #[test]
    fn native_page_host_capability_requires_nomadnet_aspect_evidence() {
        let svc = DiscoveryService::new();
        svc.accept_announce_with_type(
            "0123456789abcdef0123456789abcdef".into(),
            1000,
            b"Native host",
            Some(NATIVE_NOMADNET_HOST_DEVICE_TYPE),
        )
        .unwrap();
        svc.accept_announce_with_details(
            "fedcba9876543210fedcba9876543210".into(),
            1001,
            Some("[styrene:{\"capabilities\":[\"pages\"]}] Placeholder".into()),
            None,
            None,
        )
        .unwrap();

        let devices = svc.devices();
        let native = devices
            .iter()
            .find(|device| device.destination_hash.starts_with("0123"))
            .expect("native host projection");
        assert_eq!(native.discovered_capabilities, vec![DiscoveredCapability::NativeNomadNetHost]);
        let placeholder = devices
            .iter()
            .find(|device| device.destination_hash.starts_with("fedc"))
            .expect("placeholder projection");
        assert!(placeholder.discovered_capabilities.is_empty());
    }

    #[test]
    fn peers_survive_after_creation() {
        // NodeStore is SQLite — data persists within the connection lifetime
        let svc = DiscoveryService::new();
        svc.accept_announce_with_details("p1".into(), 1000, Some("N1".into()), None, None).unwrap();
        svc.accept_announce_with_details("p2".into(), 2000, Some("N2".into()), None, None).unwrap();

        // Peers are queryable from NodeStore
        let peers = svc.peers();
        assert_eq!(peers.len(), 2);
        assert_eq!(peers[0].identity_hash, "p2"); // most recent first
    }
}
