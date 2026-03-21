//! DaemonFacade — thin Daemon trait implementation with auth enforcement.
//!
//! The IPC-facing dispatch layer. Holds `Arc<AppContext>` and delegates
//! to services after checking auth via `AuthService::check()`.
//!
//! **Call direction**: IPC → DaemonFacade → AuthService.check() → Service → storage/transport.
//! Services never call DaemonFacade. Services access each other through AppContext accessors.
//!
//! Package I — see ownership-matrix.md §DaemonFacade.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::broadcast;

use styrene_ipc::error::IpcError;
use styrene_ipc::traits::*;
use styrene_ipc::types::*;

use crate::app_context::AppContext;
use crate::services::{AutoReplyMode, Capability};
use crate::storage::messages::MessageRecord;

/// Convert an io::Error to IpcError::Internal.
fn internal(e: std::io::Error) -> IpcError {
    IpcError::Internal { message: e.to_string() }
}

/// Convert a MessageRecord to a MessageInfo IPC type.
fn record_to_message_info(r: MessageRecord) -> MessageInfo {
    let mut info = MessageInfo::default();
    info.id = r.id;
    info.source_hash = r.source;
    info.destination_hash = r.destination;
    info.timestamp = r.timestamp;
    info.content = r.content;
    info.title = if r.title.is_empty() { None } else { Some(r.title) };
    info.status = r.receipt_status.unwrap_or_default();
    info.is_outgoing = r.direction == "out";
    info.read = r.read;
    info
}

/// Thin IPC-facing facade implementing the `Daemon` composite trait.
///
/// - Checks RBAC via `auth.check(caller, capability)` before every delegation
/// - Delegates to the appropriate service through AppContext
/// - Maps service errors to IpcError
///
/// Replaces `RpcDaemon` as the IPC-facing type. `StubDaemon` (in `styrene-ipc`)
/// remains available for frontend testing without daemon infrastructure.
pub struct DaemonFacade {
    ctx: Arc<AppContext>,
    /// The identity hash of the IPC caller (for auth checks).
    /// In production, this comes from the Unix socket peer credentials
    /// or the authenticated TLS client identity.
    /// For local IPC (same machine), this is typically the daemon's own identity.
    caller_identity: String,
}

impl DaemonFacade {
    /// Create a new facade wrapping the given AppContext.
    ///
    /// `caller_identity` is the authenticated identity of the IPC peer.
    /// For local connections, pass the daemon's own identity hash.
    pub fn new(ctx: Arc<AppContext>, caller_identity: String) -> Self {
        Self {
            ctx,
            caller_identity,
        }
    }

    /// Check a capability and return IpcError if denied.
    fn require(&self, capability: &Capability) -> Result<(), IpcError> {
        if self.ctx.auth().check(&self.caller_identity, capability) {
            Ok(())
        } else {
            Err(IpcError::Unavailable {
                reason: format!("permission denied for {:?}", capability),
            })
        }
    }

    fn not_implemented(method: &str) -> IpcError {
        IpcError::not_implemented(method)
    }
}

#[async_trait]
impl DaemonIdentity for DaemonFacade {
    async fn query_identity(&self) -> Result<IdentityInfo, IpcError> {
        self.require(&Capability::Status)?;
        let svc = self.ctx.identity();
        let dest = svc.delivery_destination_hash().unwrap_or_default();
        let mut info = IdentityInfo::default();
        info.identity_hash = svc.identity_hash().to_string();
        info.destination_hash = dest.clone();
        info.lxmf_destination_hash = dest;
        info.display_name = svc.display_name().unwrap_or_default();
        info.icon = svc.icon();
        info.short_name = svc.short_name();
        Ok(info)
    }

    async fn set_identity(
        &self,
        display_name: Option<&str>,
        icon: Option<&str>,
        short_name: Option<&str>,
    ) -> Result<bool, IpcError> {
        self.require(&Capability::Status)?;
        let changed = self.ctx.identity().set_identity(display_name, icon, short_name);
        if changed {
            // Re-announce with updated identity
            self.ctx.identity().announce(None).await;
        }
        Ok(changed)
    }

    async fn announce(&self) -> Result<bool, IpcError> {
        self.require(&Capability::Status)?;
        self.ctx.identity().announce(None).await;
        Ok(true)
    }
}

#[async_trait]
impl DaemonMessaging for DaemonFacade {
    async fn send_chat(&self, request: SendChatRequest) -> Result<MessageId, IpcError> {
        self.require(&Capability::Chat)?;
        self.ctx
            .messaging()
            .send_chat(
                &request.peer_hash,
                &request.content,
                request.title.as_deref(),
            )
            .await
            .map_err(internal)
    }

