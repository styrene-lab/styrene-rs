use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use styrene_ipc::types::{
    ACTIVE_CAPABILITIES_VERSION, ActiveCapabilitiesInfo, ConfigApplyResult, DaemonStatusInfo,
    DeviceInfo, ExecResult, IdentityInfo, MessageInfo, NetworkOperationInfo, PropagationQueueEntry,
    PropagationSnapshot, RebootResult, RemoteStatusInfo, RequestObservationInfo,
    ResourceTransferInfo, StandardPropagationSnapshot, StartNetworkOperationInfo, StartRequestInfo,
};
use tokio::sync::{Mutex, mpsc};

use crate::daemon_bridge::{self, DaemonEvent, InterfaceStats, PathTableEntry};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FixtureId {
    Empty,
    #[default]
    Healthy,
    Degraded,
    HighCardinality,
    ActiveScenario,
    Error,
}

impl FixtureId {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "empty" => Ok(Self::Empty),
            "healthy" => Ok(Self::Healthy),
            "degraded" => Ok(Self::Degraded),
            "high-cardinality" => Ok(Self::HighCardinality),
            "active-scenario" => Ok(Self::ActiveScenario),
            "error" => Ok(Self::Error),
            _ => Err(format!("unknown Styrene DX fixture '{value}'")),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeProfile {
    Live { socket_path: PathBuf },
    Embedded { ephemeral: bool },
    Fixture { fixture: FixtureId },
}

impl RuntimeProfile {
    pub fn from_environment() -> Result<Self, String> {
        Self::from_values(
            std::env::var("STYRENE_DX_PROFILE").ok().as_deref(),
            std::env::var_os("STYRENE_DX_SOCKET").map(PathBuf::from),
            std::env::var("STYRENE_DX_FIXTURE").ok().as_deref(),
        )
    }

    fn from_values(
        profile: Option<&str>,
        socket_path: Option<PathBuf>,
        fixture: Option<&str>,
    ) -> Result<Self, String> {
        match profile.unwrap_or("live") {
            "live" => Ok(Self::Live {
                socket_path: socket_path.unwrap_or_else(styrene_ipc_server::default_socket_path),
            }),
            "embedded" => Ok(Self::Embedded { ephemeral: true }),
            "fixture" => {
                Ok(Self::Fixture { fixture: FixtureId::parse(fixture.unwrap_or("healthy"))? })
            }
            value => Err(format!(
                "unknown STYRENE_DX_PROFILE '{value}'; expected live, embedded, or fixture"
            )),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Live { .. } => "Live",
            Self::Embedded { .. } => "Embedded",
            Self::Fixture { .. } => "Fixture",
        }
    }

    pub fn live(socket_path: impl Into<PathBuf>) -> Result<Self, String> {
        let socket_path = socket_path.into();
        if socket_path.as_os_str().is_empty() {
            return Err("Live profile requires a daemon socket path".into());
        }
        Ok(Self::Live { socket_path })
    }

    pub const fn embedded() -> Self {
        Self::Embedded { ephemeral: true }
    }

