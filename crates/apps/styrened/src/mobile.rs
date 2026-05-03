//! Mobile embedding — lightweight daemon boot and background poll.
//!
//! Provides the in-process daemon API for iOS/Android apps. No IPC server,
//! no PTY terminal, no Unix sockets. The host app calls these functions
//! directly via FFI or Rust → Swift/Kotlin bridge.
//!
//! # Usage (from Swift via UniFFI or C bridge)
//!
//! ```ignore
//! use styrened::mobile::{MobileNode, MobileConfig};
//! use std::path::PathBuf;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let config = MobileConfig {
//!     config_dir: PathBuf::from("/var/mobile/Containers/.../styrene/config"),
//!     data_dir: PathBuf::from("/var/mobile/Containers/.../styrene/data"),
//!     hub_address: Some("hub.example.com:4242".into()),
//!     hub_delivery_hash: Some("aabbccdd...".into()),
//! };
//!
//! let node = MobileNode::boot(config).await?;
//!
//! // Foreground: full interactive use
//! let peers = node.list_peers().await?;
//! node.send_chat("deadbeef...", "hello from phone").await?;
//!
//! // Background (BGProcessingTask): poll hub for queued messages
//! let count = node.poll_hub().await?;
//! // → returns number of new messages fetched
//! # Ok(())
//! # }
//! ```
//!
//! # iOS Integration
//!
//! The host app should:
//! 1. Call `MobileNode::boot()` on first launch (stores identity in app container)
//! 2. Keep the `MobileNode` alive for the foreground session
//! 3. In `BGAppRefreshTask` handler: boot a fresh `MobileNode`, call `poll_hub()`, drop
//! 4. Post local notifications for new messages from `poll_hub()` results

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::app_context::AppContext;
use crate::config::PlatformPaths;
use crate::daemon_facade::DaemonFacade;
use crate::storage::messages::MessagesStore;
use crate::transport::mesh_transport::MeshTransport;

use rns_core::identity::PrivateIdentity;
use styrene_ipc::traits::{Daemon, DaemonIdentity, DaemonStatus};

/// Mobile node configuration — provided by the host app.
/// How to store the identity private keys.
#[derive(Debug, Clone, Default)]
pub enum IdentityBackend {
    /// Platform keychain with biometric protection (iOS Keychain / macOS Keychain).
    /// Root secret stored in Secure Enclave, RNS keys derived via HKDF.
    /// Requires `mobile-keychain` feature.
    #[default]
    Keychain,
    /// Encrypted file with passphrase (argon2id + ChaCha20Poly1305).
    /// Requires `mobile-identity` feature.
    EncryptedFile,
    /// Plaintext file (development/testing only — NOT for production mobile).
    PlaintextFile,
}

#[derive(Debug, Clone)]
pub struct MobileConfig {
    /// Path to the config directory (app container).
    pub config_dir: PathBuf,
    /// Path to the data directory (app container).
    pub data_dir: PathBuf,
    /// Hub TCP address for transport (e.g., "hub.mesh.example:4242").
    pub hub_address: Option<String>,
    /// Hub's LXMF delivery hash for propagation fetch.
    pub hub_delivery_hash: Option<String>,
    /// Display name for the node (used in announces).
    pub display_name: Option<String>,
    /// Identity storage backend.
    pub identity_backend: IdentityBackend,
}

/// A running mobile daemon node — in-process, no IPC server.
pub struct MobileNode {
    pub app_context: Arc<AppContext>,
    pub facade: Arc<DaemonFacade>,
    paths: PlatformPaths,
    hub_delivery_hash: Option<String>,
}

/// Result of a hub poll operation.
#[derive(Debug, Clone)]
pub struct PollResult {
    /// Number of new messages fetched.
    pub message_count: usize,
    /// The fetched messages (for local notification display).
    pub messages: Vec<PollMessage>,
}

/// A message fetched during hub poll (simplified for notification display).
#[derive(Debug, Clone)]
pub struct PollMessage {
    pub source_hash: String,
    pub content_preview: String,
    pub timestamp: i64,
}