    async fn mark_read(&self, peer_hash: &str) -> Result<u64, IpcError> {
        self.require(&Capability::Chat)?;
        self.ctx.messaging().mark_read(peer_hash).map_err(internal)
    }

    async fn delete_conversation(&self, peer_hash: &str) -> Result<u64, IpcError> {
        self.require(&Capability::Chat)?;
        self.ctx.messaging().delete_conversation(peer_hash).map_err(internal)
    }

    async fn delete_message(&self, message_id: &str) -> Result<bool, IpcError> {
        self.require(&Capability::Chat)?;
        self.ctx.messaging().delete_message(message_id).map_err(internal)
    }

    async fn retry_message(&self, message_id: &str) -> Result<bool, IpcError> {
        self.require(&Capability::Chat)?;
        // Look up the original message, re-deliver if outbound and failed
        let msg = self
            .ctx
            .messaging()
            .get_message(message_id)
            .map_err(internal)?
            .ok_or_else(|| IpcError::not_found("message", message_id))?;
        if msg.direction != "out" {
            return Err(IpcError::invalid_request("can only retry outbound messages"));
        }
        // Re-send via the delivery pipeline
        let _new_id = self
            .ctx
            .messaging()
            .send_chat(&msg.destination, &msg.content, Some(&msg.title))
            .await
            .map_err(internal)?;
        Ok(true)
    }

    async fn query_conversations(
        &self,
        include_unread: bool,
    ) -> Result<Vec<ConversationInfo>, IpcError> {
        self.require(&Capability::Status)?;
        let summaries = self.ctx.messaging().list_conversations(include_unread).map_err(internal)?;
        Ok(summaries
            .into_iter()
            .map(|s| {
                let mut info = ConversationInfo::default();
                info.peer_hash = s.peer_hash;
                info.peer_name = s.peer_name;
                info.last_message_timestamp = s.last_message_timestamp;
                info.last_message_content = s.last_message_content;
                info.unread_count = s.unread_count;
                info.message_count = s.message_count;
                info
            })
            .collect())
    }

    async fn query_messages(
        &self,
        peer_hash: &str,
        limit: u32,
        before_ts: Option<i64>,
    ) -> Result<Vec<MessageInfo>, IpcError> {
        self.require(&Capability::Status)?;
        let records = self
            .ctx
            .messaging()
            .list_messages_for_peer(peer_hash, limit as usize, before_ts)
            .map_err(internal)?;
        Ok(records.into_iter().map(record_to_message_info).collect())
    }

    async fn search_messages(
        &self,
        query: &str,
        peer_hash: Option<&str>,
        limit: u32,
    ) -> Result<Vec<MessageInfo>, IpcError> {
        self.require(&Capability::Status)?;
        let records = self
            .ctx
            .messaging()
            .search_messages(query, peer_hash, limit as usize)
            .map_err(internal)?;
        Ok(records.into_iter().map(record_to_message_info).collect())
    }

    async fn query_attachment(&self, _message_id: &str) -> Result<Vec<u8>, IpcError> {
        self.require(&Capability::Status)?;
        Err(Self::not_implemented("query_attachment")) // Needs attachment storage
    }

    async fn set_contact(
        &self,
        peer_hash: &str,
        alias: Option<&str>,
        notes: Option<&str>,
    ) -> Result<ContactInfo, IpcError> {
        self.require(&Capability::Chat)?;
        let c = self.ctx.messaging().set_contact(peer_hash, alias, notes).map_err(internal)?;
        let mut info = ContactInfo::default();
        info.peer_hash = c.peer_hash;
        info.alias = c.alias;
        info.notes = c.notes;
        info.created_at = Some(c.created_at);
        info.updated_at = Some(c.updated_at);
        Ok(info)
    }

    async fn remove_contact(&self, peer_hash: &str) -> Result<bool, IpcError> {
        self.require(&Capability::Chat)?;
        self.ctx.messaging().remove_contact(peer_hash).map_err(internal)
    }

    async fn query_contacts(&self) -> Result<Vec<ContactInfo>, IpcError> {
        self.require(&Capability::Status)?;
        let contacts = self.ctx.messaging().list_contacts().map_err(internal)?;
        Ok(contacts
            .into_iter()
            .map(|c| {
                let mut info = ContactInfo::default();
                info.peer_hash = c.peer_hash;
                info.alias = c.alias;
                info.notes = c.notes;
                info.created_at = Some(c.created_at);
                info.updated_at = Some(c.updated_at);
                info
            })
            .collect())
    }

