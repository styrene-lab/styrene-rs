//! IdentityService — operator identity, destination resolution, announce trigger.
//!
//! Owns: 1.4 operator identity, 2.4 destination resolution, announce trigger.
//! Package: E

use crate::transport::mesh_transport::MeshTransport;
use rns_core::hash::AddressHash;
use rns_core::identity::Identity;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

const MAX_PUBLIC_IDENTITY_FIELD_CHARS: usize = 64;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PublicIdentityMetadata {
    pub display_name: Option<String>,
    pub icon: Option<String>,
    pub short_name: Option<String>,
}

#[derive(Default)]
struct PublicIdentityState {
    metadata: PublicIdentityMetadata,
    path: Option<PathBuf>,
}

/// Manages the daemon's own identity and resolves peer identities.
pub struct IdentityService {
    /// Our operator identity hash (hex string for IPC compat).
    identity_hash: String,
    /// Our LXMF delivery destination hash (set after transport init).
    delivery_destination_hash: std::sync::Mutex<Option<String>>,
    /// Transport for announce and identity resolution.
    transport: Arc<dyn MeshTransport>,
    /// Public fields and their persistence target share one serialized state.
    public_identity: std::sync::Mutex<PublicIdentityState>,
    custody: std::sync::Mutex<Option<styrene_ipc::types::IdentityCustodyInfo>>,
}

impl IdentityService {
    /// Create with a known identity hash and transport reference.
    pub fn with_transport(identity_hash: String, transport: Arc<dyn MeshTransport>) -> Self {
        Self {
            identity_hash,
            delivery_destination_hash: std::sync::Mutex::new(None),
            transport,
            public_identity: std::sync::Mutex::new(PublicIdentityState::default()),
            custody: std::sync::Mutex::new(None),
        }
    }