    pub const fn fixture(fixture: FixtureId) -> Self {
        Self::Fixture { fixture }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConnectionGeneration(pub u64);

impl ConnectionGeneration {
    pub fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BackendCapabilities {
    pub messaging: bool,
    pub content: bool,
    pub fleet: bool,
    pub propagation: bool,
    pub scenarios: bool,
    pub administration: bool,
}

impl BackendCapabilities {
    pub fn negotiated(
        connected: bool,
        server_generation: Option<u64>,
        active: Option<&ActiveCapabilitiesInfo>,
    ) -> Self {
        let Some(active) = active.filter(|value| {
            connected && server_generation.is_some() && value.version == ACTIVE_CAPABILITIES_VERSION
        }) else {
            return Self::default();
        };
        let degraded = |id: &str| active.degraded.iter().any(|item| item.id == id);
        let authorized =
            |id: &str| !degraded(id) && active.authorized_operations.iter().any(|item| item == id);
        let runtime = |id: &str| !degraded(id) && active.runtime.iter().any(|item| item == id);
        Self {
            messaging: authorized("chat.send"),
            content: authorized("page.browse"),
            fleet: authorized("rpc.status")
                || authorized("rpc.exec")
                || authorized("rpc.reboot")
                || authorized("rpc.fleet_apply"),
            propagation: authorized("rpc.status"),
            scenarios: runtime("fixture.scenarios"),
            administration: authorized("rpc.config_update")
                || authorized("policy.update")
                || authorized("rpc.exec"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConversationInfo {
    pub peer_hash: String,
    pub peer_name: Option<String>,
    pub last_message: Option<String>,
    pub last_timestamp: Option<i64>,
    pub unread_count: u32,
    pub message_count: u32,
    pub pinned: bool,
    pub muted: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BackendPage<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
    pub pagination_supported: bool,
    pub reset: bool,
}

pub struct OpenedSession {
    pub backend: Arc<dyn BackendSession>,
    pub events: mpsc::Receiver<DaemonEvent>,
    pub generation: ConnectionGeneration,
}

#[async_trait]
pub trait BackendSession: Send + Sync {
    fn profile(&self) -> &RuntimeProfile;
    fn diagnostics(&self) -> daemon_bridge::BrokerDiagnostics;

    async fn send_chat_outcome(
        &self,
        request: styrene_ipc::types::SendChatRequest,
    ) -> Result<styrene_ipc::types::SendChatOutcome, String>;
    async fn draft(
        &self,
        peer_hash: &str,
    ) -> Result<Option<styrene_ipc::types::ConversationDraft>, String>;
    async fn save_draft(
        &self,
        peer_hash: &str,
        content: &str,
    ) -> Result<styrene_ipc::types::ConversationDraft, String>;
    async fn discard_draft(&self, peer_hash: &str) -> Result<(), String>;
    async fn retry_message(
        &self,
        message_id: &str,
    ) -> Result<styrene_ipc::types::MessagingOperationOutcome, String>;
    async fn cancel_message(
        &self,
        message_id: &str,
    ) -> Result<styrene_ipc::types::MessagingOperationOutcome, String>;
    async fn browse_page(
        &self,
        host: &str,
        path: &str,
    ) -> Result<daemon_bridge::PageResponse, String>;
    async fn navigate_page(
        &self,
        request: styrene_ipc::types::PageNavigationRequest,
    ) -> Result<daemon_bridge::PageResponse, String> {
        let _ = request;
        Err("authoritative page navigation is unavailable in this backend".into())
    }
    async fn close_page(&self, session_id: &str) -> Result<(), String> {
        let _ = session_id;
        Err("authoritative page close is unavailable in this backend".into())
    }
    async fn start_file_download(
        &self,
        request: styrene_ipc::types::FileDownloadRequest,
    ) -> Result<styrene_ipc::types::FileDownloadInfo, String> {
        let _ = request;
        Err("file downloads are unavailable in this backend".into())
    }
    async fn file_download(
        &self,
        download_id: &str,
    ) -> Result<styrene_ipc::types::FileDownloadInfo, String> {
        let _ = download_id;
        Err("file downloads are unavailable in this backend".into())
    }
    async fn cancel_file_download(
        &self,
        download_id: &str,
    ) -> Result<styrene_ipc::types::FileDownloadInfo, String> {
        let _ = download_id;
        Err("file downloads are unavailable in this backend".into())
    }
    async fn save_file_download(
        &self,
        download_id: &str,
        destination: &str,
    ) -> Result<styrene_ipc::types::FileDownloadInfo, String> {
        let _ = (download_id, destination);
        Err("file downloads are unavailable in this backend".into())
    }
    async fn path_table(&self) -> Result<Vec<PathTableEntry>, String>;
    async fn interface_stats(&self) -> Result<Vec<InterfaceStats>, String>;
    async fn links(&self) -> Result<styrene_ipc::types::LinkSnapshot, String>;
    async fn conversations(&self) -> Result<Vec<ConversationInfo>, String>;
    async fn messages(&self, peer_hash: &str, limit: u32) -> Result<Vec<MessageInfo>, String>;
    async fn message(&self, message_id: &str) -> Result<Option<MessageInfo>, String>;
    async fn conversation_page(
        &self,
        cursor: Option<&str>,
    ) -> Result<BackendPage<ConversationInfo>, String> {
        let _ = cursor;
        Ok(BackendPage {
            items: self.conversations().await?,
            next_cursor: None,
            pagination_supported: false,
            reset: false,
        })
    }
    async fn message_page(
        &self,
        peer_hash: &str,
        cursor: Option<&str>,
    ) -> Result<BackendPage<MessageInfo>, String> {
        let _ = cursor;
        Ok(BackendPage {
            items: self.messages(peer_hash, 50).await?,
            next_cursor: None,
            pagination_supported: false,
            reset: false,
        })
    }
    async fn propagation_snapshot(
        &self,
        cursor: Option<&str>,
    ) -> Result<PropagationSnapshot, String>;
    async fn standard_propagation_snapshot(&self) -> Result<StandardPropagationSnapshot, String>;
    async fn fleet_status(&self, destination: &str) -> Result<RemoteStatusInfo, String>;
    async fn fleet_exec(
        &self,
        destination: &str,
        command: &str,
        args: &[String],
    ) -> Result<ExecResult, String>;
    async fn fleet_reboot(
        &self,
        destination: &str,
        delay: Option<u64>,
    ) -> Result<RebootResult, String>;
    async fn fleet_apply(
        &self,
        destination: &str,
        profile_base64: &str,
    ) -> Result<ConfigApplyResult, String>;
    async fn block_peer(&self, identity_hash: &str) -> Result<(), String>;
    async fn start_network_operation(
        &self,
        request: StartNetworkOperationInfo,
    ) -> Result<NetworkOperationInfo, String>;
    async fn cancel_network_operation(
        &self,
        operation_id: &str,
    ) -> Result<NetworkOperationInfo, String>;
    async fn network_operations(&self) -> Result<Vec<NetworkOperationInfo>, String>;
    async fn start_request(
        &self,
        request: StartRequestInfo,
    ) -> Result<RequestObservationInfo, String>;
    async fn cancel_request(&self, request_id: &str) -> Result<RequestObservationInfo, String>;
    async fn requests(&self) -> Result<Vec<RequestObservationInfo>, String>;
    async fn resources(&self) -> Result<Vec<ResourceTransferInfo>, String>;
    async fn cancel_resource(&self, resource_hash: &str) -> Result<bool, String>;
    async fn shutdown(&self);
}

pub async fn open_session(
    profile: RuntimeProfile,
    generation: ConnectionGeneration,
) -> Result<OpenedSession, String> {
    match profile {
        RuntimeProfile::Live { socket_path } => {
            let (broker, events) = daemon_bridge::connect_ipc(&socket_path, generation).await?;
            Ok(OpenedSession {
                backend: Arc::new(IpcBackend {
                    profile: RuntimeProfile::Live { socket_path },
                    broker,
                    embedded: Mutex::new(None),
                }),
                events,
                generation,
            })
        }
        RuntimeProfile::Embedded { ephemeral } => {
            let (broker, events, handle) =
                daemon_bridge::connect_embedded(ephemeral, generation).await?;
            Ok(OpenedSession {
                backend: Arc::new(IpcBackend {
                    profile: RuntimeProfile::Embedded { ephemeral },
                    broker,
                    embedded: Mutex::new(Some(handle)),
                }),
                events,
                generation,
            })
        }
        RuntimeProfile::Fixture { fixture: FixtureId::Error } => {
            Err("deterministic Fixture error state".into())
        }
        RuntimeProfile::Fixture { fixture } => Ok(open_fixture(fixture, generation)),
    }
}

struct IpcBackend {
    profile: RuntimeProfile,
    broker: daemon_bridge::RequestBroker,
    embedded: Mutex<Option<daemon_bridge::EmbeddedDaemon>>,
}

#[async_trait]
impl BackendSession for IpcBackend {
    fn profile(&self) -> &RuntimeProfile {
        &self.profile
    }

    fn diagnostics(&self) -> daemon_bridge::BrokerDiagnostics {
        self.broker.diagnostics()
    }

    async fn send_chat_outcome(
        &self,
        request: styrene_ipc::types::SendChatRequest,
    ) -> Result<styrene_ipc::types::SendChatOutcome, String> {
        self.broker.send_chat_outcome(&request).await
    }

    async fn draft(
        &self,
        peer_hash: &str,
    ) -> Result<Option<styrene_ipc::types::ConversationDraft>, String> {
        self.broker.draft(peer_hash).await
    }

    async fn save_draft(
        &self,
        peer_hash: &str,
        content: &str,
    ) -> Result<styrene_ipc::types::ConversationDraft, String> {
        self.broker.save_draft(peer_hash, content).await
    }

    async fn discard_draft(&self, peer_hash: &str) -> Result<(), String> {
        self.broker.discard_draft(peer_hash).await
    }

    async fn retry_message(
        &self,
        message_id: &str,
    ) -> Result<styrene_ipc::types::MessagingOperationOutcome, String> {
        self.broker.message_lifecycle(message_id, true).await
    }

    async fn cancel_message(
        &self,
        message_id: &str,
    ) -> Result<styrene_ipc::types::MessagingOperationOutcome, String> {
        self.broker.message_lifecycle(message_id, false).await
    }

    async fn browse_page(
        &self,
        host: &str,
        path: &str,
    ) -> Result<daemon_bridge::PageResponse, String> {
        self.broker.browse_page(host, path).await
    }

    async fn navigate_page(
        &self,
        request: styrene_ipc::types::PageNavigationRequest,
    ) -> Result<daemon_bridge::PageResponse, String> {
        self.broker.navigate_page(request).await
    }

    async fn close_page(&self, session_id: &str) -> Result<(), String> {
        self.broker.close_page(session_id).await
    }

    async fn start_file_download(
        &self,
        request: styrene_ipc::types::FileDownloadRequest,
    ) -> Result<styrene_ipc::types::FileDownloadInfo, String> {
        self.broker.start_file_download(request).await
    }

    async fn file_download(
        &self,
        download_id: &str,
    ) -> Result<styrene_ipc::types::FileDownloadInfo, String> {
        self.broker.file_download(download_id).await
    }

    async fn cancel_file_download(
        &self,
        download_id: &str,
    ) -> Result<styrene_ipc::types::FileDownloadInfo, String> {
        self.broker.cancel_file_download(download_id).await
    }

    async fn save_file_download(
        &self,
        download_id: &str,
        destination: &str,
    ) -> Result<styrene_ipc::types::FileDownloadInfo, String> {
        self.broker.save_file_download(download_id, destination).await
    }

    async fn path_table(&self) -> Result<Vec<PathTableEntry>, String> {
        self.broker.path_table().await
    }

    async fn interface_stats(&self) -> Result<Vec<InterfaceStats>, String> {
        self.broker.interface_stats().await
    }

    async fn links(&self) -> Result<styrene_ipc::types::LinkSnapshot, String> {
        self.broker.links().await
    }

    async fn conversations(&self) -> Result<Vec<ConversationInfo>, String> {
        let values = self.broker.query_conversations().await?;
        Ok(parse_conversation_values(values))
    }

    async fn messages(&self, peer_hash: &str, limit: u32) -> Result<Vec<MessageInfo>, String> {
        self.broker.query_messages(peer_hash, limit).await
    }

    async fn message(&self, message_id: &str) -> Result<Option<MessageInfo>, String> {
        self.broker.query_message(message_id).await
    }

    async fn conversation_page(
        &self,
        cursor: Option<&str>,
    ) -> Result<BackendPage<ConversationInfo>, String> {
        let page = self.broker.query_conversation_page(cursor).await?;
        Ok(BackendPage {
            items: parse_conversation_values(page.items),
            next_cursor: page.next_cursor,
            pagination_supported: page.pagination_supported,
            reset: page.reset,
        })
    }

    async fn message_page(
        &self,
        peer_hash: &str,
        cursor: Option<&str>,
    ) -> Result<BackendPage<MessageInfo>, String> {
        self.broker.query_message_page(peer_hash, cursor).await
    }

    async fn propagation_snapshot(
        &self,
        cursor: Option<&str>,
    ) -> Result<PropagationSnapshot, String> {
        self.broker.propagation_snapshot(cursor).await
    }

    async fn standard_propagation_snapshot(&self) -> Result<StandardPropagationSnapshot, String> {
        self.broker.standard_propagation_snapshot().await
    }

    async fn fleet_status(&self, destination: &str) -> Result<RemoteStatusInfo, String> {
        self.broker.device_status(destination).await
    }

    async fn fleet_exec(
        &self,
        destination: &str,
        command: &str,
        args: &[String],
    ) -> Result<ExecResult, String> {
        self.broker.exec(destination, command, args).await
    }

    async fn fleet_reboot(
        &self,
        destination: &str,
        delay: Option<u64>,
    ) -> Result<RebootResult, String> {
        self.broker.reboot(destination, delay).await
    }

    async fn fleet_apply(
        &self,
        destination: &str,
        profile_base64: &str,
    ) -> Result<ConfigApplyResult, String> {
        self.broker.fleet_apply(destination, profile_base64).await
    }

    async fn block_peer(&self, identity_hash: &str) -> Result<(), String> {
        self.broker.block_peer(identity_hash).await
    }

    async fn start_network_operation(
        &self,
        request: StartNetworkOperationInfo,
    ) -> Result<NetworkOperationInfo, String> {
        self.broker.start_network_operation(request).await
    }

    async fn cancel_network_operation(
        &self,
        operation_id: &str,
    ) -> Result<NetworkOperationInfo, String> {
        self.broker.cancel_network_operation(operation_id).await
    }

    async fn network_operations(&self) -> Result<Vec<NetworkOperationInfo>, String> {
        self.broker.network_operations().await
    }

    async fn start_request(
        &self,
        request: StartRequestInfo,
    ) -> Result<RequestObservationInfo, String> {
        self.broker.start_request(request).await
    }

    async fn cancel_request(&self, request_id: &str) -> Result<RequestObservationInfo, String> {
        self.broker.cancel_request(request_id).await
    }

    async fn requests(&self) -> Result<Vec<RequestObservationInfo>, String> {
        self.broker.requests().await
    }

    async fn resources(&self) -> Result<Vec<ResourceTransferInfo>, String> {
        self.broker.resources().await
    }

    async fn cancel_resource(&self, resource_hash: &str) -> Result<bool, String> {
        self.broker.cancel_resource(resource_hash).await
    }

    async fn shutdown(&self) {
        if let Some(handle) = self.embedded.lock().await.take() {
            handle.shutdown().await;
        }
    }
}

fn parse_conversation_values(
    values: Vec<std::collections::HashMap<String, rmpv::Value>>,
) -> Vec<ConversationInfo> {
    values
        .into_iter()
        .filter_map(|value| {
            let peer_hash = value.get("peer_hash")?.as_str()?.to_string();
            Some(ConversationInfo {
                peer_hash,
                peer_name: value
                    .get("peer_name")
                    .and_then(|item| item.as_str())
                    .filter(|item| !item.is_empty())
                    .map(ToOwned::to_owned),
                last_message: value
                    .get("last_message_content")
                    .and_then(|item| item.as_str())
                    .map(ToOwned::to_owned),
                last_timestamp: value.get("last_message_timestamp").and_then(rmpv::Value::as_i64),
                unread_count: value
                    .get("unread_count")
                    .and_then(rmpv::Value::as_u64)
                    .and_then(|item| u32::try_from(item).ok())
                    .unwrap_or(0),
                message_count: value
                    .get("message_count")
                    .and_then(rmpv::Value::as_u64)
                    .and_then(|item| u32::try_from(item).ok())
                    .unwrap_or(0),
                pinned: value.get("pinned").and_then(rmpv::Value::as_bool).unwrap_or(false),
                muted: value.get("muted").and_then(rmpv::Value::as_bool).unwrap_or(false),
            })
        })
        .collect()
}

#[derive(Clone)]
struct FixtureData {
    identity: IdentityInfo,
    status: DaemonStatusInfo,
    devices: Vec<DeviceInfo>,
    paths: Vec<PathTableEntry>,
    interfaces: Vec<InterfaceStats>,
    conversations: Vec<ConversationInfo>,
    messages: Vec<MessageInfo>,
    page: String,
}

struct FixtureBackend {
    profile: RuntimeProfile,
    data: FixtureData,
}

#[async_trait]
impl BackendSession for FixtureBackend {
    fn profile(&self) -> &RuntimeProfile {
        &self.profile
    }

    fn diagnostics(&self) -> daemon_bridge::BrokerDiagnostics {
        daemon_bridge::BrokerDiagnostics::default()
    }

    async fn send_chat_outcome(
        &self,
        request: styrene_ipc::types::SendChatRequest,
    ) -> Result<styrene_ipc::types::SendChatOutcome, String> {
        let mut message = MessageInfo::default();
        message.id = format!("fixture-{}-{}", request.peer_hash, request.content.len());
        message.destination_hash = request.peer_hash;
        message.content = request.content;
        message.is_outgoing = true;
        message.status = "sent".into();
        message.requested_delivery_method = request.delivery_method.clone();
        message.actual_delivery_method = request.delivery_method;
        let mut outcome = styrene_ipc::types::SendChatOutcome::default();
        outcome.disposition = styrene_ipc::types::SendChatDisposition::Accepted;
        outcome.message_id = message.id.clone();
        outcome.requested_method =
            message.requested_delivery_method.clone().unwrap_or_else(|| "direct".into());
        outcome.actual_method =
            message.actual_delivery_method.clone().unwrap_or_else(|| "direct".into());
        outcome.message = message;
        Ok(outcome)
    }

    async fn draft(
        &self,
        _peer_hash: &str,
    ) -> Result<Option<styrene_ipc::types::ConversationDraft>, String> {
        Ok(None)
    }

    async fn save_draft(
        &self,
        peer_hash: &str,
        content: &str,
    ) -> Result<styrene_ipc::types::ConversationDraft, String> {
        let mut draft = styrene_ipc::types::ConversationDraft::default();
        draft.peer_hash = peer_hash.into();
        draft.content = content.into();
        Ok(draft)
    }

    async fn discard_draft(&self, _peer_hash: &str) -> Result<(), String> {
        Ok(())
    }

    async fn retry_message(
        &self,
        message_id: &str,
    ) -> Result<styrene_ipc::types::MessagingOperationOutcome, String> {
        let mut outcome = styrene_ipc::types::MessagingOperationOutcome::default();
        outcome.disposition = styrene_ipc::types::MessagingDisposition::Applied;
        outcome.target_id = message_id.into();
        Ok(outcome)
    }

    async fn cancel_message(
        &self,
        message_id: &str,
    ) -> Result<styrene_ipc::types::MessagingOperationOutcome, String> {
        self.retry_message(message_id).await
    }

    async fn browse_page(
        &self,
        host: &str,
        _path: &str,
    ) -> Result<daemon_bridge::PageResponse, String> {
        let mut page = styrene_ipc::types::PageContent::default();
        page.source_bytes = self.data.page.as_bytes().to_vec();
        page.rendered_text = self.data.page.clone();
        page.host_hash = host.into();
        page.fetched_at = 1_700_000_200;
        Ok(daemon_bridge::PageResponse { page })
    }

    async fn path_table(&self) -> Result<Vec<PathTableEntry>, String> {
        Ok(self.data.paths.clone())
    }

    async fn interface_stats(&self) -> Result<Vec<InterfaceStats>, String> {
        Ok(self.data.interfaces.clone())
    }

    async fn links(&self) -> Result<styrene_ipc::types::LinkSnapshot, String> {
        Ok(Default::default())
    }

    async fn conversations(&self) -> Result<Vec<ConversationInfo>, String> {
        Ok(self.data.conversations.clone())
    }

    async fn messages(&self, peer_hash: &str, limit: u32) -> Result<Vec<MessageInfo>, String> {
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        Ok(self
            .data
            .messages
            .iter()
            .filter(|message| {
                message.source_hash == peer_hash || message.destination_hash == peer_hash
            })
            .take(limit)
            .cloned()
            .collect())
    }

    async fn message(&self, message_id: &str) -> Result<Option<MessageInfo>, String> {
        Ok(self.data.messages.iter().find(|message| message.id == message_id).cloned())
    }

    async fn propagation_snapshot(
        &self,
        cursor: Option<&str>,
    ) -> Result<PropagationSnapshot, String> {
        let active =
            matches!(&self.profile, RuntimeProfile::Fixture { fixture: FixtureId::ActiveScenario });
        let mut snapshot = PropagationSnapshot::default();
        snapshot.enabled = active;
        snapshot.expiry_secs = 604_800;
        snapshot.peer_state_supported = false;
        snapshot.sync_state_supported = false;
        if active && cursor.is_none() {
            let mut entry = PropagationQueueEntry::default();
            entry.id = "fixture-propagation-1".into();
            entry.destination_hash = "00000000000000000000000000000001".into();
            entry.source_hash = Some("00000000000000000000000000000002".into());
            entry.received_at = 1_700_000_000;
            entry.expires_at = 1_700_604_800;
            entry.size_bytes = 4096;
            entry.state = "stored".into();
            snapshot.queue.push(entry);
            snapshot.queue_count = 1;
            snapshot.queue_size_bytes = 4096;
        }
        Ok(snapshot)
    }

    async fn standard_propagation_snapshot(&self) -> Result<StandardPropagationSnapshot, String> {
        Ok(StandardPropagationSnapshot::default())
    }

    async fn fleet_status(&self, destination: &str) -> Result<RemoteStatusInfo, String> {
        let mut status = RemoteStatusInfo::default();
        status.destination_hash = destination.into();
        status.uptime = Some(3600);
        status.daemon_version = Some("fixture-1".into());
        Ok(status)
    }

    async fn fleet_exec(
        &self,
        _destination: &str,
        command: &str,
        args: &[String],
    ) -> Result<ExecResult, String> {
        let mut result = ExecResult::default();
        result.stdout = format!("fixture: {command} {}", args.join(" ")).trim().to_string();
        Ok(result)
    }

    async fn fleet_reboot(
        &self,
        _destination: &str,
        delay: Option<u64>,
    ) -> Result<RebootResult, String> {
        let mut result = RebootResult::default();
        result.accepted = true;
        result.delay_secs = delay;
        Ok(result)
    }

    async fn fleet_apply(
        &self,
        _destination: &str,
        _profile_base64: &str,
    ) -> Result<ConfigApplyResult, String> {
        Err("unsupported: fixture backend does not apply profiles".into())
    }

    async fn block_peer(&self, _identity_hash: &str) -> Result<(), String> {
        Err("permission denied: fixture backend has no administration capability".into())
    }

    async fn start_network_operation(
        &self,
        request: StartNetworkOperationInfo,
    ) -> Result<NetworkOperationInfo, String> {
        let mut operation = NetworkOperationInfo::default();
        operation.operation_id = format!("fixture-{}", request.kind.as_str());
        operation.kind = request.kind;
        operation.destination_hash = request.destination_hash;
        operation.link_id = request.link_id;
        operation.outcome = Some(styrene_ipc::types::NetworkOperationOutcome::Unavailable);
        operation.detail = Some("fixture does not emulate network mutations".into());
        operation.observation = fixture_observation();
        Ok(operation)
    }

    async fn cancel_network_operation(
        &self,
        _operation_id: &str,
    ) -> Result<NetworkOperationInfo, String> {
        Err("unsupported: fixture has no active network operation".into())
    }

    async fn network_operations(&self) -> Result<Vec<NetworkOperationInfo>, String> {
        Ok(Vec::new())
    }

    async fn start_request(
        &self,
        _request: StartRequestInfo,
    ) -> Result<RequestObservationInfo, String> {
        Err("unsupported: fixture does not emulate native requests".into())
    }

    async fn cancel_request(&self, _request_id: &str) -> Result<RequestObservationInfo, String> {
        Err("unsupported: fixture has no active request".into())
    }

    async fn requests(&self) -> Result<Vec<RequestObservationInfo>, String> {
        Ok(Vec::new())
    }

    async fn resources(&self) -> Result<Vec<ResourceTransferInfo>, String> {
        Ok(Vec::new())
    }

    async fn cancel_resource(&self, _resource_hash: &str) -> Result<bool, String> {
        Err("unsupported: fixture has no active resource".into())
    }

    async fn shutdown(&self) {}
}

fn open_fixture(fixture: FixtureId, generation: ConnectionGeneration) -> OpenedSession {
    let mut data = fixture_data(fixture);
    data.status.connection_generation = Some(generation.0);
    for path in &mut data.paths {
        path.observation.connection_generation = Some(generation.0);
    }
    for interface in &mut data.interfaces {
        interface.observation.connection_generation = Some(generation.0);
    }
    let mut capabilities = ActiveCapabilitiesInfo::default();
    capabilities.version = ACTIVE_CAPABILITIES_VERSION;
    capabilities.authorized_operations = vec![
        "chat.send".into(),
        "messaging.lifecycle".into(),
        "messaging.manage".into(),
        "page.browse".into(),
        "rpc.status".into(),
        "rpc.exec".into(),
        "rpc.reboot".into(),
    ];
    capabilities.runtime = vec!["fixture.scenarios".into(), "runtime.lxmf.direct".into()];
    data.status.active_capabilities = Some(capabilities);
    let (tx, events) = mpsc::channel(1024);
    let _ = tx.try_send(DaemonEvent::Connected);
    let _ = tx.try_send(DaemonEvent::Identity(data.identity.clone()));
    let _ = tx.try_send(DaemonEvent::Status(data.status.clone()));
    for device in &data.devices {
        let _ = tx.try_send(DaemonEvent::PeerDiscovered(device.clone()));
    }
    let _ = tx.try_send(DaemonEvent::PathTable(data.paths.clone()));
    drop(tx);
    OpenedSession {
        backend: Arc::new(FixtureBackend { profile: RuntimeProfile::Fixture { fixture }, data }),
        events,
        generation,
    }
}

fn fixture_observation() -> styrene_ipc::types::ObservationMetadata {
    let mut observation = styrene_ipc::types::ObservationMetadata::default();
    observation.source = styrene_ipc::types::ObservationSource::Fixture;
    observation
}

fn fixture_data(fixture: FixtureId) -> FixtureData {
    let mut identity = IdentityInfo::default();
    identity.identity_hash = "11111111111111111111111111111111".into();
    identity.destination_hash = "22222222222222222222222222222222".into();
    identity.display_name = "Fixture Operator".into();

    let mut status = DaemonStatusInfo::default();
    status.daemon_version = "fixture-1".into();
    status.rns_initialized = true;
    status.transport_enabled = true;
    let mut devices = Vec::new();
    let mut paths = Vec::new();
    let mut interfaces = Vec::new();
    let mut conversations = Vec::new();
    let mut messages = Vec::new();

    let count = match fixture {
        FixtureId::Empty => 0,
        FixtureId::Healthy | FixtureId::Degraded | FixtureId::ActiveScenario => 3,
        FixtureId::HighCardinality => 500,
        FixtureId::Error => 0,
    };
    for index in 0..count {
        let hash = format!("{index:032x}");
        let mut device = DeviceInfo::default();
        device.destination_hash = hash.clone();
        device.name = if index % 2 == 0 {
            format!("styrene:Fixture Peer {index}:1.0:fleet,status,exec,reboot,pages")
        } else {
            format!("Fixture Peer {index}")
        };
        device.status = if fixture == FixtureId::Degraded { "stale" } else { "online" }.into();
        device.is_styrene_node = index % 2 == 0;
        if index % 2 == 0 {
            device.discovered_capabilities =
                vec![styrene_ipc::types::DiscoveredCapability::NativeNomadNetHost];
        }
        device.last_announce = Some(1_700_000_000 + i64::from(index));
        device.announce_count = index + 1;
        devices.push(device);
        if index < 12 {
            paths.push(PathTableEntry {
                destination_hash: hash.clone(),
                hops: u8::try_from(index % 3 + 1).unwrap_or(1),
                next_hop: hash,
                interface: "fixture-tcp".into(),
                expires: Some(1_700_604_800),
                observation: fixture_observation(),
            });
        }
    }
    if fixture != FixtureId::Empty {
        let mut interface = InterfaceStats::default();
        interface.name = "Fixture TCP".into();
        interface.hash = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into();
        interface.kind = "tcp_client".into();
        interface.mode = "point_to_point".into();
        interface.enabled = true;
        interface.status =
            if fixture == FixtureId::Degraded { "degraded" } else { "online" }.into();
        interface.local_endpoint = Some("127.0.0.1:4242".into());
        interface.remote_endpoint = Some("127.0.0.1:4243".into());
        interface.tx_bytes = 4096;
        interface.rx_bytes = 8192;
        interface.peers_connected = 1;
        interface.observation = fixture_observation();
        interfaces.push(interface);
        let peer_hash = devices[0].destination_hash.clone();
        conversations.push(ConversationInfo {
            peer_hash: peer_hash.clone(),
            peer_name: Some(devices[0].name.clone()),
            last_message: Some("Fixture message".into()),
            last_timestamp: Some(1_700_000_100),
            unread_count: 1,
            message_count: 1,
            pinned: false,
            muted: false,
        });
        let mut message = MessageInfo::default();
        message.id = "fixture-message-1".into();
        message.source_hash = peer_hash;
        message.destination_hash = identity.destination_hash.clone();
        message.content = "Fixture message".into();
        message.timestamp = 1_700_000_100;
        message.status = if fixture == FixtureId::Degraded { "failed" } else { "delivered" }.into();
        message.delivery_method =
            Some(if fixture == FixtureId::ActiveScenario { "propagated" } else { "direct" }.into());
        message.read = fixture == FixtureId::Healthy;
        if fixture == FixtureId::ActiveScenario {
            let mut attachment = styrene_ipc::types::AttachmentInfo::default();
            attachment.name = "evidence.bin".into();
            attachment.content_type = "application/octet-stream".into();
            attachment.size = 4096;
            message.attachment_info = Some(attachment);
        }
        messages.push(message);
    }
    status.device_count = u32::try_from(devices.len()).unwrap_or(u32::MAX);
    status.interface_count = u32::try_from(interfaces.len()).unwrap_or(u32::MAX);
    status.transport_enabled = fixture != FixtureId::Degraded;
    status.propagation_enabled = fixture == FixtureId::ActiveScenario;
    status.active_links = u32::from(fixture == FixtureId::ActiveScenario);

    FixtureData {
        identity,
        status,
        devices,
        paths,
        interfaces,
        conversations,
        messages,
        page: "# Fixture Page\n\nDeterministic content from Fixture mode.".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_live() {
        assert!(matches!(
            RuntimeProfile::from_values(None, Some(PathBuf::from("/tmp/live.sock")), None),
            Ok(RuntimeProfile::Live { .. })
        ));
    }

    #[test]
    fn profiles_require_explicit_valid_names() {
        assert!(matches!(
            RuntimeProfile::from_values(Some("embedded"), None, None),
            Ok(RuntimeProfile::Embedded { .. })
        ));
        assert!(RuntimeProfile::from_values(Some("automatic"), None, None).is_err());
        assert!(RuntimeProfile::from_values(Some("fixture"), None, Some("missing")).is_err());
    }

    #[tokio::test]
    async fn fixtures_are_deterministic_and_do_not_open_runtime_services() {
        let first = open_session(
            RuntimeProfile::Fixture { fixture: FixtureId::HighCardinality },
            ConnectionGeneration(1),
        )
        .await
        .unwrap();
        let second = open_session(
            RuntimeProfile::Fixture { fixture: FixtureId::HighCardinality },
            ConnectionGeneration(2),
        )
        .await
        .unwrap();

        let normalize = |mut paths: Vec<PathTableEntry>| {
            for path in &mut paths {
                path.observation.connection_generation = None;
            }
            paths
        };
        assert_eq!(
            normalize(first.backend.path_table().await.unwrap()),
            normalize(second.backend.path_table().await.unwrap())
        );
        assert_eq!(first.backend.profile().label(), "Fixture");
        assert_eq!(first.generation, ConnectionGeneration(1));
    }

    #[test]
    fn fixtures_cover_the_required_runtime_states() {
        let empty = fixture_data(FixtureId::Empty);
        let healthy = fixture_data(FixtureId::Healthy);
        let degraded = fixture_data(FixtureId::Degraded);
        let high_cardinality = fixture_data(FixtureId::HighCardinality);
        let active = fixture_data(FixtureId::ActiveScenario);
        let error = RuntimeProfile::fixture(FixtureId::Error);

        assert!(empty.devices.is_empty());
        assert_eq!(healthy.devices.len(), 3);
        assert!(healthy.status.transport_enabled);
        assert!(!degraded.status.transport_enabled);
        assert!(degraded.devices.iter().all(|device| device.status == "stale"));
        assert_eq!(high_cardinality.devices.len(), 500);
        assert!(active.status.propagation_enabled);
        assert_eq!(active.status.active_links, 1);
        assert_eq!(healthy.messages[0].delivery_method.as_deref(), Some("direct"));
        assert_eq!(degraded.messages[0].status, "failed");
        assert_eq!(active.messages[0].attachment_info.as_ref().map(|item| item.size), Some(4096));
        assert!(matches!(error, RuntimeProfile::Fixture { fixture: FixtureId::Error }));
    }

    #[tokio::test]
    async fn error_fixture_fails_without_opening_a_daemon_or_network_interface() {
        let result =
            open_session(RuntimeProfile::fixture(FixtureId::Error), ConnectionGeneration(7)).await;
        assert!(matches!(result, Err(error) if error.contains("Fixture error")));
    }

    #[test]
    fn backend_features_are_derived_only_from_current_negotiation() {
        assert_eq!(
            BackendCapabilities::negotiated(true, Some(1), None),
            BackendCapabilities::default()
        );
        let mut active = ActiveCapabilitiesInfo::default();
        active.version = ACTIVE_CAPABILITIES_VERSION + 1;
        active.authorized_operations =
            vec!["chat.send".into(), "rpc.exec".into(), "rpc.status".into()];
        assert_eq!(
            BackendCapabilities::negotiated(true, Some(1), Some(&active)),
            BackendCapabilities::default()
        );
        active.version = ACTIVE_CAPABILITIES_VERSION;
        active.runtime = vec!["runtime.standard-lxmf.propagation".into()];
        let negotiated = BackendCapabilities::negotiated(true, Some(1), Some(&active));
        assert!(negotiated.messaging);
        assert!(negotiated.fleet);
        assert!(negotiated.administration);
        assert!(negotiated.propagation);
        active.runtime = vec!["runtime.standard-lxmf.propagation-client".into()];
        assert!(BackendCapabilities::negotiated(true, Some(1), Some(&active)).propagation);
        active.runtime = vec!["lxmf.propagation".into()];
        assert!(BackendCapabilities::negotiated(true, Some(1), Some(&active)).propagation);
        active.authorized_operations.retain(|operation| operation != "rpc.status");
        assert!(!BackendCapabilities::negotiated(true, Some(1), Some(&active)).propagation);
        assert_eq!(
            BackendCapabilities::negotiated(false, Some(1), Some(&active)),
            BackendCapabilities::default()
        );
    }

    #[tokio::test]
    #[ignore = "live socket failure smoke is isolated from ordinary offline validation"]
    async fn missing_live_socket_never_falls_back_to_embedded() {
        let path = std::env::temp_dir().join(format!(
            "styrene-dx-missing-{}-{}.sock",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let result =
            open_session(RuntimeProfile::Live { socket_path: path }, ConnectionGeneration(1)).await;
        assert!(result.is_err());
    }
}