    async fn resolve_name(
        &self,
        name: &str,
        prefix: Option<&str>,
    ) -> Result<Option<PeerHash>, IpcError> {
        self.require(&Capability::Status)?;
        Ok(self.ctx.discovery().resolve_name(name, prefix))
    }
}

#[async_trait]
impl DaemonStatus for DaemonFacade {
    async fn query_status(&self) -> Result<DaemonStatusInfo, IpcError> {
        self.require(&Capability::Status)?;
        let status = self.ctx.status();
        let mut info = DaemonStatusInfo::default();
        info.uptime = status.uptime_secs();
        info.daemon_version = env!("CARGO_PKG_VERSION").to_string();
        info.rns_initialized = self.ctx.transport().is_connected();
        info.lxmf_initialized = self.ctx.transport().is_connected();
        info.device_count = self.ctx.discovery().peer_count() as u32;
        info.interface_count = status.interface_count() as u32;
        info.propagation_enabled = status.propagation_enabled();
        info.transport_enabled = self.ctx.transport().is_connected();
        Ok(info)
    }

    async fn query_config(&self) -> Result<ConfigSnapshot, IpcError> {
        self.require(&Capability::Status)?;
        // Return a minimal snapshot. Full config mapping is follow-on work.
        Ok(ConfigSnapshot::default())
    }

    async fn query_devices(&self, _styrene_only: bool) -> Result<Vec<DeviceInfo>, IpcError> {
        self.require(&Capability::Status)?;
        let announces = self
            .ctx
            .discovery()
            .list_announces(500)
            .map_err(|e| IpcError::Internal {
                message: e.to_string(),
            })?;
        Ok(announces
            .into_iter()
            .map(|a| {
                let mut d = DeviceInfo::default();
                d.destination_hash = a.peer.clone();
                d.identity_hash = a.peer;
                d.name = a.name.unwrap_or_default();
                d.device_type = "unknown".into();
                d.status = "announced".into();
                d.is_styrene_node = !a.capabilities.is_empty();
                d.last_announce = Some(a.timestamp);
                d.announce_count = a.seen_count as u32;
                d
            })
            .collect())
    }

    async fn query_path_info(&self, dest_hash: &str) -> Result<PathInfo, IpcError> {
        self.require(&Capability::Status)?;
        let dest_bytes: [u8; 16] = hex::decode(dest_hash)
            .map_err(|e| IpcError::invalid_request(format!("invalid hash: {e}")))?
            .try_into()
            .map_err(|_| IpcError::invalid_request("hash must be 16 bytes"))?;
        let dest = rns_core::hash::AddressHash::new(dest_bytes);

        let path = self.ctx.transport().query_path(&dest).await;
        let mut info = PathInfo::default();
        info.destination_hash = dest_hash.to_string();
        if let Some((hops, iface)) = path {
            info.hops = Some(hops as u32);
            info.interface = Some(hex::encode(iface.as_slice()));
        }
        Ok(info)
    }

    async fn query_auto_reply(&self) -> Result<AutoReplyConfig, IpcError> {
        self.require(&Capability::Status)?;
        let config = self.ctx.auto_reply().config();
        let mut ar = AutoReplyConfig::default();
        ar.mode = match config.mode {
            AutoReplyMode::Disabled => "disabled".into(),
            AutoReplyMode::All => "all".into(),
            AutoReplyMode::FirstOnly => "first_only".into(),
        };
        ar.message = if config.message.is_empty() {
            None
        } else {
            Some(config.message)
        };
        ar.cooldown_secs = Some(config.cooldown.as_secs());
        Ok(ar)
    }

    async fn set_auto_reply(
        &self,
        mode: &str,
        message: Option<&str>,
        cooldown_secs: Option<u64>,
    ) -> Result<bool, IpcError> {
        self.require(&Capability::Status)?;
        let auto_reply_mode = match mode {
            "disabled" | "off" => AutoReplyMode::Disabled,
            "all" => AutoReplyMode::All,
            "first_only" | "first" => AutoReplyMode::FirstOnly,
            _ => {
                return Err(IpcError::InvalidRequest {
                    message: format!("unknown auto-reply mode: {mode}"),
                });
            }
        };
        self.ctx.auto_reply().set_config(
            crate::services::auto_reply::AutoReplyConfig {
                mode: auto_reply_mode,
                message: message.unwrap_or_default().to_string(),
                cooldown: std::time::Duration::from_secs(cooldown_secs.unwrap_or(300)),
            },
        );
        Ok(true)
    }
}