    /// Create a stub for tests (no transport). Also used as `Default`.
    pub fn new() -> Self {
        Self {
            identity_hash: String::new(),
            delivery_destination_hash: std::sync::Mutex::new(None),
            transport: Arc::new(crate::transport::null_transport::NullTransport::new()),
            public_identity: std::sync::Mutex::new(PublicIdentityState::default()),
            custody: std::sync::Mutex::new(None),
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
        self.public_identity.lock().unwrap().metadata.display_name.clone()
    }

    /// Get the operator icon.
    pub fn icon(&self) -> Option<String> {
        self.public_identity.lock().unwrap().metadata.icon.clone()
    }

    /// Get the operator short name.
    pub fn short_name(&self) -> Option<String> {
        self.public_identity.lock().unwrap().metadata.short_name.clone()
    }

    pub fn custody(&self) -> Option<styrene_ipc::types::IdentityCustodyInfo> {
        self.custody.lock().unwrap().clone()
    }

    pub(crate) fn configure_mobile_identity(
        &self,
        metadata_path: PathBuf,
        metadata: PublicIdentityMetadata,
        custody: styrene_ipc::types::IdentityCustodyInfo,
    ) {
        *self.public_identity.lock().unwrap() =
            PublicIdentityState { metadata, path: Some(metadata_path) };
        *self.custody.lock().unwrap() = Some(custody);
    }

    pub fn set_identity_validated(
        &self,
        display_name: Option<&str>,
        icon: Option<&str>,
        short_name: Option<&str>,
    ) -> Result<bool, String> {
        let display_name = display_name.map(|value| validate_public_field("display name", value));
        let icon = icon.map(|value| validate_public_field("icon", value));
        let short_name = short_name.map(|value| validate_public_field("short name", value));
        let display_name = display_name.transpose()?;
        let icon = icon.transpose()?;
        let short_name = short_name.transpose()?;

        self.update_public_identity(display_name, icon, short_name)
    }

    fn update_public_identity(
        &self,
        display_name: Option<String>,
        icon: Option<String>,
        short_name: Option<String>,
    ) -> Result<bool, String> {
        let mut state = self.public_identity.lock().unwrap();
        let current = &state.metadata;
        let next = PublicIdentityMetadata {
            display_name: display_name.or(current.display_name.clone()),
            icon: icon.or(current.icon.clone()),
            short_name: short_name.or(current.short_name.clone()),
        };
        if next == *current {
            return Ok(false);
        }
        if let Some(path) = &state.path {
            let bytes = serde_json::to_vec(&next)
                .map_err(|error| format!("serialize public identity metadata: {error}"))?;
            crate::config::atomic_write_private(path, &bytes)
                .map_err(|error| format!("persist public identity metadata: {error}"))?;
        }
        state.metadata = next;
        Ok(true)
    }

    /// Set identity fields. Returns true if any field changed.
    pub fn set_identity(
        &self,
        display_name: Option<&str>,
        icon: Option<&str>,
        short_name: Option<&str>,
    ) -> bool {
        self.update_public_identity(
            display_name.map(str::to_owned),
            icon.map(str::to_owned),
            short_name.map(str::to_owned),
        )
        .unwrap_or(false)
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
        let encoded = if app_data.is_none() {
            self.display_name().and_then(|name| {
                crate::announce_names::encode_delivery_display_name_app_data(&name)
            })
        } else {
            None
        };
        self.transport.announce(app_data.or(encoded.as_deref())).await;
    }

    /// Request path discovery for a destination.
    pub async fn request_path(&self, dest: &AddressHash) {
        self.transport.request_path(dest).await;
    }
}

pub(crate) fn validate_public_field(kind: &str, value: &str) -> Result<String, String> {
    let normalized = value.trim();
    if normalized.is_empty() {
        return Err(format!("{kind} must not be empty"));
    }
    if normalized.chars().any(char::is_control) {
        return Err(format!("{kind} must not contain control characters"));
    }
    if normalized.chars().count() > MAX_PUBLIC_IDENTITY_FIELD_CHARS {
        return Err(format!("{kind} exceeds {MAX_PUBLIC_IDENTITY_FIELD_CHARS} characters"));
    }
    Ok(normalized.to_string())
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
    use std::sync::Barrier;

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

    #[test]
    fn concurrent_public_metadata_edits_merge_in_memory_and_on_disk() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("identity-public.json");
        let svc = Arc::new(IdentityService::new());
        svc.public_identity.lock().unwrap().path = Some(path.clone());
        let barrier = Arc::new(Barrier::new(4));

        let edits = [
            (Some("Field Node"), None, None),
            (None, Some("radio"), None),
            (None, None, Some("FN")),
        ];
        let threads = edits.map(|(display_name, icon, short_name)| {
            let svc = svc.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                svc.set_identity_validated(display_name, icon, short_name).unwrap()
            })
        });
        barrier.wait();
        for thread in threads {
            assert!(thread.join().unwrap());
        }

        let persisted: PublicIdentityMetadata =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let in_memory = svc.public_identity.lock().unwrap().metadata.clone();
        assert_eq!(persisted, in_memory);
        assert_eq!(persisted.display_name.as_deref(), Some("Field Node"));
        assert_eq!(persisted.icon.as_deref(), Some("radio"));
        assert_eq!(persisted.short_name.as_deref(), Some("FN"));
    }

    #[test]
    fn failed_public_metadata_persistence_leaves_memory_unchanged() {
        let temp = tempfile::tempdir().unwrap();
        let parent_file = temp.path().join("not-a-directory");
        std::fs::write(&parent_file, b"occupied").unwrap();
        let svc = IdentityService::new();
        svc.public_identity.lock().unwrap().path = Some(parent_file.join("identity-public.json"));

        let error = svc
            .set_identity_validated(Some("Field Node"), None, None)
            .expect_err("persistence through a file parent must fail");

        assert!(error.contains("persist public identity metadata"));
        assert_eq!(svc.display_name(), None);
    }

    #[tokio::test]
    async fn announce_delegates_to_transport() {
        let mock = Arc::new(MockTransport::new_default());
        let svc = IdentityService::with_transport("test".into(), mock.clone());
        svc.announce(Some(b"app-data")).await;
        assert_eq!(mock.call_count(), 1);
    }

    #[tokio::test]
    async fn announce_uses_current_normalized_public_name() {
        use crate::transport::mock_transport::MockCall;

        let mock = Arc::new(MockTransport::new_default());
        let svc = IdentityService::with_transport("test".into(), mock.clone());
        assert!(svc.set_identity_validated(Some("  Field Node  "), None, None).unwrap());

        svc.announce(None).await;

        let expected = crate::announce_names::encode_delivery_display_name_app_data("Field Node");
        assert!(matches!(
            mock.calls().as_slice(),
            [MockCall::Announce { app_data }] if app_data == &expected
        ));
    }
}