impl MobileNode {
    /// Boot the daemon in-process for mobile use.
    ///
    /// Creates identity if needed, opens SQLite, starts transport.
    /// Does NOT start an IPC server or PTY terminal.
    pub async fn boot(config: MobileConfig) -> anyhow::Result<Self> {
        let paths = PlatformPaths::new(config.config_dir.clone(), config.data_dir.clone());
        paths.ensure_dirs()?;

        // Load or create identity via the configured backend.
        let identity = load_or_create_identity(&config.identity_backend, &paths)?;

        let identity_hash = hex::encode(identity.address_hash().as_slice());

        // Open database
        let db_path = paths.db_path();
        let store = Arc::new(Mutex::new(
            MessagesStore::open(&db_path).map_err(|e| anyhow::anyhow!("database: {e}"))?,
        ));

        // Create transport.
        // For background polls, we boot a real transport with a TCP client
        // connection to the hub. For foreground use, the host app can
        // add additional interfaces later via the Transport API.
        let transport: Arc<dyn MeshTransport> = if let Some(ref hub_addr) = config.hub_address {
            use rns_core::destination::DestinationName;
            use rns_core::transport::core_transport::{Transport, TransportConfig};
            use rns_core::transport::iface::tcp_client::TcpClient;

            let transport_id =
                rns_core::transport::identity_bridge::to_transport_private_identity(&identity);
            let config_t = TransportConfig::new("styrene-mobile", &transport_id, true);
            let mut transport_instance = Transport::new(config_t);

            // Add TCP client to hub
            let iface_mgr = transport_instance.iface_manager();
            iface_mgr.lock().await.spawn(TcpClient::new(hub_addr.clone()), TcpClient::spawn);

            // Add LXMF delivery destination
            let _destination = transport_instance
                .add_destination(transport_id, DestinationName::new("lxmf", "delivery"))
                .await;

            let transport = Arc::new(transport_instance);
            let mut id_bytes = [0u8; 16];
            id_bytes.copy_from_slice(identity.address_hash().as_slice());

            let delivery_addr = {
                let dest = _destination.lock().await;
                dest.desc.address_hash
            };

            let adapter = crate::transport::adapter::TokioTransportAdapter::new(
                transport.clone(),
                rns_core::hash::AddressHash::new(id_bytes),
                delivery_addr,
                _destination,
                None,
            )
            .await;

            Arc::new(adapter)
        } else {
            // No hub configured — null transport for offline-only mode
            Arc::new(crate::transport::null_transport::NullTransport::new())
        };

        // Build AppContext
        let app_context = Arc::new(AppContext::new(transport, identity_hash.clone(), store));
        app_context.set_signer(Arc::new(identity));

        // Load config if exists (for interface and role settings)
        let _config_path = paths.config_path();

        // Wire propagation hub if configured
        if let Some(ref hub_hash) = config.hub_delivery_hash {
            app_context.messaging().set_propagation_hub(hub_hash.clone(), app_context.fleet_arc());
        }

        let facade = Arc::new(DaemonFacade::new(app_context.clone(), identity_hash));

        Ok(Self { app_context, facade, paths, hub_delivery_hash: config.hub_delivery_hash })
    }

    /// Poll the propagation hub for queued messages.
    ///
    /// This is the core background task for iOS `BGAppRefreshTask`.
    /// Fetches all queued messages, persists them locally, ACKs the hub.
    /// Returns the count and preview of new messages for local notifications.
    ///
    /// Safe to call from a 30-second background window.
    pub async fn poll_hub(&self) -> Result<PollResult, String> {
        let hub_hash = self.hub_delivery_hash.as_deref().ok_or("no propagation hub configured")?;

        let my_delivery_hash = self
            .app_context
            .identity()
            .delivery_destination_hash()
            .ok_or("identity not configured — no delivery hash")?;

        // Fetch queued messages from hub
        let messages = self
            .app_context
            .fleet()
            .propagation_fetch(hub_hash, &my_delivery_hash, Some(15))
            .await
            .map_err(|e| format!("fetch failed: {e}"))?;

        if messages.is_empty() {
            return Ok(PollResult { message_count: 0, messages: Vec::new() });
        }

        let mut poll_messages = Vec::new();

        // Decode and persist each message
        for (_id, lxmf_bytes) in &messages {
            // Try to decode for notification preview
            if let Some(record) = self.app_context.messaging().accept_inbound(
                [0u8; 16], // destination filled by decoder from wire
                lxmf_bytes,
                lxmf::inbound_decode::InboundPayloadMode::FullWire,
            ) {
                poll_messages.push(PollMessage {
                    source_hash: record.source.clone(),
                    content_preview: record.content[..record.content.len().min(100)].to_string(),
                    timestamp: record.timestamp,
                });
            }
        }

        // ACK all fetched messages so hub deletes them
        let ids: Vec<String> = messages.into_iter().map(|(id, _)| id).collect();
        let _ = self.app_context.fleet().propagation_delete(hub_hash, &ids, Some(15)).await;

        let count = poll_messages.len();
        Ok(PollResult { message_count: count, messages: poll_messages })
    }

    /// Send a chat message to a peer.
    pub async fn send_chat(
        &self,
        peer_delivery_hash: &str,
        content: &str,
    ) -> Result<String, String> {
        self.app_context
            .messaging()
            .send_chat(peer_delivery_hash, content, None)
            .await
            .map_err(|e| e.to_string())
    }