#[async_trait]
impl DaemonFleet for DaemonFacade {
    async fn device_status(
        &self,
        dest: &str,
        timeout: Option<u64>,
    ) -> Result<RemoteStatusInfo, IpcError> {
        self.require(&Capability::Status)?;
        self.ctx.fleet().device_status(dest, timeout).await.map_err(internal)
    }

    async fn exec(
        &self,
        dest: &str,
        cmd: &str,
        args: Vec<String>,
        timeout: Option<u64>,
    ) -> Result<ExecResult, IpcError> {
        self.require(&Capability::Exec)?;
        self.ctx.fleet().exec(dest, cmd, &args, timeout).await.map_err(internal)
    }

    async fn reboot_device(
        &self,
        dest: &str,
        delay: Option<u64>,
        timeout: Option<u64>,
    ) -> Result<RebootResult, IpcError> {
        self.require(&Capability::Reboot)?;
        self.ctx.fleet().reboot_device(dest, delay, timeout).await.map_err(internal)
    }

    async fn self_update(
        &self,
        _dest: &str,
        _version: Option<&str>,
        _timeout: Option<u64>,
    ) -> Result<SelfUpdateResult, IpcError> {
        self.require(&Capability::UpdateConfig)?;
        Err(Self::not_implemented("self_update"))
    }

    async fn remote_inbox(
        &self,
        dest: &str,
        limit: u32,
        timeout: Option<u64>,
    ) -> Result<Vec<ConversationInfo>, IpcError> {
        self.require(&Capability::Status)?;
        self.ctx.fleet().remote_inbox(dest, limit, timeout).await.map_err(internal)
    }

    async fn remote_messages(
        &self,
        dest: &str,
        peer_hash: &str,
        limit: u32,
        timeout: Option<u64>,
    ) -> Result<Vec<MessageInfo>, IpcError> {
        self.require(&Capability::Status)?;
        self.ctx.fleet().remote_messages(dest, peer_hash, limit, timeout).await.map_err(internal)
    }

    async fn terminal_open(&self, _request: TerminalOpenRequest) -> Result<SessionId, IpcError> {
        self.require(&Capability::Exec)?;
        Err(Self::not_implemented("terminal_open"))
    }

    async fn terminal_input(&self, _session_id: &str, _data: &[u8]) -> Result<bool, IpcError> {
        self.require(&Capability::Exec)?;
        Err(Self::not_implemented("terminal_input"))
    }

    async fn terminal_resize(
        &self,
        _session_id: &str,
        _rows: u16,
        _cols: u16,
    ) -> Result<bool, IpcError> {
        self.require(&Capability::Exec)?;
        Err(Self::not_implemented("terminal_resize"))
    }

    async fn terminal_close(&self, _session_id: &str) -> Result<bool, IpcError> {
        self.require(&Capability::Exec)?;
        Err(Self::not_implemented("terminal_close"))
    }
}

#[async_trait]
impl DaemonEvents for DaemonFacade {
    async fn subscribe_messages(
        &self,
        peer_hashes: &[String],
    ) -> Result<broadcast::Receiver<DaemonEvent>, IpcError> {
        self.require(&Capability::Status)?;
        Ok(self.ctx.events().subscribe_messages(peer_hashes))
    }

    async fn subscribe_devices(&self) -> Result<broadcast::Receiver<DaemonEvent>, IpcError> {
        self.require(&Capability::Status)?;
        Ok(self.ctx.events().subscribe_devices())
    }
}

#[async_trait]
impl DaemonTunnel for DaemonFacade {
    async fn list_tunnels(&self) -> Result<Vec<TunnelInfo>, IpcError> {
        Err(Self::not_implemented("list_tunnels"))
    }

    async fn tunnel_status(&self, _peer_hash: &str) -> Result<TunnelInfo, IpcError> {
        Err(Self::not_implemented("tunnel_status"))
    }

    async fn tunnel_rekey(&self, _peer_hash: &str) -> Result<bool, IpcError> {
        Err(Self::not_implemented("tunnel_rekey"))
    }

    async fn tunnel_teardown(&self, _peer_hash: &str) -> Result<bool, IpcError> {
        Err(Self::not_implemented("tunnel_teardown"))
    }

    async fn list_tunnel_sas(&self, _peer_hash: &str) -> Result<Vec<TunnelSaInfo>, IpcError> {
        Err(Self::not_implemented("list_tunnel_sas"))
    }
}

