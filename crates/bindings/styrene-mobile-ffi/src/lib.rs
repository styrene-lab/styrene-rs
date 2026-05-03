//! UniFFI bindings for styrene-rs mobile embedding.
//!
//! Exposes `MobileNode` to Kotlin (Android) and Swift (iOS) via UniFFI.
//! The host app boots a node in-process and calls methods directly.
//!
//! Generate bindings:
//!   cargo run -p uniffi-bindgen -- generate \
//!     --library target/debug/libstyrene_mobile_ffi.a \
//!     --language kotlin --out-dir out/

use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

uniffi::setup_scaffolding!();

// ── Error Type ──────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum MobileError {
    #[error("boot failed: {msg}")]
    Boot { msg: String },
    #[error("not connected: {msg}")]
    NotConnected { msg: String },
    #[error("hub unavailable: {msg}")]
    HubUnavailable { msg: String },
    #[error("send failed: {msg}")]
    SendFailed { msg: String },
    #[error("internal error: {msg}")]
    Internal { msg: String },
}

// ── Data Transfer Types ─────────────────────────────────────────────────────

#[derive(uniffi::Record)]
pub struct MobileConfig {
    pub config_dir: String,
    pub data_dir: String,
    pub hub_address: Option<String>,
    pub hub_delivery_hash: Option<String>,
    /// Display name for the node (shown in announces).
    pub display_name: Option<String>,
    /// Identity backend: "keychain" (default), "encrypted_file", "plaintext_file"
    pub identity_backend: Option<String>,
}

#[derive(uniffi::Record)]
pub struct PollResult {
    pub message_count: u32,
    pub messages: Vec<PollMessage>,
}

#[derive(uniffi::Record)]
pub struct PollMessage {
    pub source_hash: String,
    pub content_preview: String,
    pub timestamp: i64,
}

#[derive(uniffi::Record)]
pub struct NodeStatus {
    pub identity_hash: String,
    pub daemon_version: String,
    pub transport_active: bool,
    pub peer_count: u32,
    pub link_count: u32,
    pub uptime_secs: u64,
}

#[derive(uniffi::Record)]
pub struct PeerInfo {
    pub destination_hash: String,
    pub name: Option<String>,
    pub status: String,
}

#[derive(uniffi::Record)]
pub struct ConversationInfo {
    pub peer_hash: String,
    pub unread_count: u32,
    pub message_count: u32,
    pub last_activity: i64,
}

#[derive(uniffi::Record)]
pub struct MessageEntry {
    pub id: String,
    pub source_hash: String,
    pub destination_hash: String,
    pub content: String,
    pub timestamp: i64,
    pub is_outgoing: bool,
}

#[derive(uniffi::Record)]
pub struct ContactEntry {
    pub peer_hash: String,
    pub alias: Option<String>,
    pub notes: Option<String>,
}

// ── MobileNode ──────────────────────────────────────────────────────────────

#[derive(uniffi::Object)]
pub struct MobileNode {
    inner: Mutex<Option<styrened::mobile::MobileNode>>,
    rt: tokio::runtime::Handle,
}