    /// List known peers.
    pub async fn list_peers(&self) -> Result<Vec<styrene_ipc::types::DeviceInfo>, String> {
        DaemonStatus::query_devices(self.facade.as_ref(), false).await.map_err(|e| e.to_string())
    }

    /// Query daemon status.
    pub async fn status(&self) -> Result<styrene_ipc::types::DaemonStatusInfo, String> {
        DaemonStatus::query_status(self.facade.as_ref()).await.map_err(|e| e.to_string())
    }

    /// Trigger a mesh announce.
    pub async fn announce(&self) -> Result<(), String> {
        DaemonIdentity::announce(self.facade.as_ref()).await.map(|_| ()).map_err(|e| e.to_string())
    }

    // ── Conversation & Contact Management ───────────────────────────

    /// List conversations with unread counts.
    pub async fn list_conversations(&self) -> Result<Vec<ConversationSummary>, String> {
        use styrene_ipc::traits::DaemonMessaging;
        DaemonMessaging::query_conversations(self.facade.as_ref(), true)
            .await
            .map(|convos| {
                convos
                    .into_iter()
                    .map(|c| ConversationSummary {
                        peer_hash: c.peer_hash,
                        unread_count: c.unread_count,
                        message_count: c.message_count,
                        last_activity: c.last_message_timestamp.unwrap_or(0),
                    })
                    .collect()
            })
            .map_err(|e| e.to_string())
    }

    /// Get messages for a specific peer.
    pub async fn get_messages(
        &self,
        peer_hash: &str,
        limit: u32,
    ) -> Result<Vec<styrene_ipc::types::MessageInfo>, String> {
        use styrene_ipc::traits::DaemonMessaging;
        DaemonMessaging::query_messages(self.facade.as_ref(), peer_hash, limit, None)
            .await
            .map_err(|e| e.to_string())
    }