// DaemonFacade automatically implements `Daemon` because it implements
// all six sub-traits: DaemonMessaging + DaemonIdentity + DaemonStatus +
// DaemonFleet + DaemonEvents + DaemonTunnel.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::messages::MessagesStore;
    use crate::transport::mesh_transport::MeshTransport;
    use crate::transport::null_transport::NullTransport;
    use std::sync::Mutex;
    use styrene_ipc::traits::Daemon;

    fn make_facade() -> DaemonFacade {
        let transport: Arc<dyn MeshTransport> = Arc::new(NullTransport::new());
        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let ctx = Arc::new(AppContext::new(transport, "test-identity".into(), store));
        DaemonFacade::new(ctx, "test-caller".into())
    }

    #[test]
    fn facade_implements_daemon_trait() {
        let facade = make_facade();
        // Verify it can be used as Arc<dyn Daemon>
        let _: Arc<dyn Daemon> = Arc::new(facade);
    }

    #[tokio::test]
    async fn query_identity_returns_identity_hash() {
        let facade = make_facade();
        let info = facade.query_identity().await.unwrap();
        assert_eq!(info.identity_hash, "test-identity");
    }

    #[tokio::test]
    async fn announce_succeeds() {
        let facade = make_facade();
        let result = facade.announce().await.unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn query_status_returns_basic_info() {
        let facade = make_facade();
        let status = facade.query_status().await.unwrap();
        assert!(!status.rns_initialized); // NullTransport
        assert_eq!(status.device_count, 0);
    }

    #[tokio::test]
    async fn query_auto_reply_returns_disabled() {
        let facade = make_facade();
        let config = facade.query_auto_reply().await.unwrap();
        assert_eq!(config.mode, "disabled");
    }

    #[tokio::test]
    async fn set_auto_reply_updates_config() {
        let facade = make_facade();
        facade
            .set_auto_reply("all", Some("I'm away"), Some(600))
            .await
            .unwrap();
        let config = facade.query_auto_reply().await.unwrap();
        assert_eq!(config.mode, "all");
        assert_eq!(config.message, Some("I'm away".into()));
        assert_eq!(config.cooldown_secs, Some(600));
    }

    #[tokio::test]
    async fn set_auto_reply_invalid_mode_returns_error() {
        let facade = make_facade();
        let result = facade
            .set_auto_reply("bogus", None, None)
            .await;
        assert!(matches!(result, Err(IpcError::InvalidRequest { .. })));
    }

    #[tokio::test]
    async fn not_implemented_methods_return_correct_error() {
        let facade = make_facade();
        // send_chat returns Internal (no transport in test mode), not NotImplemented
        let result = facade.send_chat(SendChatRequest::default()).await;
        assert!(matches!(result, Err(IpcError::Internal { .. })));

        let result = facade.list_tunnels().await;
        assert!(matches!(result, Err(IpcError::NotImplemented { .. })));
    }

    #[tokio::test]
    async fn blocked_caller_gets_denied() {
        let transport: Arc<dyn MeshTransport> = Arc::new(NullTransport::new());
        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let ctx = Arc::new(AppContext::new(transport, "daemon".into(), store));

        // Block the caller
        ctx.auth().block("blocked-caller");

        let facade = DaemonFacade::new(ctx, "blocked-caller".into());
        let result = facade.query_status().await;
        assert!(matches!(result, Err(IpcError::Unavailable { .. })));
    }

    #[tokio::test]
    async fn peer_cannot_exec() {
        let transport: Arc<dyn MeshTransport> = Arc::new(NullTransport::new());
        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let ctx = Arc::new(AppContext::new(transport, "daemon".into(), store));
        // Default role is Peer — can chat/status but not exec
        let facade = DaemonFacade::new(ctx, "peer-caller".into());

        let result = facade.exec("dest", "ls", vec![], None).await;
        assert!(matches!(result, Err(IpcError::Unavailable { .. })));
    }

    #[tokio::test]
    async fn query_devices_returns_announces() {
        let transport: Arc<dyn MeshTransport> = Arc::new(NullTransport::new());
        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let ctx = Arc::new(AppContext::new(transport, "daemon".into(), store));

        // Add some devices through discovery
        ctx.discovery()
            .accept_announce_with_details(
                "node1".into(),
                1000,
                Some("TestNode".into()),
                None,
                None,
            )
            .unwrap();

        let facade = DaemonFacade::new(ctx, "caller".into());
        let devices = facade.query_devices(false).await.unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "TestNode");
    }
}