#[uniffi::export]
impl MobileNode {
    /// Boot the daemon in-process. Call once on app launch.
    #[uniffi::constructor]
    pub fn boot(config: MobileConfig) -> Result<Arc<Self>, MobileError> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| MobileError::Boot { msg: format!("runtime: {e}") })?;

        let handle = rt.handle().clone();

        let identity_backend = match config.identity_backend.as_deref() {
            Some("encrypted_file") => styrened::mobile::IdentityBackend::EncryptedFile,
            Some("plaintext_file") => styrened::mobile::IdentityBackend::PlaintextFile,
            _ => styrened::mobile::IdentityBackend::Keychain, // default
        };

        let inner_config = styrened::mobile::MobileConfig {
            config_dir: PathBuf::from(&config.config_dir),
            data_dir: PathBuf::from(&config.data_dir),
            hub_address: config.hub_address,
            hub_delivery_hash: config.hub_delivery_hash,
            display_name: config.display_name,
            identity_backend,
        };

        let node = rt
            .block_on(styrened::mobile::MobileNode::boot(inner_config))
            .map_err(|e| MobileError::Boot { msg: e.to_string() })?;

        // Keep runtime alive for process lifetime
        std::mem::forget(rt);

        Ok(Arc::new(Self { inner: Mutex::new(Some(node)), rt: handle }))
    }

    /// Poll the propagation hub for queued messages.
    pub fn poll_hub(&self) -> Result<PollResult, MobileError> {
        self.rt.block_on(async {
            let guard = self.inner.lock().await;
            let node =
                guard.as_ref().ok_or(MobileError::NotConnected { msg: "node shut down".into() })?;

            let result =
                node.poll_hub().await.map_err(|e| MobileError::HubUnavailable { msg: e })?;

            Ok(PollResult {
                message_count: result.message_count as u32,
                messages: result
                    .messages
                    .into_iter()
                    .map(|m| PollMessage {
                        source_hash: m.source_hash,
                        content_preview: m.content_preview,
                        timestamp: m.timestamp,
                    })
                    .collect(),
            })
        })
    }

    /// Send a chat message to a peer.
    pub fn send_chat(&self, peer_hash: String, content: String) -> Result<String, MobileError> {
        self.rt.block_on(async {
            let guard = self.inner.lock().await;
            let node =
                guard.as_ref().ok_or(MobileError::NotConnected { msg: "node shut down".into() })?;

            node.send_chat(&peer_hash, &content)
                .await
                .map_err(|e| MobileError::SendFailed { msg: e })
        })
    }

    /// Trigger a mesh announce.
    pub fn announce(&self) -> Result<(), MobileError> {
        self.rt.block_on(async {
            let guard = self.inner.lock().await;
            let node =
                guard.as_ref().ok_or(MobileError::NotConnected { msg: "node shut down".into() })?;

            node.announce().await.map_err(|e| MobileError::Internal { msg: e })
        })
    }

    /// Get current node status.
    pub fn status(&self) -> Result<NodeStatus, MobileError> {
        self.rt.block_on(async {
            let guard = self.inner.lock().await;
            let node =
                guard.as_ref().ok_or(MobileError::NotConnected { msg: "node shut down".into() })?;

            let id_hash = node.app_context.identity().identity_hash().to_string();
            let s = node.status().await.map_err(|e| MobileError::Internal { msg: e })?;

            Ok(NodeStatus {
                identity_hash: id_hash,
                daemon_version: s.daemon_version,
                transport_active: s.transport_enabled,
                peer_count: s.device_count,
                link_count: s.active_links,
                uptime_secs: s.uptime,
            })
        })
    }

    /// List known peers.
    pub fn list_peers(&self) -> Result<Vec<PeerInfo>, MobileError> {
        self.rt.block_on(async {
            let guard = self.inner.lock().await;
            let node =
                guard.as_ref().ok_or(MobileError::NotConnected { msg: "node shut down".into() })?;

            let devices = node.list_peers().await.map_err(|e| MobileError::Internal { msg: e })?;

            Ok(devices
                .into_iter()
                .map(|d| PeerInfo {
                    destination_hash: d.destination_hash,
                    name: if d.name.is_empty() { None } else { Some(d.name) },
                    status: d.status,
                })
                .collect())
        })
    }

    /// Get the local node's identity hash.
    pub fn identity_hash(&self) -> String {
        self.rt.block_on(async {
            let guard = self.inner.lock().await;
            guard
                .as_ref()
                .map(|n| n.app_context.identity().identity_hash().to_string())
                .unwrap_or_default()
        })
    }

    // ── Conversations & Contacts ──────────────────────────────────

    /// List conversations with unread counts.
    pub fn list_conversations(&self) -> Result<Vec<ConversationInfo>, MobileError> {
        self.rt.block_on(async {
            let guard = self.inner.lock().await;
            let node =
                guard.as_ref().ok_or(MobileError::NotConnected { msg: "node shut down".into() })?;
            let convos =
                node.list_conversations().await.map_err(|e| MobileError::Internal { msg: e })?;
            Ok(convos
                .into_iter()
                .map(|c| ConversationInfo {
                    peer_hash: c.peer_hash,
                    unread_count: c.unread_count,
                    message_count: c.message_count,
                    last_activity: c.last_activity,
                })
                .collect())
        })
    }

    /// Get messages for a peer (most recent first).
    pub fn get_messages(
        &self,
        peer_hash: String,
        limit: u32,
    ) -> Result<Vec<MessageEntry>, MobileError> {
        self.rt.block_on(async {
            let guard = self.inner.lock().await;
            let node =
                guard.as_ref().ok_or(MobileError::NotConnected { msg: "node shut down".into() })?;
            let msgs = node
                .get_messages(&peer_hash, limit)
                .await
                .map_err(|e| MobileError::Internal { msg: e })?;
            Ok(msgs
                .into_iter()
                .map(|m| MessageEntry {
                    id: m.id,
                    source_hash: m.source_hash,
                    destination_hash: m.destination_hash,
                    content: m.content,
                    timestamp: m.timestamp,
                    is_outgoing: m.is_outgoing,
                })
                .collect())
        })
    }

    /// Set a contact alias.
    pub fn set_contact(&self, peer_hash: String, alias: String) -> Result<(), MobileError> {
        self.rt.block_on(async {
            let guard = self.inner.lock().await;
            let node =
                guard.as_ref().ok_or(MobileError::NotConnected { msg: "node shut down".into() })?;
            node.set_contact(&peer_hash, &alias).await.map_err(|e| MobileError::Internal { msg: e })
        })
    }

    /// List contacts.
    pub fn list_contacts(&self) -> Result<Vec<ContactEntry>, MobileError> {
        self.rt.block_on(async {
            let guard = self.inner.lock().await;
            let node =
                guard.as_ref().ok_or(MobileError::NotConnected { msg: "node shut down".into() })?;
            let contacts =
                node.list_contacts().await.map_err(|e| MobileError::Internal { msg: e })?;
            Ok(contacts
                .into_iter()
                .map(|c| ContactEntry {
                    peer_hash: c.peer_hash,
                    alias: c.alias.filter(|s| !s.is_empty()),
                    notes: c.notes.filter(|s| !s.is_empty()),
                })
                .collect())
        })
    }

    /// Mark conversation as read.
    pub fn mark_read(&self, peer_hash: String) -> Result<(), MobileError> {
        self.rt.block_on(async {
            let guard = self.inner.lock().await;
            let node =
                guard.as_ref().ok_or(MobileError::NotConnected { msg: "node shut down".into() })?;
            node.mark_read(&peer_hash).await.map_err(|e| MobileError::Internal { msg: e })
        })
    }

    /// Browse a Micron page from a peer.
    pub fn browse_page(&self, host: String, path: String) -> Result<String, MobileError> {
        self.rt.block_on(async {
            let guard = self.inner.lock().await;
            let node =
                guard.as_ref().ok_or(MobileError::NotConnected { msg: "node shut down".into() })?;
            node.browse_page(&host, &path).await.map_err(|e| MobileError::Internal { msg: e })
        })
    }

    /// Get the local node's delivery hash (share this with contacts).
    pub fn delivery_hash(&self) -> Option<String> {
        self.rt.block_on(async {
            let guard = self.inner.lock().await;
            guard.as_ref().and_then(|n| n.app_context.identity().delivery_destination_hash())
        })
    }

    /// Check if transport is connected to the hub.
    pub fn is_connected(&self) -> bool {
        self.rt.block_on(async {
            let guard = self.inner.lock().await;
            guard.as_ref().map(|n| n.app_context.transport().is_connected()).unwrap_or(false)
        })
    }

    /// Shut down the node. Call on app termination.
    pub fn shutdown(&self) {
        self.rt.block_on(async {
            let mut guard = self.inner.lock().await;
            *guard = None;
        });
    }
}