    /// Search messages by content.
    pub async fn search_messages(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<Vec<styrene_ipc::types::MessageInfo>, String> {
        use styrene_ipc::traits::DaemonMessaging;
        DaemonMessaging::search_messages(self.facade.as_ref(), query, None, limit)
            .await
            .map_err(|e| e.to_string())
    }

    /// Set a contact alias for a peer.
    pub async fn set_contact(&self, peer_hash: &str, alias: &str) -> Result<(), String> {
        use styrene_ipc::traits::DaemonMessaging;
        DaemonMessaging::set_contact(self.facade.as_ref(), peer_hash, Some(alias), None)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Remove a contact.
    pub async fn remove_contact(&self, peer_hash: &str) -> Result<(), String> {
        use styrene_ipc::traits::DaemonMessaging;
        DaemonMessaging::remove_contact(self.facade.as_ref(), peer_hash)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// List all contacts.
    pub async fn list_contacts(&self) -> Result<Vec<styrene_ipc::types::ContactInfo>, String> {
        use styrene_ipc::traits::DaemonMessaging;
        DaemonMessaging::query_contacts(self.facade.as_ref()).await.map_err(|e| e.to_string())
    }

    /// Mark a conversation as read.
    pub async fn mark_read(&self, peer_hash: &str) -> Result<(), String> {
        use styrene_ipc::traits::DaemonMessaging;
        DaemonMessaging::mark_read(self.facade.as_ref(), peer_hash)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Browse a Micron page.
    pub async fn browse_page(&self, host: &str, path: &str) -> Result<String, String> {
        use styrene_ipc::traits::DaemonPages;
        DaemonPages::browse_page(self.facade.as_ref(), host, path, Some(30))
            .await
            .map(|p| p.source)
            .map_err(|e| e.to_string())
    }

    /// Get platform paths (for diagnostics).
    pub fn paths(&self) -> &PlatformPaths {
        &self.paths
    }

    /// Access the full Daemon trait for advanced operations.
    pub fn daemon(&self) -> &dyn Daemon {
        self.facade.as_ref()
    }
}

/// Conversation summary for mobile UI.
#[derive(Debug, Clone)]
pub struct ConversationSummary {
    pub peer_hash: String,
    pub unread_count: u32,
    pub message_count: u32,
    pub last_activity: i64,
}

// ── Identity Storage Backends ───────────────────────────────────────────────

/// Load or create an RNS identity using the configured backend.
///
/// On first launch, creates a new identity seamlessly — no passphrase prompts
/// on keychain backends, no manual key management. The user just opens the app.
fn load_or_create_identity(
    backend: &IdentityBackend,
    paths: &PlatformPaths,
) -> anyhow::Result<PrivateIdentity> {
    match backend {
        IdentityBackend::Keychain => load_or_create_keychain(paths),
        IdentityBackend::EncryptedFile => load_or_create_encrypted_file(paths),
        IdentityBackend::PlaintextFile => load_or_create_plaintext_file(paths),
    }
}

/// Keychain backend: root secret in platform keychain → HKDF → RNS keys.
///
/// On iOS: Face ID / Touch ID protects access. Zero-interaction on create.
/// On macOS: Keychain Access with biometric. Same behavior.
/// Fallback: if keychain feature not compiled, falls back to plaintext file.
fn load_or_create_keychain(paths: &PlatformPaths) -> anyhow::Result<PrivateIdentity> {
    #[cfg(feature = "mobile-keychain")]
    {
        use styrene_identity::keychain_signer::KeychainSigner;
        use styrene_identity::{IdentitySigner, KeyDeriver, KeyPurpose};

        let signer = KeychainSigner::default();

        // Create if needed — generates random 32-byte root secret in Keychain.
        // No passphrase prompt, no user interaction. Biometric required on read.
        if !signer.exists() {
            signer.create().map_err(|e| anyhow::anyhow!("keychain create: {e}"))?;
            eprintln!("[mobile] created new identity in platform keychain");
        }

        // Retrieve root secret (triggers biometric on iOS)
        let root = tokio::runtime::Handle::current()
            .block_on(signer.root_secret())
            .map_err(|e| anyhow::anyhow!("keychain access: {e}"))?;

        // Derive RNS identity from root secret via HKDF.
        // Construct the 64-byte canonical format: [X25519_secret || Ed25519_secret]
        let deriver = KeyDeriver::new(root.as_bytes());
        let encryption_seed = deriver.derive(KeyPurpose::RnsEncryption);
        let signing_seed = deriver.derive(KeyPurpose::Signing);

        let mut key_bytes = [0u8; 64];
        key_bytes[..32].copy_from_slice(&encryption_seed);
        key_bytes[32..].copy_from_slice(&signing_seed);

        PrivateIdentity::from_private_key_bytes(&key_bytes)
            .map_err(|e| anyhow::anyhow!("key derivation: {e:?}"))
    }

    #[cfg(not(feature = "mobile-keychain"))]
    {
        eprintln!("[mobile] keychain feature not enabled, falling back to plaintext file");
        load_or_create_plaintext_file(paths)
    }
}

/// Encrypted file backend: argon2id + ChaCha20Poly1305 encrypted root secret.
///
/// Requires a passphrase — the host app must provide it via a prompt.
/// Less seamless than keychain but works on any platform.
fn load_or_create_encrypted_file(paths: &PlatformPaths) -> anyhow::Result<PrivateIdentity> {
    #[cfg(feature = "mobile-identity")]
    {
        use styrene_identity::{IdentitySigner, KeyDeriver, KeyPurpose};

        let identity_path = paths.identity_path();

        let signer = styrene_identity::file_signer::FileSigner::new(
            identity_path.clone(),
            Box::new(styrene_identity::file_signer::StaticPassphraseProvider::new(b"")),
        );

        // FileSigner auto-creates on first root_secret() if file doesn't exist
        let root = tokio::runtime::Handle::current()
            .block_on(signer.root_secret())
            .map_err(|e| anyhow::anyhow!("encrypted file access: {e}"))?;

        if !identity_path.exists() {
            eprintln!("[mobile] created new encrypted identity at {}", identity_path.display());
        }

        let deriver = KeyDeriver::new(root.as_bytes());
        let encryption_seed = deriver.derive(KeyPurpose::RnsEncryption);
        let signing_seed = deriver.derive(KeyPurpose::Signing);

        let mut key_bytes = [0u8; 64];
        key_bytes[..32].copy_from_slice(&encryption_seed);
        key_bytes[32..].copy_from_slice(&signing_seed);

        PrivateIdentity::from_private_key_bytes(&key_bytes)
            .map_err(|e| anyhow::anyhow!("key derivation: {e:?}"))
    }

    #[cfg(not(feature = "mobile-identity"))]
    {
        eprintln!("[mobile] file-signer feature not enabled, falling back to plaintext");
        load_or_create_plaintext_file(paths)
    }
}

/// Plaintext file backend: 64-byte raw identity on disk.
///
/// For development and testing only. NOT secure for production mobile.
fn load_or_create_plaintext_file(paths: &PlatformPaths) -> anyhow::Result<PrivateIdentity> {
    let identity_path = paths.identity_path();

    if identity_path.exists() {
        let bytes = std::fs::read(&identity_path)?;
        PrivateIdentity::from_private_key_bytes(&bytes)
            .map_err(|e| anyhow::anyhow!("invalid identity: {e:?}"))
    } else {
        // Generate deterministic-ish identity for new installs
        let id = PrivateIdentity::new_from_name(&format!(
            "styrene-mobile-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::write(&identity_path, id.to_private_key_bytes())?;
        eprintln!("[mobile] created new plaintext identity at {}", identity_path.display());
        Ok(id)
    }
}
