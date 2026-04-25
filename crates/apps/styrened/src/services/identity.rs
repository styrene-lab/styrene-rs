//! IdentityService — operator identity, destination resolution, announce trigger.
//!
//! Owns: 1.4 operator identity, 2.4 destination resolution, announce trigger.
//! Package: E

use crate::transport::mesh_transport::MeshTransport;
use rns_core::hash::AddressHash;
use rns_core::identity::Identity;
use std::sync::Arc;

/// Manages the daemon's own identity and resolves peer identities.
pub struct IdentityService {
    /// Our operator identity hash (hex string for IPC compat).
    identity_hash: String,
    /// Our LXMF delivery destination hash (set after transport init).
    delivery_destination_hash: std::sync::Mutex<Option<String>>,
    /// Transport for announce and identity resolution.
    transport: Arc<dyn MeshTransport>,
    /// Operator display name (set via TUI or CLI).
    display_name: std::sync::Mutex<Option<String>>,
    /// Operator icon (emoji or short identifier).
    icon: std::sync::Mutex<Option<String>>,
    /// Operator short name.
    short_name: std::sync::Mutex<Option<String>>,
}

impl IdentityService {
    /// Create with a known identity hash and transport reference.
    pub fn with_transport(identity_hash: String, transport: Arc<dyn MeshTransport>) -> Self {
        Self {
            identity_hash,
            delivery_destination_hash: std::sync::Mutex::new(None),
            transport,
            display_name: std::sync::Mutex::new(None),
            icon: std::sync::Mutex::new(None),
            short_name: std::sync::Mutex::new(None),
        }
    }

    /// Create a stub for tests (no transport). Also used as `Default`.
    pub fn new() -> Self {
        Self {
            identity_hash: String::new(),
            delivery_destination_hash: std::sync::Mutex::new(None),
            transport: Arc::new(crate::transport::null_transport::NullTransport::new()),
            display_name: std::sync::Mutex::new(None),
            icon: std::sync::Mutex::new(None),
            short_name: std::sync::Mutex::new(None),
        }
    }

    /// Our operator identity hash (hex-encoded).
    pub fn identity_hash(&self) -> &str {
        &self.identity_hash
    }

    /// Our LXMF delivery destination hash (hex-encoded), if set.
    pub fn delivery_destination_hash(&self) -> Option<String> {
        self.delivery_destination_hash.lock().unwrap().clone()
    }

    /// Set the delivery destination hash (called during transport bootstrap).
    pub fn set_delivery_destination_hash(&self, hash: Option<String>) {
        *self.delivery_destination_hash.lock().unwrap() = hash;
    }

    /// Our identity address hash from the transport layer.
    pub fn transport_identity_hash(&self) -> AddressHash {
        self.transport.identity_hash()
    }

    /// Our delivery destination address hash from the transport layer.
    pub fn transport_destination_hash(&self) -> AddressHash {
        self.transport.destination_hash()
    }

    /// Get the operator display name.
    pub fn display_name(&self) -> Option<String> {
        self.display_name.lock().unwrap().clone()
    }

    /// Get the operator icon.
    pub fn icon(&self) -> Option<String> {
        self.icon.lock().unwrap().clone()
    }

    /// Get the operator short name.
    pub fn short_name(&self) -> Option<String> {
        self.short_name.lock().unwrap().clone()
    }

    /// Set identity fields. Returns true if any field changed.
    pub fn set_identity(
        &self,
        display_name: Option<&str>,
        icon: Option<&str>,
        short_name: Option<&str>,
    ) -> bool {
        let mut changed = false;
        if let Some(name) = display_name {
            let mut dn = self.display_name.lock().unwrap();
            if dn.as_deref() != Some(name) {
                *dn = Some(name.to_string());
                changed = true;
            }
        }
        if let Some(ic) = icon {
            let mut i = self.icon.lock().unwrap();
            if i.as_deref() != Some(ic) {
                *i = Some(ic.to_string());
                changed = true;
            }
        }
        if let Some(sn) = short_name {
            let mut s = self.short_name.lock().unwrap();
            if s.as_deref() != Some(sn) {
                *s = Some(sn.to_string());
                changed = true;
            }
        }
        changed
    }

    /// Resolve a peer's identity from the transport announce table.
    ///
    /// This is strategy 1 of the 5-strategy resolution cascade:
    /// 1. Transport announce table (this method)
    /// 2. NodeStore lookup (DiscoveryService)
    /// 3. Path request + wait
    /// 4. Prefix match in NodeStore
    /// 5. Return unknown
    pub async fn resolve_peer_identity(&self, dest: &AddressHash) -> Option<Identity> {
        self.transport.resolve_identity(dest).await
    }

    /// Trigger an announce with optional app_data.
    pub async fn announce(&self, app_data: Option<&[u8]>) {
        self.transport.announce(app_data).await;
    }

    /// Request path discovery for a destination.
    pub async fn request_path(&self, dest: &AddressHash) {
        self.transport.request_path(dest).await;
    }
}

impl Default for IdentityService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::mock_transport::MockTransport;

    #[test]
    fn identity_hash_returns_configured_value() {
        let mock = Arc::new(MockTransport::new_default());
        let svc = IdentityService::with_transport("abc123".into(), mock);
        assert_eq!(svc.identity_hash(), "abc123");
    }

    #[test]
    fn delivery_destination_hash_starts_none() {
        let svc = IdentityService::new();
        assert!(svc.delivery_destination_hash().is_none());
    }

    #[test]
    fn set_delivery_destination_hash_updates() {
        let svc = IdentityService::new();
        svc.set_delivery_destination_hash(Some("deadbeef".into()));
        assert_eq!(svc.delivery_destination_hash(), Some("deadbeef".into()));
    }

    #[tokio::test]
    async fn resolve_peer_identity_delegates_to_transport() {
        let mock = Arc::new(MockTransport::new_default());
        let id = rns_core::identity::PrivateIdentity::new_from_name("peer1");
        mock.queue_resolve(Some(*id.as_identity()));

        let svc = IdentityService::with_transport("test".into(), mock.clone());
        let dest = AddressHash::new([1u8; 16]);
        let result = svc.resolve_peer_identity(&dest).await;
        assert!(result.is_some());
        assert_eq!(mock.call_count(), 1);
    }

    #[test]
    fn set_identity_stores_fields() {
        let svc = IdentityService::new();
        assert!(svc.display_name().is_none());

        let changed = svc.set_identity(Some("Alice"), Some("🔑"), Some("A"));
        assert!(changed);
        assert_eq!(svc.display_name().as_deref(), Some("Alice"));
        assert_eq!(svc.icon().as_deref(), Some("🔑"));
        assert_eq!(svc.short_name().as_deref(), Some("A"));
    }

    #[test]
    fn set_identity_returns_false_when_unchanged() {
        let svc = IdentityService::new();
        svc.set_identity(Some("Alice"), None, None);
        let changed = svc.set_identity(Some("Alice"), None, None);
        assert!(!changed);
    }

    #[tokio::test]
    async fn announce_delegates_to_transport() {
        let mock = Arc::new(MockTransport::new_default());
        let svc = IdentityService::with_transport("test".into(), mock.clone());
        svc.announce(Some(b"app-data")).await;
        assert_eq!(mock.call_count(), 1);
    }
}
