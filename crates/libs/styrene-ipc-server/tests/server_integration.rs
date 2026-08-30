//! Integration tests for the IPC server.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use styrene_ipc::error::IpcError;
use styrene_ipc::traits::*;
use styrene_ipc::types::*;
use styrene_ipc_server::wire::{self, MessageType, REQUEST_ID_SIZE};
use styrene_ipc_server::{IpcServer, IpcServerConfig};
use tokio::net::UnixStream;

// ── Test daemon ─────────────────────────────────────────────────────────

#[derive(Default)]
struct TestDaemon {
    pages: std::sync::Mutex<PageTestState>,
    blocked_browse: std::sync::Mutex<Option<Arc<BlockedBrowse>>>,
    panic_browse: std::sync::atomic::AtomicBool,
    cleaned_owners: std::sync::Mutex<Vec<u64>>,
    cleanup_failures: std::sync::atomic::AtomicUsize,
    peer_calls: std::sync::atomic::AtomicUsize,
    send_chat_calls: std::sync::atomic::AtomicUsize,
}

#[derive(Clone, Copy, Debug)]
enum BlockedBrowseStage {
    Path,
    Link,
    Transfer,
}

struct BlockedBrowse {
    stage: BlockedBrowseStage,
    entered: tokio::sync::Notify,
    cancelled: std::sync::atomic::AtomicBool,
    created_link_closed: std::sync::atomic::AtomicBool,
    owner_cleaned: std::sync::atomic::AtomicBool,
    owner: std::sync::atomic::AtomicU64,
}

impl BlockedBrowse {
    fn new(stage: BlockedBrowseStage) -> Self {
        Self {
            stage,
            entered: tokio::sync::Notify::new(),
            cancelled: std::sync::atomic::AtomicBool::new(false),
            created_link_closed: std::sync::atomic::AtomicBool::new(false),
            owner_cleaned: std::sync::atomic::AtomicBool::new(false),
            owner: std::sync::atomic::AtomicU64::new(0),
        }
    }

    fn created_link(&self) -> bool {
        matches!(self.stage, BlockedBrowseStage::Link | BlockedBrowseStage::Transfer)
    }
}

struct BlockedBrowseGuard(Arc<BlockedBrowse>);

impl Drop for BlockedBrowseGuard {
    fn drop(&mut self) {
        self.0.cancelled.store(true, std::sync::atomic::Ordering::Release);
        if self.0.created_link() {
            self.0.created_link_closed.store(true, std::sync::atomic::Ordering::Release);
        }
    }
}

#[derive(Default)]
struct PageTestState {
    next_id: u64,
    sessions: HashMap<String, u64>,
    downloads: HashMap<String, (u64, FileDownloadInfo)>,
}

fn canonical_test_message() -> MessageInfo {
    let mut message = MessageInfo::default();
    message.id = "message-2".into();
    message.source_hash = "11111111111111111111111111111111".into();
    message.destination_hash = "22222222222222222222222222222222".into();
    message.lxmf_timestamp = Some(1_700_000_000.125);
    message.canonical_title = Some(vec![0xfe]);
    message.canonical_content = Some(vec![0xff, 0x00]);
    message.canonical_fields_msgpack = Some(vec![0x81, 0xcc, 0x01, 0xcd, 0x00, 0x02]);
    message.canonical_signature = Some(vec![0x33; 64]);
    message.canonical_stamp = Some(vec![0x44; 32]);
    message.canonical_wire = Some(vec![0x55; 128]);
    message.authentication_state = MessageAuthenticationState::Verified;
    message.stamp_state = MessageStampState::Invalid;
    message.stamp_value = Some(17);
    message.stamp_cost = Some(12);
    message
}

fn maximum_accepted_message() -> MessageInfo {
    let mut message = canonical_test_message();
    message.content = String::from_utf8_lossy(&vec![0xff; 1024 * 1024]).into_owned();
    message.canonical_title = Some(vec![0xaa; 1024 * 1024]);
    message.canonical_content = Some(vec![0xbb; 1024 * 1024]);
    message.canonical_fields_msgpack = Some(vec![0xcc; 1024 * 1024]);
    message.canonical_wire = Some(vec![0xdd; 4 * 1024 * 1024]);
    message
}

fn assert_canonical_wire_fields(message: &[(rmpv::Value, rmpv::Value)]) {
    let field = |name: &str| {
        message.iter().find(|(key, _)| key.as_str() == Some(name)).map(|(_, value)| value)
    };
    assert_eq!(field("lxmf_timestamp").and_then(rmpv::Value::as_f64), Some(1_700_000_000.125));
    for secret in [
        "canonical_title",
        "canonical_content",
        "canonical_fields_msgpack",
        "canonical_signature",
        "canonical_stamp",
        "canonical_wire",
    ] {
        assert!(field(secret).is_none(), "{secret} must not cross local IPC");
    }
    assert_eq!(field("authentication_state").and_then(rmpv::Value::as_str), Some("verified"));
    assert_eq!(field("stamp_state").and_then(rmpv::Value::as_str), Some("invalid"));
    assert_eq!(field("stamp_value").and_then(rmpv::Value::as_u64), Some(17));
    assert_eq!(field("stamp_cost").and_then(rmpv::Value::as_u64), Some(12));
}

fn test_observation(source: ObservationSource) -> ObservationMetadata {
    let mut observation = ObservationMetadata::at(source, Some(90), 100, 30);
    observation.connection_generation = Some(73);
    observation
}

fn test_path() -> PathInfo {
    let mut path = PathInfo::default();
    path.destination_hash = "11111111111111111111111111111111".into();
    path.hops = Some(2);
    path.next_hop = Some("22222222222222222222222222222222".into());
    path.interface = Some("33333333333333333333333333333333".into());
    path.expires = Some(700);
    path.observation = test_observation(ObservationSource::TransportPathTable);
    path
}

fn test_request(state: RequestState) -> RequestObservationInfo {
    let mut request = RequestObservationInfo::default();
    request.request_id = "aa".repeat(16);
    request.path_hash = "bb".repeat(16);
    request.link_id = "cc".repeat(16);
    request.started_monotonic_ms = 10;
    request.deadline_monotonic_ms = 1_010;
    request.request_size = 3;
    request.response = Some(vec![0xc4, 0x01, 0xaa]);
    request.request_resource_hash = Some("dd".repeat(32));
    request.state = state;
    request
}

fn test_network_operation(outcome: Option<NetworkOperationOutcome>) -> NetworkOperationInfo {
    let mut operation = NetworkOperationInfo::default();
    operation.operation_id = "dd".repeat(16);
    operation.kind = NetworkOperationKind::PathRequest;
    operation.destination_hash = Some("11".repeat(16));
    operation.started_unix_ms = 100;
    operation.deadline_unix_ms = 1_100;
    operation.cancellable = true;
    operation.progress = NetworkOperationProgress::AwaitingPath;
    operation.outcome = outcome;
    operation.observation = test_observation(ObservationSource::OperationCoordinator);
    operation.observation.correlation_id = Some(operation.operation_id.clone());
    operation
}

fn test_resource() -> ResourceTransferInfo {
    let mut resource = ResourceTransferInfo::default();
    resource.resource_hash = "ee".repeat(32);
    resource.link_id = "cc".repeat(16);
    resource.direction = ResourceDirection::Inbound;
    resource.state = ResourceTransferState::Transferring;
    resource.received_bytes = 512;
    resource.total_bytes = 1_024;
    resource.progress = 0.5;
    resource.cancellable = true;
    resource.observation = test_observation(ObservationSource::TransportResourceState);
    resource
}

#[async_trait]
impl DaemonStatus for TestDaemon {
    async fn query_status(&self) -> Result<DaemonStatusInfo, IpcError> {
        let mut info = DaemonStatusInfo::default();
        info.uptime = 42;
        info.daemon_version = "test-0.1.0".into();
        info.rns_initialized = true;
        info.standard_lxmf_propagation_destination_registered = true;
        info.standard_lxmf_propagation_active = false;
        let mut degraded = DegradedCapabilityInfo::default();
        degraded.id = "runtime.native-nomadnet.host".into();
        degraded.reason = "request handler unavailable".into();
        let mut capabilities = ActiveCapabilitiesInfo::default();
        capabilities.version = ACTIVE_CAPABILITIES_VERSION;
        capabilities.runtime = vec!["runtime.lxmf.direct".into()];
        capabilities.degraded = vec![degraded];
        capabilities.authorized_operations = vec!["chat.send".into()];
        info.active_capabilities = Some(capabilities);
        Ok(info)
    }
    async fn mobile_diagnostics(&self) -> Result<MobileDiagnosticSnapshot, IpcError> {
        Ok(MobileDiagnosticSnapshot {
            schema_version: MOBILE_DIAGNOSTIC_SCHEMA_VERSION,
            backend_revision: "styrened/test".into(),
            first_sequence: Some(7),
            last_sequence: Some(7),
            event_count: 1,
            retained_bytes: 128,
            max_events: MOBILE_DIAGNOSTIC_MAX_EVENTS,
            max_bytes: MOBILE_DIAGNOSTIC_MAX_BYTES,
            truncated: true,
            dropped_events: 2,
            events: vec![MobileDiagnosticEvent {
                sequence: 7,
                unix_time_ms: Some(1_700_000_000_000),
                source: MobileDiagnosticSource::Messaging,
                stage: MobileDiagnosticStage::Outbound,
                severity: MobileDiagnosticSeverity::Warning,
                generation: 3,
                safe_correlation: Some("sha256:abcd".into()),
            }],
        })
    }
    async fn export_mobile_diagnostics(&self) -> Result<MobileDiagnosticExport, IpcError> {
        let bytes = br#"{"schema_version":1}"#.to_vec();
        Ok(MobileDiagnosticExport {
            schema_version: MOBILE_DIAGNOSTIC_SCHEMA_VERSION,
            backend_revision: "styrened/test".into(),
            content_type: "application/vnd.styrene.mobile-diagnostics+json".into(),
            digest_sha256: "ab".repeat(32),
            first_sequence: Some(7),
            last_sequence: Some(7),
            event_count: 1,
            byte_count: bytes.len() as u64,
            max_events: MOBILE_DIAGNOSTIC_MAX_EVENTS,
            max_bytes: MOBILE_DIAGNOSTIC_MAX_BYTES,
            truncated: true,
            dropped_events: 2,
            bytes,
        })
    }
    async fn query_standard_propagation(&self) -> Result<StandardPropagationSnapshot, IpcError> {
        let mut snapshot = StandardPropagationSnapshot::default();
        snapshot.version = STANDARD_PROPAGATION_SNAPSHOT_VERSION;
        snapshot.registered = true;
        snapshot.active = true;
        snapshot.observed_at = Some(100);
        let mut policy = StandardPropagationPolicyInfo::default();
        policy.target_cost = 16;
        policy.flexibility = 3;
        policy.peering_cost = 18;
        policy.transfer_limit_kb = 256;
        policy.sync_limit_kb = 4000;
        policy.queue_max_count = 4096;
        policy.queue_max_bytes = 16 * 1024 * 1024;
        policy.expiry_secs = 2_592_000;
        policy.throttle_secs = 180;
        policy.max_offer_links = 3;
        snapshot.policy = Some(policy);
        let mut attempt = StandardPropagationAttemptObservation::default();
        attempt.attempt_id = "11".repeat(16);
        attempt.correlation_id = "22".repeat(16);
        attempt.peer_hash = Some("33".repeat(16));
        attempt.direction = StandardPropagationDirection::Ingress;
        attempt.stage = StandardPropagationStage::Offer;
        attempt.state = StandardPropagationAttemptState::Running;
        attempt.outcome = StandardPropagationOutcome::Pending;
        attempt.started_at = 90;
        attempt.updated_at = 100;
        attempt.offered_count = 2;
        attempt.wanted_count = 1;
        snapshot.attempts.push(attempt);
        Ok(snapshot)
    }
    async fn query_config(&self) -> Result<ConfigSnapshot, IpcError> {
        Ok(ConfigSnapshot::default())
    }
    async fn query_devices(&self, _styrene_only: bool) -> Result<Vec<DeviceInfo>, IpcError> {
        let mut d = DeviceInfo::default();
        d.destination_hash = "abcd1234".into();
        d.name = "test-node".into();
        d.standard_lxmf_propagation_active = Some(false);
        Ok(vec![d])
    }
    async fn query_path_info(&self, _dest: &str) -> Result<PathInfo, IpcError> {
        Ok(test_path())
    }
    async fn query_path_table(&self) -> Result<Vec<PathInfo>, IpcError> {
        Ok(vec![test_path()])
    }
    async fn query_auto_reply(&self) -> Result<AutoReplyConfig, IpcError> {
        Ok(AutoReplyConfig::default())
    }
    async fn set_auto_reply(
        &self,
        _mode: &str,
        _msg: Option<&str>,
        _cd: Option<u64>,
    ) -> Result<bool, IpcError> {
        Ok(true)
    }
    async fn save_config(&self, _config: ConfigSnapshot) -> Result<bool, IpcError> {
        Ok(true)
    }
    async fn block_peer(&self, _hash: &str) -> Result<bool, IpcError> {
        Ok(true)
    }
    async fn unblock_peer(&self, _hash: &str) -> Result<bool, IpcError> {
        Ok(true)
    }
    async fn blocked_peers(&self) -> Result<Vec<String>, IpcError> {
        Ok(vec![])
    }
    async fn list_interfaces(&self) -> Result<Vec<InterfaceDetail>, IpcError> {
        let mut interface = InterfaceDetail::default();
        interface.name = "runtime".into();
        interface.hash = "11111111111111111111111111111111".into();
        interface.kind = "tcp_server".into();
        interface.mode = "full".into();
        interface.enabled = true;
        interface.status = "listening".into();
        interface.host = Some("127.0.0.1".into());
        interface.port = Some(4242);
        interface.local_endpoint = Some("127.0.0.1:4242".into());
        interface.remote_endpoint = Some("192.0.2.1:5252".into());
        interface.parent_hash = Some("22222222222222222222222222222222".into());
        interface.tx_bytes = u64::MAX;
        interface.rx_bytes = i64::MAX as u64 + 1;
        interface.peers_connected = 3;
        interface.observation = test_observation(ObservationSource::RuntimeInterfaceRegistry);
        interface.observation.interface_generation = Some(5);
        Ok(vec![interface])
    }
    async fn search_peers(&self, _q: &str, _limit: u32) -> Result<Vec<DeviceInfo>, IpcError> {
        Ok(vec![])
    }
    async fn bookmark_peer(&self, _hash: &str) -> Result<bool, IpcError> {
        Ok(true)
    }
    async fn unbookmark_peer(&self, _hash: &str) -> Result<bool, IpcError> {
        Ok(true)
    }
}

#[async_trait]
impl DaemonIdentity for TestDaemon {
    async fn query_identity(&self) -> Result<IdentityInfo, IpcError> {
        let mut info = IdentityInfo::default();
        info.identity_hash = "deadbeef".into();
        info.display_name = "Test Node".into();
        info.custody = Some(IdentityCustodyInfo {
            requested_backend: IdentityCustodyBackend::EncryptedFile,
            active_backend: Some(IdentityCustodyBackend::EncryptedFile),
            protection: Some(IdentityCustodyProtection::EncryptedAtRest),
            authentication: IdentityCustodyAuthentication::DeviceAuthentication,
            availability: IdentityCustodyAvailability::Available,
            downgrade: IdentityCustodyDowngrade::None,
            failure: None,
        });
        Ok(info)
    }
    async fn set_identity(
        &self,
        _name: Option<&str>,
        _icon: Option<&str>,
        _short: Option<&str>,
    ) -> Result<bool, IpcError> {
        Ok(true)
    }
    async fn announce(&self) -> Result<bool, IpcError> {
        Ok(true)
    }
}

#[async_trait]
impl DaemonMessaging for TestDaemon {
    async fn send_chat(&self, _req: SendChatRequest) -> Result<MessageId, IpcError> {
        self.send_chat_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok("sent-message".into())
    }
    async fn send_chat_outcome(&self, req: SendChatRequest) -> Result<SendChatOutcome, IpcError> {
        let mut message = MessageInfo::default();
        message.id = "persisted-message".into();
        message.destination_hash = req.peer_hash;
        message.content = req.content;
        message.status = "failed: fixture".into();
        message.is_outgoing = true;
        let mut outcome = SendChatOutcome::default();
        outcome.disposition = SendChatDisposition::Failed;
        outcome.message_id = message.id.clone();
        outcome.message = message;
        outcome.requested_method = "paper".into();
        outcome.actual_method = "paper".into();
        outcome.terminal_error = Some("fixture".into());
        Ok(outcome)
    }
    async fn set_draft(
        &self,
        peer_hash: &str,
        content: &str,
    ) -> Result<ConversationDraft, IpcError> {
        let mut draft = ConversationDraft::default();
        draft.peer_hash = peer_hash.into();
        draft.content = content.into();
        draft.updated_at = 42;
        Ok(draft)
    }
    async fn draft(&self, peer_hash: &str) -> Result<Option<ConversationDraft>, IpcError> {
        self.set_draft(peer_hash, "retained").await.map(Some)
    }
    async fn clear_draft(&self, _peer_hash: &str) -> Result<MessagingDisposition, IpcError> {
        Ok(MessagingDisposition::Unchanged)
    }
    async fn mark_read(&self, _peer: &str) -> Result<u64, IpcError> {
        self.peer_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(0)
    }
    async fn delete_conversation(&self, _peer: &str) -> Result<u64, IpcError> {
        Ok(0)
    }
    async fn delete_message(&self, _id: &str) -> Result<bool, IpcError> {
        Ok(false)
    }
    async fn retry_message(&self, _id: &str) -> Result<bool, IpcError> {
        Ok(false)
    }

    async fn cancel_message(&self, _id: &str) -> Result<bool, IpcError> {
        Ok(false)
    }
    async fn query_conversations(&self, unread: bool) -> Result<Vec<ConversationInfo>, IpcError> {
        let mut conversation = ConversationInfo::default();
        conversation.peer_hash = if unread { "unread" } else { "all" }.into();
        Ok(vec![conversation])
    }
    async fn query_conversation_page(
        &self,
        _unread: bool,
        _limit: u32,
        cursor: Option<&str>,
    ) -> Result<ConversationPage, IpcError> {
        let mut page = ConversationPage::default();
        if cursor.is_none() {
            page.next_cursor = Some("conversation-cursor".into());
        }
        Ok(page)
    }
    async fn query_messages(
        &self,
        peer: &str,
        _limit: u32,
        _before: Option<i64>,
    ) -> Result<Vec<MessageInfo>, IpcError> {
        self.peer_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if peer.starts_with("ffff") {
            return Ok(vec![maximum_accepted_message(), maximum_accepted_message()]);
        }
        let mut attempt = MessageAttemptInfo::default();
        attempt.message_id = "message-2".into();
        attempt.number = 2;
        attempt.started_unix_ms = 100;
        attempt.deadline_unix_ms = 200;
        attempt.state = "failed".into();
        attempt.route.outcome = MessageAttemptRouteOutcome::Observed;
        attempt.route.connection_generation = Some(4);
        attempt.route.observed_at = Some(150);
        attempt.route.next_hop = Some("33".repeat(16));
        attempt.route.hops = Some(1);
        attempt.route.stale = false;
        let mut interface = MessageAttemptInterfaceObservation::default();
        interface.id = "44".repeat(16);
        interface.kind = "tcp_client".into();
        interface.generation = 4;
        attempt.route.interface = Some(interface);
        let mut message = canonical_test_message();
        message.requested_delivery_method = Some("opportunistic".into());
        message.actual_delivery_method = Some("direct".into());
        message.fallback_reason = Some("packet limit".into());
        message.correlation_id = Some("send-1".into());
        message.attempts = vec![attempt];
        Ok(vec![message])
    }

    async fn query_message(&self, message_id: &str) -> Result<Option<MessageInfo>, IpcError> {
        Ok((message_id == "message-2").then(canonical_test_message))
    }
    async fn query_message_page(
        &self,
        peer: &str,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<MessagePage, IpcError> {
        if cursor == Some("stale") {
            return Err(IpcError::Conflict { message: "cursor_stale".into() });
        }
        let mut page = MessagePage::default();
        page.messages = self.query_messages(peer, limit, None).await?;
        if cursor.is_none() {
            page.next_cursor = Some("message-cursor".into());
        }
        Ok(page)
    }
    async fn search_messages(
        &self,
        query: &str,
        _peer: Option<&str>,
        _limit: u32,
    ) -> Result<Vec<MessageInfo>, IpcError> {
        if query == "denied" {
            return Err(IpcError::Denied { capability: "messaging.history.read".into() });
        }
        if query == "oversized-page" {
            return Ok(vec![maximum_accepted_message(), maximum_accepted_message()]);
        }
        Ok(vec![canonical_test_message()])
    }
    async fn query_attachment(&self, _id: &str) -> Result<Vec<u8>, IpcError> {
        Ok(vec![0, 1, 2, 3])
    }
    async fn list_attachments(&self, message_id: &str) -> Result<Vec<AttachmentInfo>, IpcError> {
        let mut info = AttachmentInfo::default();
        info.ordinal = 0;
        info.id = "sha256:test".into();
        info.name = format!("{message_id}.bin");
        info.size = 4;
        info.checksum = "sha256:test".into();
        info.availability = "available".into();
        info.integrity = "verified".into();
        let mut transfer = AttachmentTransferInfo::default();
        transfer.message_id = message_id.into();
        transfer.transfer_id = "transfer-1".into();
        transfer.resource_hash = Some("11".repeat(32));
        transfer.representation = "resource".into();
        transfer.direction = "outbound".into();
        transfer.state = "transferring".into();
        transfer.transferred = 2;
        transfer.total = 4;
        transfer.cancellable = true;
        info.transfer = Some(Box::new(transfer));
        Ok(vec![info])
    }
    async fn query_attachment_chunk(
        &self,
        message_id: &str,
        ordinal: u8,
        offset: u64,
        max_bytes: u32,
    ) -> Result<AttachmentChunk, IpcError> {
        if ordinal != 0 || offset > 4 || max_bytes == 0 || max_bytes > 256 * 1024 {
            return Err(IpcError::invalid_request("invalid attachment range"));
        }
        let bytes = [0, 1, 2, 3];
        let end = (offset as usize + max_bytes as usize).min(bytes.len());
        let mut chunk = AttachmentChunk::default();
        chunk.attachment = self.list_attachments(message_id).await?.remove(0);
        chunk.data = bytes[offset as usize..end].to_vec();
        chunk.next_offset = end as u64;
        chunk.done = end == bytes.len();
        Ok(chunk)
    }
    async fn set_contact(
        &self,
        peer: &str,
        alias: Option<&str>,
        notes: Option<&str>,
    ) -> Result<ContactInfo, IpcError> {
        let mut contact = ContactInfo::default();
        contact.peer_hash = peer.into();
        contact.alias = alias.map(str::to_owned);
        contact.notes = notes.map(str::to_owned);
        contact.created_at = Some(10);
        contact.updated_at = Some(11);
        Ok(contact)
    }
    async fn remove_contact(&self, _peer: &str) -> Result<bool, IpcError> {
        Ok(false)
    }
    async fn query_contacts(&self) -> Result<Vec<ContactInfo>, IpcError> {
        Ok(vec![self.set_contact(&"11".repeat(16), Some("alias"), Some("notes")).await?])
    }
    async fn resolve_name(
        &self,
        _name: &str,
        _prefix: Option<&str>,
    ) -> Result<Option<PeerHash>, IpcError> {
        Ok(None)
    }
    async fn pin_conversation(&self, _peer: &str) -> Result<bool, IpcError> {
        Ok(true)
    }
    async fn unpin_conversation(&self, _peer: &str) -> Result<bool, IpcError> {
        Ok(true)
    }
    async fn mute_conversation(&self, _peer: &str) -> Result<bool, IpcError> {
        Ok(true)
    }
    async fn unmute_conversation(&self, _peer: &str) -> Result<bool, IpcError> {
        Ok(true)
    }
}

#[async_trait]
impl DaemonFleet for TestDaemon {
    async fn device_status(
        &self,
        _dest: &str,
        _timeout: Option<u64>,
    ) -> Result<RemoteStatusInfo, IpcError> {
        Err(IpcError::not_implemented("fleet"))
    }
    async fn exec(
        &self,
        _dest: &str,
        _cmd: &str,
        _args: Vec<String>,
        _timeout: Option<u64>,
    ) -> Result<ExecResult, IpcError> {
        Err(IpcError::not_implemented("fleet"))
    }
    async fn reboot_device(
        &self,
        _dest: &str,
        _delay: Option<u64>,
        _timeout: Option<u64>,
    ) -> Result<RebootResult, IpcError> {
        Err(IpcError::not_implemented("fleet"))
    }
    async fn self_update(
        &self,
        _dest: &str,
        _version: Option<&str>,
        _timeout: Option<u64>,
    ) -> Result<SelfUpdateResult, IpcError> {
        Err(IpcError::not_implemented("fleet"))
    }
    async fn remote_inbox(
        &self,
        _dest: &str,
        _limit: u32,
        _timeout: Option<u64>,
    ) -> Result<Vec<ConversationInfo>, IpcError> {
        Err(IpcError::not_implemented("fleet"))
    }
    async fn remote_messages(
        &self,
        _dest: &str,
        _peer_hash: &str,
        _limit: u32,
        _timeout: Option<u64>,
    ) -> Result<Vec<MessageInfo>, IpcError> {
        Err(IpcError::not_implemented("fleet"))
    }
    async fn terminal_open(&self, _req: TerminalOpenRequest) -> Result<SessionId, IpcError> {
        Err(IpcError::not_implemented("fleet"))
    }
    async fn terminal_input(&self, _session: &str, _data: &[u8]) -> Result<bool, IpcError> {
        Err(IpcError::not_implemented("fleet"))
    }
    async fn terminal_resize(
        &self,
        _session: &str,
        _rows: u16,
        _cols: u16,
    ) -> Result<bool, IpcError> {
        Err(IpcError::not_implemented("fleet"))
    }
    async fn terminal_close(&self, _session: &str) -> Result<bool, IpcError> {
        Err(IpcError::not_implemented("fleet"))
    }
    async fn fleet_apply(
        &self,
        _dest: &str,
        _profile_bytes: Vec<u8>,
        _verify: bool,
        _timeout: Option<u64>,
    ) -> Result<ConfigApplyResult, IpcError> {
        Err(IpcError::not_implemented("fleet"))
    }
    async fn fleet_grant(
        &self,
        _identity_hash: &str,
        _role: &str,
        _label: &str,
        _grants: Vec<String>,
    ) -> Result<bool, IpcError> {
        Err(IpcError::not_implemented("fleet"))
    }
    async fn fleet_revoke(&self, _identity_hash: &str) -> Result<bool, IpcError> {
        Err(IpcError::not_implemented("fleet"))
    }
}

#[async_trait]
impl DaemonPages for TestDaemon {
    async fn browse_page(
        &self,
        _host: &str,
        _path: &str,
        _timeout: Option<u64>,
    ) -> Result<PageContent, IpcError> {
        Err(IpcError::not_implemented("browse_page"))
    }
    async fn browse_page_for_owner(
        &self,
        owner: u64,
        host: &str,
        path: &str,
        _timeout: Option<u64>,
    ) -> Result<PageContent, IpcError> {
        if self.panic_browse.swap(false, std::sync::atomic::Ordering::AcqRel) {
            self.pages
                .lock()
                .expect("page test state lock")
                .sessions
                .insert("panic-session".into(), owner);
            panic!("scripted browse panic");
        }
        let blocked = self.blocked_browse.lock().expect("blocked browse lock").clone();
        if let Some(blocked) = blocked {
            blocked.owner.store(owner, std::sync::atomic::Ordering::Release);
            self.pages
                .lock()
                .expect("page test state lock")
                .sessions
                .insert("blocked-session".into(), owner);
            let _guard = BlockedBrowseGuard(Arc::clone(&blocked));
            blocked.entered.notify_waiters();
            std::future::pending::<()>().await;
            unreachable!("blocked browse is cancelled by physical disconnect");
        }
        let mut state = self.pages.lock().expect("page test state lock");
        state.next_id += 1;
        let session_id = format!("session-{}", state.next_id);
        state.sessions.insert(session_id.clone(), owner);
        let mut page = PageContent::default();
        page.correlation_id = format!("page-{}", state.next_id);
        page.host_hash = host.into();
        page.request.native_path = path.into();
        page.navigation.session_id = session_id;
        page.navigation.address = format!("{host}:{path}");
        let mut password = PageFormField::default();
        password.name = "password".into();
        password.kind = PageFormFieldKind::Password;
        page.fields.push(password);
        Ok(page)
    }
    async fn navigate_page_for_owner(
        &self,
        owner: u64,
        request: PageNavigationRequest,
    ) -> Result<PageContent, IpcError> {
        let state = self.pages.lock().expect("page test state lock");
        let session_id = request
            .session_id
            .as_deref()
            .ok_or_else(|| IpcError::invalid_request("session required"))?;
        if state.sessions.get(session_id) != Some(&owner) {
            return Err(IpcError::invalid_request("session owner mismatch"));
        }
        drop(state);
        let mut page = PageContent::default();
        page.correlation_id = "navigated".into();
        page.navigation.session_id = session_id.into();
        page.navigation.address = request.target.unwrap_or_else(|| "/page/index.mu".into());
        Ok(page)
    }
    async fn close_page_session_for_owner(
        &self,
        owner: u64,
        session_id: &str,
    ) -> Result<PageNavigationInfo, IpcError> {
        let mut state = self.pages.lock().expect("page test state lock");
        if state.sessions.get(session_id) != Some(&owner) {
            return Err(IpcError::invalid_request("session owner mismatch"));
        }
        state.sessions.remove(session_id);
        let mut info = PageNavigationInfo::default();
        info.session_id = session_id.into();
        Ok(info)
    }
    async fn start_file_download_for_owner(
        &self,
        owner: u64,
        _request: FileDownloadRequest,
    ) -> Result<FileDownloadInfo, IpcError> {
        let mut state = self.pages.lock().expect("page test state lock");
        state.next_id += 1;
        let mut info = FileDownloadInfo::default();
        info.download_id = format!("download-{}", state.next_id);
        info.state = FileDownloadState::Completed;
        info.integrity_verified = true;
        state.downloads.insert(info.download_id.clone(), (owner, info.clone()));
        Ok(info)
    }
    async fn file_download_for_owner(
        &self,
        owner: u64,
        download_id: &str,
    ) -> Result<FileDownloadInfo, IpcError> {
        let state = self.pages.lock().expect("page test state lock");
        state
            .downloads
            .get(download_id)
            .filter(|(record_owner, _)| *record_owner == owner)
            .map(|(_, info)| info.clone())
            .ok_or_else(|| IpcError::invalid_request("download owner mismatch"))
    }
    async fn cancel_file_download_for_owner(
        &self,
        owner: u64,
        download_id: &str,
    ) -> Result<FileDownloadInfo, IpcError> {
        let mut state = self.pages.lock().expect("page test state lock");
        let (record_owner, info) = state
            .downloads
            .get_mut(download_id)
            .ok_or_else(|| IpcError::invalid_request("download missing"))?;
        if *record_owner != owner {
            return Err(IpcError::invalid_request("download owner mismatch"));
        }
        info.state = FileDownloadState::Cancelled;
        Ok(info.clone())
    }
    async fn save_file_download_for_owner(
        &self,
        owner: u64,
        download_id: &str,
        destination: &str,
    ) -> Result<FileDownloadInfo, IpcError> {
        let mut state = self.pages.lock().expect("page test state lock");
        let (record_owner, info) = state
            .downloads
            .get_mut(download_id)
            .ok_or_else(|| IpcError::invalid_request("download missing"))?;
        if *record_owner != owner {
            return Err(IpcError::invalid_request("download owner mismatch"));
        }
        std::fs::write(destination, b"download")
            .map_err(|error| IpcError::invalid_request(error.to_string()))?;
        info.state = FileDownloadState::Saved;
        info.saved_path = Some(destination.into());
        Ok(info.clone())
    }
    async fn cleanup_page_owner(&self, owner: u64) -> Result<(), IpcError> {
        self.cleaned_owners.lock().expect("cleaned owners lock").push(owner);
        if self
            .cleanup_failures
            .fetch_update(
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
                |remaining| if remaining > 0 { Some(remaining - 1) } else { None },
            )
            .is_ok()
        {
            return Err(IpcError::Internal { message: "scripted owner cleanup failure".into() });
        }
        let mut state = self.pages.lock().expect("page test state lock");
        state.sessions.retain(|_, session_owner| *session_owner != owner);
        state.downloads.retain(|_, (download_owner, _)| *download_owner != owner);
        if let Some(blocked) = self.blocked_browse.lock().expect("blocked browse lock").as_ref()
            && blocked.owner.load(std::sync::atomic::Ordering::Acquire) == owner
        {
            blocked.owner_cleaned.store(true, std::sync::atomic::Ordering::Release);
        }
        Ok(())
    }
    async fn list_pages(
        &self,
        _host: &str,
        _timeout: Option<u64>,
    ) -> Result<Vec<PageInfo>, IpcError> {
        Ok(vec![])
    }
    async fn page_hosts(&self) -> Result<Vec<DeviceInfo>, IpcError> {
        Ok(vec![])
    }
}

#[async_trait]
impl DaemonEvents for TestDaemon {
    async fn link_snapshot(&self) -> Result<styrene_ipc::types::LinkSnapshot, IpcError> {
        use styrene_ipc::types::{LinkActivity, LinkEvent, LinkEventKind, LinkSnapshot};

        let active = LinkEvent::new("link-active", "peer-active", "active", Some(4.5));
        let mut historical = LinkEvent::new("link-closed", "peer-closed", "closed", Some(8.0));
        historical.kind = LinkEventKind::Teardown;
        historical.activity = LinkActivity::Historical;
        let mut snapshot = LinkSnapshot::default();
        snapshot.active.push(active);
        snapshot.history.push(historical);
        Ok(snapshot)
    }

    async fn subscribe_messages(
        &self,
        _peers: &[String],
    ) -> Result<tokio::sync::broadcast::Receiver<DaemonEvent>, IpcError> {
        let (tx, rx) = tokio::sync::broadcast::channel(16);
        drop(tx);
        Ok(rx)
    }
    async fn subscribe_devices(
        &self,
    ) -> Result<tokio::sync::broadcast::Receiver<DaemonEvent>, IpcError> {
        let (tx, rx) = tokio::sync::broadcast::channel(16);
        drop(tx);
        Ok(rx)
    }
    async fn subscribe_links(
        &self,
    ) -> Result<tokio::sync::broadcast::Receiver<DaemonEvent>, IpcError> {
        let (tx, rx) = tokio::sync::broadcast::channel(16);
        drop(tx);
        Ok(rx)
    }
    async fn subscribe_routes(
        &self,
    ) -> Result<tokio::sync::broadcast::Receiver<DaemonEvent>, IpcError> {
        let (tx, rx) = tokio::sync::broadcast::channel(16);
        drop(tx);
        Ok(rx)
    }

    async fn start_request(
        &self,
        _request: StartRequestInfo,
    ) -> Result<RequestObservationInfo, IpcError> {
        Ok(test_request(RequestState::Pending))
    }

    async fn request_receipt(
        &self,
        request_id: &str,
    ) -> Result<Option<RequestObservationInfo>, IpcError> {
        Ok((request_id == "aa".repeat(16)).then(|| test_request(RequestState::Succeeded)))
    }

    async fn request_receipts(&self) -> Result<Vec<RequestObservationInfo>, IpcError> {
        Ok(vec![test_request(RequestState::Succeeded)])
    }

    async fn cancel_request(&self, _request_id: &str) -> Result<RequestObservationInfo, IpcError> {
        Ok(test_request(RequestState::Cancelled))
    }

    async fn resource_transfers(&self) -> Result<Vec<ResourceTransferInfo>, IpcError> {
        Ok(vec![test_resource()])
    }

    async fn cancel_resource(&self, resource_hash: &str) -> Result<bool, IpcError> {
        Ok(resource_hash == "ee".repeat(32))
    }

    async fn start_network_operation(
        &self,
        _request: StartNetworkOperationInfo,
    ) -> Result<NetworkOperationInfo, IpcError> {
        Ok(test_network_operation(None))
    }

    async fn network_operation(
        &self,
        operation_id: &str,
    ) -> Result<Option<NetworkOperationInfo>, IpcError> {
        Ok((operation_id == "dd".repeat(16))
            .then(|| test_network_operation(Some(NetworkOperationOutcome::Succeeded))))
    }

    async fn network_operations(&self) -> Result<Vec<NetworkOperationInfo>, IpcError> {
        Ok(vec![test_network_operation(Some(NetworkOperationOutcome::Succeeded))])
    }

    async fn cancel_network_operation(
        &self,
        _operation_id: &str,
    ) -> Result<NetworkOperationInfo, IpcError> {
        Ok(test_network_operation(Some(NetworkOperationOutcome::Cancelled)))
    }
}

#[async_trait]
impl DaemonTunnel for TestDaemon {
    async fn list_tunnels(&self) -> Result<Vec<TunnelInfo>, IpcError> {
        Ok(vec![])
    }
    async fn tunnel_status(&self, _peer: &str) -> Result<TunnelInfo, IpcError> {
        Err(IpcError::not_implemented("tunnel"))
    }
    async fn tunnel_rekey(&self, _peer: &str) -> Result<bool, IpcError> {
        Err(IpcError::not_implemented("tunnel"))
    }
    async fn tunnel_teardown(&self, _peer: &str) -> Result<bool, IpcError> {
        Err(IpcError::not_implemented("tunnel"))
    }
    async fn list_tunnel_sas(&self, _peer: &str) -> Result<Vec<TunnelSaInfo>, IpcError> {
        Ok(vec![])
    }
    async fn tunnel_establish(&self, _peer: &str) -> Result<String, IpcError> {
        Err(IpcError::not_implemented("tunnel"))
    }
    async fn tunnel_operation(&self, _peer: &str) -> Result<TunnelOperationInfo, IpcError> {
        Err(IpcError::not_implemented("tunnel operation"))
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────

async fn setup_server() -> (IpcServer, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let sock_path = dir.path().join("test.sock");
    let sock = sock_path.clone();
    std::mem::forget(dir);

    let config = IpcServerConfig { socket_path: sock.clone(), event_capacity: 64 };
    let daemon: Arc<dyn Daemon> = Arc::new(TestDaemon::default());
    let mut server = IpcServer::new(daemon, config);
    server.start().await.expect("start");
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    (server, sock)
}

async fn setup_page_server() -> (IpcServer, std::path::PathBuf, Arc<TestDaemon>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let sock = dir.path().join("pages.sock");
    std::mem::forget(dir);
    let daemon = Arc::new(TestDaemon::default());
    let config = IpcServerConfig { socket_path: sock.clone(), event_capacity: 64 };
    let mut server = IpcServer::new(daemon.clone(), config);
    server.start().await.expect("start");
    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    (server, sock, daemon)
}

#[tokio::test]
async fn typed_request_start_query_snapshot_and_cancel_dispatch() {
    let daemon: Arc<dyn Daemon> = Arc::new(TestDaemon::default());
    let mut start = HashMap::new();
    start.insert("link_id".into(), rmpv::Value::from("cc".repeat(16)));
    start.insert("path".into(), rmpv::Value::from("/page/index.mu"));
    start.insert("data".into(), rmpv::Value::Binary(vec![0xc0]));
    start.insert("timeout_ms".into(), rmpv::Value::from(1_000_u64));
    start.insert("max_response_size".into(), rmpv::Value::from(4_096_u64));
    let started =
        styrene_ipc_server::dispatch::dispatch(&daemon, MessageType::CmdRequestStart, start)
            .await
            .expect("start request");
    assert_eq!(started["state"].as_str(), Some("pending"), "{started:?}");

    let request_id = "aa".repeat(16);
    let query = HashMap::from([("request_id".into(), rmpv::Value::from(request_id.clone()))]);
    let receipt =
        styrene_ipc_server::dispatch::dispatch(&daemon, MessageType::QueryRequest, query.clone())
            .await
            .expect("query request");
    assert_eq!(receipt["state"].as_str(), Some("succeeded"));
    assert_eq!(receipt["response"].as_slice(), Some(&[0xc4, 0x01, 0xaa][..]));
    assert_eq!(
        receipt["request_resource_hash"].as_str(),
        Some("dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd")
    );

    let snapshot =
        styrene_ipc_server::dispatch::dispatch(&daemon, MessageType::QueryRequests, HashMap::new())
            .await
            .expect("request snapshot");
    assert_eq!(snapshot["requests"].as_array().map(Vec::len), Some(1));
    let queried = &snapshot["requests"].as_array().expect("request array")[0];
    let response = queried
        .as_map()
        .expect("request map")
        .iter()
        .find(|(key, _)| key.as_str() == Some("response"))
        .map(|(_, value)| value);
    assert_eq!(response.and_then(rmpv::Value::as_slice), Some(&[0xc4, 0x01, 0xaa][..]));

    let cancelled =
        styrene_ipc_server::dispatch::dispatch(&daemon, MessageType::CmdRequestCancel, query)
            .await
            .expect("cancel request");
    assert_eq!(cancelled["state"].as_str(), Some("cancelled"));
}

#[tokio::test]
async fn message_cancellation_dispatches_to_daemon() {
    let daemon: Arc<dyn Daemon> = Arc::new(TestDaemon::default());
    let payload = HashMap::from([("message_id".into(), rmpv::Value::from("message-1"))]);

    let response =
        styrene_ipc_server::dispatch::dispatch(&daemon, MessageType::CmdCancelMessage, payload)
            .await
            .expect("cancel message");

    assert_eq!(response["cancelled"].as_bool(), Some(false));
}

#[tokio::test]
async fn peer_hashes_are_canonical_before_daemon_invocation() {
    let daemon = Arc::new(TestDaemon::default());
    let daemon_api: Arc<dyn Daemon> = daemon.clone();
    for invalid in ["aa".into(), "A".repeat(32), "gg".repeat(16), "aa".repeat(17)] {
        let payload = HashMap::from([("peer_hash".into(), rmpv::Value::from(invalid))]);
        assert!(
            styrene_ipc_server::dispatch::dispatch(&daemon_api, MessageType::CmdMarkRead, payload,)
                .await
                .is_err()
        );
    }
    assert_eq!(daemon.peer_calls.load(std::sync::atomic::Ordering::SeqCst), 0);

    let payload = HashMap::from([("peer_hash".into(), rmpv::Value::from("ab".repeat(16)))]);
    styrene_ipc_server::dispatch::dispatch(&daemon_api, MessageType::CmdMarkRead, payload)
        .await
        .expect("canonical peer hash");
    assert_eq!(daemon.peer_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn typed_network_operation_start_query_and_cancel_dispatch() {
    let daemon: Arc<dyn Daemon> = Arc::new(TestDaemon::default());
    let start = HashMap::from([
        ("kind".into(), rmpv::Value::from("path_request")),
        ("destination_hash".into(), rmpv::Value::from("11".repeat(16))),
        ("timeout_ms".into(), rmpv::Value::from(1_000_u64)),
    ]);
    let started = styrene_ipc_server::dispatch::dispatch_for_connection(
        &daemon,
        MessageType::CmdNetworkOperationStart,
        start,
        7,
    )
    .await
    .expect("start network operation");
    assert_eq!(started["operation_id"].as_str(), Some("dddddddddddddddddddddddddddddddd"));
    assert_eq!(started["progress"].as_str(), Some("awaiting_path"));
    assert_eq!(started["connection_generation"].as_u64(), Some(7));
    assert_eq!(started["correlation_id"].as_str(), started["operation_id"].as_str());

    let query = HashMap::from([("operation_id".into(), rmpv::Value::from("dd".repeat(16)))]);
    let completed = styrene_ipc_server::dispatch::dispatch(
        &daemon,
        MessageType::QueryNetworkOperation,
        query.clone(),
    )
    .await
    .expect("query network operation");
    assert_eq!(completed["outcome"].as_str(), Some("succeeded"));

    let cancelled = styrene_ipc_server::dispatch::dispatch(
        &daemon,
        MessageType::CmdNetworkOperationCancel,
        query,
    )
    .await
    .expect("cancel network operation");
    assert_eq!(cancelled["outcome"].as_str(), Some("cancelled"));
}

#[tokio::test]
async fn request_operations_share_physical_connection_generation() {
    let (mut server, sock) = setup_server().await;
    let mut stream = UnixStream::connect(&sock).await.expect("connect");
    let mut start = HashMap::new();
    start.insert("link_id".into(), rmpv::Value::from("cc".repeat(16)));
    start.insert("path".into(), rmpv::Value::from("/page/index.mu"));
    start.insert("data".into(), rmpv::Value::Binary(vec![0xc0]));
    start.insert("timeout_ms".into(), rmpv::Value::from(1_000_u64));
    start.insert("max_response_size".into(), rmpv::Value::from(4_096_u64));
    let started = send_and_recv(&mut stream, MessageType::CmdRequestStart, &start).await;
    let request_id = "aa".repeat(16);
    let query = HashMap::from([("request_id".into(), rmpv::Value::from(request_id))]);
    let queried = send_and_recv(&mut stream, MessageType::QueryRequest, &query).await;
    let listed = send_and_recv(&mut stream, MessageType::QueryRequests, &HashMap::new()).await;
    let cancelled = send_and_recv(&mut stream, MessageType::CmdRequestCancel, &query).await;

    let generation = started.payload["connection_generation"].as_u64().expect("generation");
    assert_eq!(queried.payload["connection_generation"].as_u64(), Some(generation));
    assert_eq!(cancelled.payload["connection_generation"].as_u64(), Some(generation));
    let listed_request = &listed.payload["requests"].as_array().expect("requests")[0];
    let listed_generation = listed_request
        .as_map()
        .expect("request map")
        .iter()
        .find(|(key, _)| key.as_str() == Some("connection_generation"))
        .and_then(|(_, value)| value.as_u64());
    assert_eq!(listed_generation, Some(generation));
    server.stop().await;
}

fn gen_request_id() -> [u8; REQUEST_ID_SIZE] {
    let mut id = [0u8; REQUEST_ID_SIZE];
    id[0] = 42;
    id[15] = 99;
    id
}

async fn send_and_recv(
    stream: &mut UnixStream,
    msg_type: MessageType,
    payload: &HashMap<String, rmpv::Value>,
) -> wire::Frame {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let req_id = gen_request_id();
    let bytes = wire::encode_frame(msg_type, &req_id, payload).expect("encode");
    stream.write_all(&bytes).await.expect("write");
    stream.flush().await.expect("flush");

    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await.expect("read len");
    let total = u32::from_be_bytes(len_buf) as usize;
    let mut frame_buf = vec![0u8; total];
    stream.read_exact(&mut frame_buf).await.expect("read frame");

    let mut full = Vec::with_capacity(4 + total);
    full.extend_from_slice(&len_buf);
    full.extend_from_slice(&frame_buf);
    wire::decode_frame(&full).expect("decode")
}

fn typed_value<T: serde::de::DeserializeOwned>(frame: &wire::Frame, key: &str) -> T {
    let bytes = frame.payload[key].as_slice().expect("typed response bytes");
    rmp_serde::from_slice(bytes).expect("typed response")
}

// ── Tests ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn unix_socket_page_workflow_is_owner_scoped_and_secret_safe() {
    let (mut server, sock, daemon) = setup_page_server().await;
    let mut owner = UnixStream::connect(&sock).await.expect("owner connect");
    let mut other = UnixStream::connect(&sock).await.expect("other connect");
    let page_frame = send_and_recv(
        &mut owner,
        MessageType::QueryPage,
        &HashMap::from([
            ("host".into(), rmpv::Value::from("local")),
            ("path".into(), rmpv::Value::from("/page/index.mu")),
        ]),
    )
    .await;
    let page: PageContent = typed_value(&page_frame, "page");
    let session_id = page.navigation.session_id.clone();

    let mut submission = PageFormSubmission::default();
    submission.values.insert("password".into(), vec!["socket-secret".into()]);
    let mut navigation = PageNavigationRequest::default();
    navigation.session_id = Some(session_id.clone());
    navigation.target = Some("/page/next.mu".into());
    navigation.submission = Some(submission);
    let navigation_bytes = rmp_serde::to_vec_named(&navigation).unwrap();
    let denied = send_and_recv(
        &mut other,
        MessageType::CmdPageNavigate,
        &HashMap::from([("navigation".into(), rmpv::Value::Binary(navigation_bytes.clone()))]),
    )
    .await;
    assert_eq!(denied.msg_type, MessageType::Error);
    let navigated = send_and_recv(
        &mut owner,
        MessageType::CmdPageNavigate,
        &HashMap::from([("navigation".into(), rmpv::Value::Binary(navigation_bytes))]),
    )
    .await;
    assert_eq!(navigated.msg_type, MessageType::Result);
    let encoded_response =
        wire::encode_frame(navigated.msg_type, &navigated.request_id, &navigated.payload).unwrap();
    assert!(
        !encoded_response.windows(b"socket-secret".len()).any(|window| window == b"socket-secret")
    );

    let mut request = FileDownloadRequest::default();
    request.session_id = Some(session_id.clone());
    request.target = "/file/data.bin".into();
    let request = rmp_serde::to_vec_named(&request).unwrap();
    let first = send_and_recv(
        &mut owner,
        MessageType::CmdFileDownloadStart,
        &HashMap::from([("download_request".into(), rmpv::Value::Binary(request.clone()))]),
    )
    .await;
    let first: FileDownloadInfo = typed_value(&first, "download");
    let second = send_and_recv(
        &mut owner,
        MessageType::CmdFileDownloadStart,
        &HashMap::from([("download_request".into(), rmpv::Value::Binary(request))]),
    )
    .await;
    let second: FileDownloadInfo = typed_value(&second, "download");
    let queried = send_and_recv(
        &mut owner,
        MessageType::QueryFileDownload,
        &HashMap::from([("download_id".into(), rmpv::Value::from(second.download_id.clone()))]),
    )
    .await;
    let queried: FileDownloadInfo = typed_value(&queried, "download");
    assert_eq!(queried.download_id, second.download_id);
    let cancelled = send_and_recv(
        &mut owner,
        MessageType::CmdFileDownloadCancel,
        &HashMap::from([("download_id".into(), rmpv::Value::from(first.download_id))]),
    )
    .await;
    let cancelled: FileDownloadInfo = typed_value(&cancelled, "download");
    assert_eq!(cancelled.state, FileDownloadState::Cancelled);

    let root = tempfile::tempdir().unwrap();
    let destination = root.path().join("saved.bin");
    let saved = send_and_recv(
        &mut owner,
        MessageType::CmdFileDownloadSave,
        &HashMap::from([
            ("download_id".into(), rmpv::Value::from(second.download_id)),
            ("destination".into(), rmpv::Value::from(destination.to_string_lossy().as_ref())),
        ]),
    )
    .await;
    let saved: FileDownloadInfo = typed_value(&saved, "download");
    assert_eq!(saved.state, FileDownloadState::Saved);
    assert_eq!(std::fs::read(destination).unwrap(), b"download");

    let closed = send_and_recv(
        &mut owner,
        MessageType::CmdPageDisconnect,
        &HashMap::from([("session_id".into(), rmpv::Value::from(session_id))]),
    )
    .await;
    assert_eq!(closed.msg_type, MessageType::Result);

    let orphan = send_and_recv(
        &mut owner,
        MessageType::QueryPage,
        &HashMap::from([
            ("host".into(), rmpv::Value::from("local")),
            ("path".into(), rmpv::Value::from("/page/orphan.mu")),
        ]),
    )
    .await;
    let orphan: PageContent = typed_value(&orphan, "page");
    drop(owner);
    tokio::time::timeout(tokio::time::Duration::from_secs(1), async {
        loop {
            if !daemon.pages.lock().unwrap().sessions.contains_key(&orphan.navigation.session_id) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("disconnect owner cleanup");
    server.stop().await;
}

#[tokio::test]
async fn physical_disconnect_cancels_blocked_browse_stages_and_cleans_owner() {
    use tokio::io::AsyncWriteExt;

    for stage in [BlockedBrowseStage::Path, BlockedBrowseStage::Link, BlockedBrowseStage::Transfer]
    {
        let (mut server, sock, daemon) = setup_page_server().await;
        let blocked = Arc::new(BlockedBrowse::new(stage));
        *daemon.blocked_browse.lock().unwrap() = Some(Arc::clone(&blocked));
        let mut stream = UnixStream::connect(&sock).await.expect("connect blocked browse");
        let request = wire::encode_frame(
            MessageType::QueryPage,
            &gen_request_id(),
            &HashMap::from([
                ("host".into(), rmpv::Value::from("local")),
                ("path".into(), rmpv::Value::from("/page/blocked.mu")),
            ]),
        )
        .expect("encode blocked browse");
        let entered = blocked.entered.notified();
        stream.write_all(&request).await.expect("write blocked browse");
        stream.flush().await.expect("flush blocked browse");
        tokio::time::timeout(tokio::time::Duration::from_secs(1), entered)
            .await
            .expect("browse dispatch entered blocked stage");

        drop(stream);
        tokio::time::timeout(tokio::time::Duration::from_secs(1), async {
            loop {
                let cancelled = blocked.cancelled.load(std::sync::atomic::Ordering::Acquire);
                let cleaned = blocked.owner_cleaned.load(std::sync::atomic::Ordering::Acquire);
                let state_removed =
                    !daemon.pages.lock().unwrap().sessions.contains_key("blocked-session");
                if cancelled && cleaned && state_removed {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("disconnect cancelled dispatch and removed owner state");
        assert_eq!(
            blocked.created_link_closed.load(std::sync::atomic::Ordering::Acquire),
            blocked.created_link(),
            "Created-link disposition was not cleaned at {stage:?} stage"
        );
        server.stop().await;
    }
}

#[tokio::test]
async fn owner_cleanup_guard_covers_connection_termination_and_dispatch_panic() {
    use tokio::io::AsyncWriteExt;

    for panic_dispatch in [false, true] {
        let (mut server, sock, daemon) = setup_page_server().await;
        daemon.panic_browse.store(panic_dispatch, std::sync::atomic::Ordering::Release);
        let std_stream =
            std::os::unix::net::UnixStream::connect(&sock).expect("connect cleanup guard");
        std_stream.shutdown(std::net::Shutdown::Read).expect("shutdown client read direction");
        std_stream.set_nonblocking(true).expect("nonblocking cleanup guard socket");
        let mut stream = UnixStream::from_std(std_stream).expect("tokio cleanup guard socket");
        let request = wire::encode_frame(
            MessageType::QueryPage,
            &gen_request_id(),
            &HashMap::from([
                ("host".into(), rmpv::Value::from("local")),
                ("path".into(), rmpv::Value::from("/page/guard.mu")),
            ]),
        )
        .expect("encode cleanup guard request");
        stream.write_all(&request).await.expect("write cleanup guard request");
        stream.flush().await.expect("flush cleanup guard request");
        if !panic_dispatch {
            stream.shutdown().await.expect("close cleanup guard connection");
        }

        tokio::time::timeout(tokio::time::Duration::from_secs(1), async {
            while daemon.cleaned_owners.lock().unwrap().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!("owner cleanup after connection termination or panic={panic_dispatch}")
        });
        assert!(daemon.pages.lock().unwrap().sessions.is_empty());
        server.stop().await;
    }
}

#[tokio::test]
async fn owner_cleanup_guard_covers_writer_termination() {
    let daemon = Arc::new(TestDaemon::default());
    daemon.pages.lock().unwrap().sessions.insert("writer-session".into(), 66);
    let (server_stream, _client_stream) = UnixStream::pair().expect("writer socket pair");
    let (read_half, write_half) = server_stream.into_split();
    let (event_tx, event_rx) = tokio::sync::broadcast::channel(1);
    drop(event_tx);
    let daemon_trait: Arc<dyn Daemon> = daemon.clone();

    tokio::time::timeout(
        tokio::time::Duration::from_secs(1),
        styrene_ipc_server::connection::handle_client_with_generation(
            daemon_trait,
            read_half,
            write_half,
            event_rx,
            66,
        ),
    )
    .await
    .expect("connection exits when writer terminates");
    assert!(!daemon.pages.lock().unwrap().sessions.contains_key("writer-session"));
}

#[tokio::test]
async fn owner_cleanup_guard_covers_connection_task_cancellation() {
    let daemon = Arc::new(TestDaemon::default());
    daemon.pages.lock().unwrap().sessions.insert("cancelled-session".into(), 77);
    let (server_stream, _client_stream) = UnixStream::pair().expect("socket pair");
    let (read_half, write_half) = server_stream.into_split();
    let (_event_tx, event_rx) = tokio::sync::broadcast::channel(1);
    let daemon_trait: Arc<dyn Daemon> = daemon.clone();
    let task = tokio::spawn(styrene_ipc_server::connection::handle_client_with_generation(
        daemon_trait,
        read_half,
        write_half,
        event_rx,
        77,
    ));
    tokio::task::yield_now().await;
    task.abort();
    let _ = task.await;

    tokio::time::timeout(tokio::time::Duration::from_secs(1), async {
        while !daemon.cleaned_owners.lock().unwrap().contains(&77) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("owner cleanup after connection task cancellation");
    assert!(!daemon.pages.lock().unwrap().sessions.contains_key("cancelled-session"));
}

#[tokio::test]
async fn owner_cleanup_guard_retries_instead_of_disarming_after_failure() {
    let daemon = Arc::new(TestDaemon::default());
    daemon.pages.lock().unwrap().sessions.insert("retry-session".into(), 99);
    daemon.cleanup_failures.store(1, std::sync::atomic::Ordering::Release);
    let (server_stream, client_stream) = UnixStream::pair().expect("retry socket pair");
    let (read_half, write_half) = server_stream.into_split();
    let (_event_tx, event_rx) = tokio::sync::broadcast::channel(1);
    let daemon_trait: Arc<dyn Daemon> = daemon.clone();
    let task = tokio::spawn(styrene_ipc_server::connection::handle_client_with_generation(
        daemon_trait,
        read_half,
        write_half,
        event_rx,
        99,
    ));
    drop(client_stream);
    task.await.expect("connection cleanup task");

    tokio::time::timeout(tokio::time::Duration::from_secs(1), async {
        loop {
            let attempts =
                daemon.cleaned_owners.lock().unwrap().iter().filter(|owner| **owner == 99).count();
            if attempts >= 2 && !daemon.pages.lock().unwrap().sessions.contains_key("retry-session")
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("armed cleanup guard retried retained owner state");
}

#[tokio::test]
async fn ping_pong() {
    let (mut server, sock) = setup_server().await;
    let mut stream = UnixStream::connect(&sock).await.expect("connect");
    let resp = send_and_recv(&mut stream, MessageType::Ping, &HashMap::new()).await;
    assert_eq!(resp.msg_type, MessageType::Pong);
    server.stop().await;
}

#[tokio::test]
async fn legacy_and_chunked_query_attachment_return_binary_over_socket() {
    let (mut server, sock) = setup_server().await;
    let mut stream = UnixStream::connect(&sock).await.expect("connect");
    let legacy = HashMap::from([("message_id".into(), rmpv::Value::from("message"))]);
    let response = send_and_recv(&mut stream, MessageType::QueryAttachment, &legacy).await;
    assert_eq!(response.msg_type, MessageType::Result);
    assert_eq!(
        response.payload.get("data").and_then(rmpv::Value::as_slice),
        Some([0, 1, 2, 3].as_slice())
    );

    let chunked = HashMap::from([
        ("message_id".into(), rmpv::Value::from("message")),
        ("ordinal".into(), rmpv::Value::from(0)),
        ("offset".into(), rmpv::Value::from(1)),
        ("max_bytes".into(), rmpv::Value::from(2)),
    ]);
    let response = send_and_recv(&mut stream, MessageType::QueryAttachment, &chunked).await;
    assert_eq!(response.msg_type, MessageType::Result);
    assert_eq!(
        response.payload.get("data").and_then(rmpv::Value::as_slice),
        Some([1, 2].as_slice())
    );
    assert_eq!(response.payload.get("next_offset").and_then(rmpv::Value::as_u64), Some(3));
    assert_eq!(response.payload.get("done").and_then(rmpv::Value::as_bool), Some(false));
    server.stop().await;
}

#[tokio::test]
async fn attachment_transfer_query_and_cancel_are_audited_over_socket() {
    let (mut server, sock) = setup_server().await;
    let mut stream = UnixStream::connect(&sock).await.expect("connect");
    let payload = HashMap::from([("message_id".into(), rmpv::Value::from("message"))]);
    let response = send_and_recv(&mut stream, MessageType::QueryAttachmentTransfer, &payload).await;
    assert_eq!(response.msg_type, MessageType::Result);
    let transfer = response
        .payload
        .get("attachment_transfer")
        .and_then(rmpv::Value::as_map)
        .expect("typed transfer");
    assert!(transfer.iter().any(|(key, value)| {
        key.as_str() == Some("message_id") && value.as_str() == Some("message")
    }));
    let response =
        send_and_recv(&mut stream, MessageType::CmdAttachmentTransferCancel, &payload).await;
    assert_eq!(response.msg_type, MessageType::Result);
    assert_eq!(response.payload.get("success").and_then(rmpv::Value::as_bool), Some(false));
    assert!(response.payload.contains_key("outcome"));
    server.stop().await;
}

#[tokio::test]
async fn additive_send_outcome_and_draft_dispatch_are_typed_and_redacted() {
    let daemon: Arc<dyn Daemon> = Arc::new(TestDaemon::default());
    let peer = "11".repeat(16);
    let send = styrene_ipc_server::dispatch::dispatch(
        &daemon,
        MessageType::CmdSendChatOutcome,
        HashMap::from([
            ("peer_hash".into(), rmpv::Value::from(peer.as_str())),
            ("content".into(), rmpv::Value::from("retained")),
            ("delivery_method".into(), rmpv::Value::from("paper")),
        ]),
    )
    .await
    .unwrap();
    assert_eq!(send["message_id"].as_str(), Some("persisted-message"));
    let outcome = send["outcome"].as_map().unwrap();
    assert!(outcome.iter().all(|(key, _)| key.as_str() != Some("uri")));

    let saved = styrene_ipc_server::dispatch::dispatch(
        &daemon,
        MessageType::CmdSetDraft,
        HashMap::from([
            ("peer_hash".into(), rmpv::Value::from(peer.as_str())),
            ("content".into(), rmpv::Value::from("retained")),
        ]),
    )
    .await
    .unwrap();
    assert!(saved["draft"].as_map().is_some());
    let queried = styrene_ipc_server::dispatch::dispatch(
        &daemon,
        MessageType::QueryDraft,
        HashMap::from([("peer_hash".into(), rmpv::Value::from(peer.as_str()))]),
    )
    .await
    .unwrap();
    assert!(queried["draft"].as_map().is_some());
}

#[tokio::test]
async fn malformed_send_chat_attachments_never_invoke_daemon() {
    let daemon = Arc::new(TestDaemon::default());
    let daemon_api: Arc<dyn Daemon> = daemon.clone();
    for malformed in [
        HashMap::from([
            ("peer_hash".into(), rmpv::Value::from("11".repeat(16))),
            ("content".into(), rmpv::Value::from("body")),
            ("attachment_name".into(), rmpv::Value::from("orphan.bin")),
        ]),
        HashMap::from([
            ("peer_hash".into(), rmpv::Value::from("11".repeat(16))),
            ("content".into(), rmpv::Value::from("body")),
            (
                "attachments".into(),
                rmpv::Value::Array(vec![rmpv::Value::Map(vec![
                    (rmpv::Value::from("name"), rmpv::Value::from("bad.bin")),
                    (rmpv::Value::from("bytes"), rmpv::Value::Binary(vec![1])),
                    (rmpv::Value::from("content_type"), rmpv::Value::from(7)),
                ])]),
            ),
        ]),
        HashMap::from([
            ("peer_hash".into(), rmpv::Value::from("11".repeat(16))),
            ("content".into(), rmpv::Value::from("body")),
            (
                "attachments".into(),
                rmpv::Value::Array(vec![rmpv::Value::Map(vec![
                    (rmpv::Value::from("name"), rmpv::Value::from("bad.bin")),
                    (rmpv::Value::from("bytes"), rmpv::Value::Binary(vec![1])),
                    (rmpv::Value::from("expected_sha256"), rmpv::Value::from("ABC")),
                ])]),
            ),
        ]),
    ] {
        assert!(
            styrene_ipc_server::dispatch::dispatch(
                &daemon_api,
                MessageType::CmdSendChat,
                malformed,
            )
            .await
            .is_err()
        );
    }
    assert_eq!(daemon.send_chat_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn messaging_operations_preserve_scalars_and_add_typed_outcomes_over_socket() {
    let (mut server, sock) = setup_server().await;
    let mut stream = UnixStream::connect(&sock).await.expect("connect");
    let peer = "11".repeat(16);
    let peer_payload = HashMap::from([("peer_hash".into(), rmpv::Value::from(peer.as_str()))]);
    for (opcode, scalar) in [
        (MessageType::CmdMarkRead, "count"),
        (MessageType::CmdDeleteConversation, "count"),
        (MessageType::CmdPinConversation, "success"),
        (MessageType::CmdUnpinConversation, "success"),
        (MessageType::CmdMuteConversation, "success"),
        (MessageType::CmdUnmuteConversation, "success"),
    ] {
        let response = send_and_recv(&mut stream, opcode, &peer_payload).await;
        assert_eq!(response.msg_type, MessageType::Result);
        assert!(response.payload.contains_key(scalar));
        assert!(response.payload.get("outcome").and_then(rmpv::Value::as_map).is_some());
    }
    let message_payload = HashMap::from([("message_id".into(), rmpv::Value::from("message-id"))]);
    for (opcode, scalar) in [
        (MessageType::CmdDeleteMessage, "success"),
        (MessageType::CmdRetryMessage, "retried"),
        (MessageType::CmdCancelMessage, "cancelled"),
    ] {
        let response = send_and_recv(&mut stream, opcode, &message_payload).await;
        assert!(response.payload.contains_key(scalar));
        assert!(response.payload.get("outcome").and_then(rmpv::Value::as_map).is_some());
    }
    let contact = HashMap::from([
        ("peer_hash".into(), rmpv::Value::from(peer.as_str())),
        ("alias".into(), rmpv::Value::from("alias")),
        ("notes".into(), rmpv::Value::from("notes")),
    ]);
    let response = send_and_recv(&mut stream, MessageType::CmdSetContact, &contact).await;
    assert!(response.payload.contains_key("ok"));
    assert!(response.payload.contains_key("outcome"));
    let removed = send_and_recv(&mut stream, MessageType::CmdRemoveContact, &peer_payload).await;
    assert!(removed.payload.contains_key("removed"));
    assert!(removed.payload.contains_key("outcome"));
    let contacts = send_and_recv(&mut stream, MessageType::QueryContacts, &HashMap::new()).await;
    let first = contacts.payload["contacts"].as_array().unwrap()[0].as_map().unwrap();
    for field in ["peer_hash", "alias", "notes", "created_at", "updated_at"] {
        assert!(first.iter().any(|(key, _)| key.as_str() == Some(field)));
    }
    let search = HashMap::from([
        ("query".into(), rmpv::Value::from("literal")),
        ("limit".into(), rmpv::Value::from(1_u64)),
    ]);
    let response = send_and_recv(&mut stream, MessageType::QuerySearchMessages, &search).await;
    assert!(response.payload.contains_key("messages"));
    assert!(response.payload.contains_key("outcome"));
    server.stop().await;
}

#[tokio::test]
async fn authorization_denial_is_typed_and_non_retryable_over_socket() {
    let (mut server, sock) = setup_server().await;
    let mut stream = UnixStream::connect(&sock).await.expect("connect");
    let payload = HashMap::from([
        ("query".into(), rmpv::Value::from("denied")),
        ("limit".into(), rmpv::Value::from(1_u64)),
    ]);
    let response = send_and_recv(&mut stream, MessageType::QuerySearchMessages, &payload).await;
    assert_eq!(response.msg_type, MessageType::Error);
    assert_eq!(response.payload["kind"].as_str(), Some("denied"));
    assert_eq!(response.payload["code"].as_str(), Some("denied"));
    let error = IpcError::Denied { capability: "messaging.history.read".into() };
    let typed_error: IpcError = rmpv::ext::from_value(response.payload["typed_error"].clone())
        .expect("typed authorization error");
    assert_eq!(typed_error, error);
    assert!(!error.is_retryable());
    let invalid = HashMap::from([
        ("query".into(), rmpv::Value::from("")),
        ("limit".into(), rmpv::Value::from(1_u64)),
    ]);
    let response = send_and_recv(&mut stream, MessageType::QuerySearchMessages, &invalid).await;
    assert_eq!(response.msg_type, MessageType::Error);
    assert_eq!(response.payload["kind"].as_str(), Some("invalid_request"));
    server.stop().await;
}

#[tokio::test]
async fn query_status() {
    let (mut server, sock) = setup_server().await;
    let mut stream = UnixStream::connect(&sock).await.expect("connect");
    let resp = send_and_recv(&mut stream, MessageType::QueryStatus, &HashMap::new()).await;
    assert_eq!(resp.msg_type, MessageType::Result);
    assert_eq!(resp.payload.get("uptime").and_then(|v| v.as_u64()), Some(42));
    assert_eq!(resp.payload.get("daemon_version").and_then(|v| v.as_str()), Some("test-0.1.0"));
    assert!(resp.payload.get("connection_generation").and_then(|v| v.as_u64()).is_some());
    assert_eq!(
        resp.payload
            .get("standard_lxmf_propagation_destination_registered")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    assert_eq!(
        resp.payload.get("standard_lxmf_propagation_active").and_then(|value| value.as_bool()),
        Some(false)
    );
    let capabilities = resp.payload.get("active_capabilities").and_then(|v| v.as_map()).unwrap();
    let item = |key: &str| {
        capabilities.iter().find(|(k, _)| k.as_str() == Some(key)).map(|(_, value)| value)
    };
    assert_eq!(item("version").and_then(|v| v.as_u64()), Some(1));
    assert_eq!(item("runtime").and_then(|v| v.as_array()).map(Vec::len), Some(1));
    assert_eq!(item("degraded").and_then(|v| v.as_array()).map(Vec::len), Some(1));
    let capability_json: serde_json::Value = rmpv::ext::from_value(
        resp.payload.get("active_capabilities").expect("active capabilities").clone(),
    )
    .expect("JSON-compatible active capabilities");
    let typed_capabilities: ActiveCapabilitiesInfo =
        serde_json::from_value(capability_json).expect("typed active capabilities");
    assert_eq!(typed_capabilities.version, ACTIVE_CAPABILITIES_VERSION);
    assert_eq!(typed_capabilities.runtime, ["runtime.lxmf.direct"]);
    assert_eq!(typed_capabilities.authorized_operations, ["chat.send"]);
    assert_eq!(typed_capabilities.degraded.len(), 1);
    assert_eq!(typed_capabilities.degraded[0].id, "runtime.native-nomadnet.host");
    assert_eq!(typed_capabilities.degraded[0].reason, "request handler unavailable");
    server.stop().await;
}

#[tokio::test]
async fn standard_propagation_query_roundtrips_typed_metadata_without_payload_inventory() {
    fn assert_safe(value: &rmpv::Value) {
        match value {
            rmpv::Value::Map(entries) => {
                for (key, value) in entries {
                    if let Some(key) = key.as_str() {
                        assert!(!matches!(
                            key,
                            "lxmf_data"
                                | "stamp"
                                | "transient_id"
                                | "destination_hash"
                                | "failure_detail"
                                | "cursor"
                        ));
                    }
                    assert_safe(value);
                }
            }
            rmpv::Value::Array(values) => values.iter().for_each(assert_safe),
            _ => {}
        }
    }

    let (mut server, sock) = setup_server().await;
    let mut stream = UnixStream::connect(&sock).await.expect("connect");
    let response =
        send_and_recv(&mut stream, MessageType::QueryStandardPropagation, &HashMap::new()).await;
    assert_eq!(response.msg_type, MessageType::Result);
    let value = rmpv::Value::Map(
        response
            .payload
            .iter()
            .map(|(key, value)| (rmpv::Value::from(key.as_str()), value.clone()))
            .collect(),
    );
    assert_safe(&value);
    let encoded = rmp_serde::to_vec_named(&response.payload).expect("encode response payload");
    let snapshot: StandardPropagationSnapshot =
        rmp_serde::from_slice(&encoded).expect("typed standard propagation snapshot");
    let debug = format!("{snapshot:?}");
    for forbidden in ["lxmf_data", "transient_id", "failure_detail", "recipient_destination"] {
        assert!(!debug.contains(forbidden), "debug projection leaked {forbidden}");
    }
    assert_eq!(snapshot.version, STANDARD_PROPAGATION_SNAPSHOT_VERSION);
    assert!(snapshot.registered);
    assert!(snapshot.active);
    assert!(snapshot.connection_generation.is_some());
    assert_eq!(snapshot.policy.unwrap().transfer_limit_kb, 256);
    assert_eq!(snapshot.attempts.len(), 1);
    assert_eq!(snapshot.attempts[0].stage, StandardPropagationStage::Offer);
    assert_eq!(snapshot.attempts[0].outcome, StandardPropagationOutcome::Pending);
    server.stop().await;
}

#[tokio::test]
async fn status_generation_is_stable_per_connection_and_changes_on_reconnect() {
    let (mut server, sock) = setup_server().await;
    let mut first = UnixStream::connect(&sock).await.expect("connect first");
    let first_status = send_and_recv(&mut first, MessageType::QueryStatus, &HashMap::new()).await;
    let repeated = send_and_recv(&mut first, MessageType::QueryStatus, &HashMap::new()).await;
    let mut second = UnixStream::connect(&sock).await.expect("connect second");
    let second_status = send_and_recv(&mut second, MessageType::QueryStatus, &HashMap::new()).await;

    let generation = |frame: &wire::Frame| {
        frame.payload.get("connection_generation").and_then(|value| value.as_u64()).unwrap()
    };
    assert_eq!(generation(&first_status), generation(&repeated));
    assert_ne!(generation(&first_status), generation(&second_status));
    server.stop().await;
    server.start().await.expect("restart");
    let mut restarted = UnixStream::connect(&sock).await.expect("connect restarted");
    let restarted_status =
        send_and_recv(&mut restarted, MessageType::QueryStatus, &HashMap::new()).await;
    assert_ne!(generation(&first_status), generation(&restarted_status));
    server.stop().await;
}

#[tokio::test]
async fn query_identity() {
    let (mut server, sock) = setup_server().await;
    let mut stream = UnixStream::connect(&sock).await.expect("connect");
    let resp = send_and_recv(&mut stream, MessageType::QueryIdentity, &HashMap::new()).await;
    assert_eq!(resp.msg_type, MessageType::Result);
    assert_eq!(resp.payload.get("identity_hash").and_then(|v| v.as_str()), Some("deadbeef"));
    let custody = resp.payload.get("custody").and_then(rmpv::Value::as_map).expect("custody map");
    let keys = custody
        .iter()
        .map(|(key, _)| key.as_str().expect("string key"))
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        keys,
        [
            "active_backend",
            "authentication",
            "availability",
            "downgrade",
            "failure",
            "protection",
            "requested_backend",
        ]
        .into_iter()
        .collect()
    );
    let encoded = format!("{custody:?}");
    for forbidden in ["private", "passphrase", "credential", "key_material", "export"] {
        assert!(!encoded.contains(forbidden), "custody projection leaked {forbidden}");
    }
    server.stop().await;
}

#[tokio::test]
async fn mobile_diagnostics_snapshot_and_export_have_exact_bounded_wire_fields() {
    let (mut server, sock) = setup_server().await;
    let mut stream = UnixStream::connect(&sock).await.expect("connect");
    let snapshot =
        send_and_recv(&mut stream, MessageType::QueryMobileDiagnostics, &HashMap::new()).await;
    assert_eq!(snapshot.msg_type, MessageType::Result);
    assert_eq!(
        snapshot.payload.keys().map(String::as_str).collect::<std::collections::BTreeSet<_>>(),
        [
            "backend_revision",
            "dropped_events",
            "event_count",
            "events",
            "first_sequence",
            "last_sequence",
            "max_bytes",
            "max_events",
            "retained_bytes",
            "schema_version",
            "truncated",
        ]
        .into_iter()
        .collect()
    );
    let event = snapshot
        .payload
        .get("events")
        .and_then(rmpv::Value::as_array)
        .and_then(|events| events.first())
        .and_then(rmpv::Value::as_map)
        .expect("diagnostic event");
    assert_eq!(
        event
            .iter()
            .map(|(key, _)| key.as_str().expect("string key"))
            .collect::<std::collections::BTreeSet<_>>(),
        [
            "generation",
            "safe_correlation",
            "sequence",
            "severity",
            "source",
            "stage",
            "unix_time_ms",
        ]
        .into_iter()
        .collect()
    );

    let export =
        send_and_recv(&mut stream, MessageType::CmdExportMobileDiagnostics, &HashMap::new()).await;
    assert_eq!(export.msg_type, MessageType::Result);
    assert_eq!(
        export.payload.keys().map(String::as_str).collect::<std::collections::BTreeSet<_>>(),
        [
            "backend_revision",
            "byte_count",
            "bytes",
            "content_type",
            "digest_sha256",
            "dropped_events",
            "event_count",
            "first_sequence",
            "last_sequence",
            "max_bytes",
            "max_events",
            "schema_version",
            "truncated",
        ]
        .into_iter()
        .collect()
    );
    let bytes = export.payload.get("bytes").and_then(rmpv::Value::as_slice).expect("export bytes");
    assert!(bytes.len() <= MOBILE_DIAGNOSTIC_MAX_BYTES as usize);
    assert_eq!(
        export.payload.get("byte_count").and_then(rmpv::Value::as_u64),
        Some(bytes.len() as u64)
    );
    server.stop().await;
}

#[tokio::test]
async fn query_messages_serializes_authoritative_lifecycle() {
    let (mut server, sock) = setup_server().await;
    let mut stream = UnixStream::connect(&sock).await.expect("connect");
    let mut payload = HashMap::new();
    payload.insert("peer_hash".into(), rmpv::Value::from("22".repeat(16)));
    let response = send_and_recv(&mut stream, MessageType::QueryMessages, &payload).await;
    let message = response
        .payload
        .get("messages")
        .and_then(rmpv::Value::as_array)
        .and_then(|messages| messages.first())
        .and_then(rmpv::Value::as_map)
        .expect("message lifecycle");
    let field = |name: &str| {
        message.iter().find(|(key, _)| key.as_str() == Some(name)).map(|(_, value)| value)
    };

    assert_eq!(
        field("requested_delivery_method").and_then(rmpv::Value::as_str),
        Some("opportunistic")
    );
    assert_eq!(field("actual_delivery_method").and_then(rmpv::Value::as_str), Some("direct"));
    assert_eq!(field("fallback_reason").and_then(rmpv::Value::as_str), Some("packet limit"));
    assert_eq!(field("correlation_id").and_then(rmpv::Value::as_str), Some("send-1"));
    let attempt = field("attempts")
        .and_then(rmpv::Value::as_array)
        .and_then(|attempts| attempts.first())
        .and_then(rmpv::Value::as_map)
        .expect("typed attempt");
    let attempt_field = |name: &str| {
        attempt.iter().find(|(key, _)| key.as_str() == Some(name)).map(|(_, value)| value)
    };
    assert_eq!(attempt_field("message_id").and_then(rmpv::Value::as_str), Some("message-2"));
    assert_eq!(attempt_field("number").and_then(rmpv::Value::as_u64), Some(2));
    assert_eq!(attempt_field("started_unix_ms").and_then(rmpv::Value::as_i64), Some(100));
    assert_eq!(attempt_field("deadline_unix_ms").and_then(rmpv::Value::as_i64), Some(200));
    assert_eq!(attempt_field("state").and_then(rmpv::Value::as_str), Some("failed"));
    assert_eq!(attempt_field("bearer"), Some(&rmpv::Value::Nil));
    assert_eq!(
        attempt
            .iter()
            .map(|(key, _)| key.as_str().expect("string key"))
            .collect::<std::collections::BTreeSet<_>>(),
        [
            "bearer",
            "deadline_unix_ms",
            "message_id",
            "number",
            "route",
            "started_unix_ms",
            "state",
        ]
        .into_iter()
        .collect()
    );
    let route = attempt_field("route").and_then(rmpv::Value::as_map).expect("route observation");
    assert_eq!(
        route
            .iter()
            .map(|(key, _)| key.as_str().expect("string key"))
            .collect::<std::collections::BTreeSet<_>>(),
        [
            "connection_generation",
            "hops",
            "interface",
            "next_hop",
            "observed_at",
            "outcome",
            "stale",
        ]
        .into_iter()
        .collect()
    );
    assert_eq!(
        route
            .iter()
            .find(|(key, _)| key.as_str() == Some("outcome"))
            .and_then(|(_, value)| value.as_str()),
        Some("observed")
    );
    assert_canonical_wire_fields(message);
    assert_eq!(
        response.payload.get("next_cursor").and_then(rmpv::Value::as_str),
        Some("message-cursor")
    );
    server.stop().await;
}

#[tokio::test]
async fn query_message_returns_one_redacted_projection_or_authoritative_absence() {
    let (mut server, sock) = setup_server().await;
    let mut stream = UnixStream::connect(&sock).await.expect("connect");
    let payload = HashMap::from([("message_id".into(), rmpv::Value::from("message-2"))]);
    let response = send_and_recv(&mut stream, MessageType::QueryMessage, &payload).await;
    assert_eq!(response.msg_type, MessageType::Result);
    let message = response
        .payload
        .get("message")
        .and_then(rmpv::Value::as_map)
        .expect("authoritative message projection");
    assert_eq!(
        message
            .iter()
            .find(|(key, _)| key.as_str() == Some("id"))
            .and_then(|(_, value)| value.as_str()),
        Some("message-2")
    );
    assert_canonical_wire_fields(message);

    let missing = HashMap::from([("message_id".into(), rmpv::Value::from("missing"))]);
    let response = send_and_recv(&mut stream, MessageType::QueryMessage, &missing).await;
    assert_eq!(response.msg_type, MessageType::Result);
    assert_eq!(response.payload.get("message"), Some(&rmpv::Value::Nil));
    server.stop().await;
}

#[tokio::test]
async fn old_conversation_query_stays_full_list_and_paged_query_adds_cursor() {
    let (mut server, sock) = setup_server().await;
    let mut stream = UnixStream::connect(&sock).await.expect("connect");
    let old = send_and_recv(&mut stream, MessageType::QueryConversations, &HashMap::new()).await;
    assert!(old.payload.contains_key("conversations"));
    assert!(!old.payload.contains_key("next_cursor"));

    let query = HashMap::from([("limit".into(), rmpv::Value::from(10_u64))]);
    let page = send_and_recv(&mut stream, MessageType::QueryConversations, &query).await;
    assert_eq!(
        page.payload.get("next_cursor").and_then(rmpv::Value::as_str),
        Some("conversation-cursor")
    );
    server.stop().await;
}

#[tokio::test]
async fn legacy_include_unread_alias_preserves_unbounded_behavior_and_rejects_conflicts() {
    let (mut server, sock) = setup_server().await;
    let mut stream = UnixStream::connect(&sock).await.expect("connect");
    for (value, expected) in [(true, "unread"), (false, "all")] {
        let query = HashMap::from([("include_unread".into(), rmpv::Value::Boolean(value))]);
        let response = send_and_recv(&mut stream, MessageType::QueryConversations, &query).await;
        assert_eq!(response.msg_type, MessageType::Result);
        assert!(!response.payload.contains_key("next_cursor"));
        let peer = response.payload["conversations"].as_array().unwrap()[0]
            .as_map()
            .unwrap()
            .iter()
            .find(|(key, _)| key.as_str() == Some("peer_hash"))
            .and_then(|(_, value)| value.as_str());
        assert_eq!(peer, Some(expected));
    }

    let conflicting = HashMap::from([
        ("unread_only".into(), rmpv::Value::Boolean(true)),
        ("include_unread".into(), rmpv::Value::Boolean(false)),
    ]);
    let response = send_and_recv(&mut stream, MessageType::QueryConversations, &conflicting).await;
    assert_eq!(response.msg_type, MessageType::Error);
    server.stop().await;
}

#[tokio::test]
async fn malformed_cursor_zero_limit_and_mixed_legacy_boundary_fail_before_daemon() {
    let (mut server, sock) = setup_server().await;
    let mut stream = UnixStream::connect(&sock).await.expect("connect");
    for query in [
        HashMap::from([
            ("peer_hash".into(), rmpv::Value::from("22".repeat(16))),
            ("cursor".into(), rmpv::Value::from("x".repeat(129))),
        ]),
        HashMap::from([
            ("peer_hash".into(), rmpv::Value::from("22".repeat(16))),
            ("limit".into(), rmpv::Value::from(0_u64)),
        ]),
        HashMap::from([
            ("peer_hash".into(), rmpv::Value::from("22".repeat(16))),
            ("cursor".into(), rmpv::Value::from("cursor")),
            ("before_ts".into(), rmpv::Value::from(10_i64)),
        ]),
    ] {
        let response = send_and_recv(&mut stream, MessageType::QueryMessages, &query).await;
        assert_eq!(response.msg_type, MessageType::Error);
    }
    server.stop().await;
}

#[tokio::test]
async fn nil_legacy_boundary_is_none_and_cursor_with_nil_uses_cursor_page() {
    let (mut server, sock) = setup_server().await;
    let mut stream = UnixStream::connect(&sock).await.expect("connect");
    let base = [("peer_hash".into(), rmpv::Value::from("22".repeat(16)))];

    let mut legacy_nil = HashMap::from(base.clone());
    legacy_nil.insert("before_ts".into(), rmpv::Value::Nil);
    let response = send_and_recv(&mut stream, MessageType::QueryMessages, &legacy_nil).await;
    assert_eq!(response.msg_type, MessageType::Result);
    assert_eq!(response.payload["next_cursor"].as_str(), Some("message-cursor"));

    let mut cursor_and_nil = HashMap::from(base);
    cursor_and_nil.insert("cursor".into(), rmpv::Value::from("message-cursor"));
    cursor_and_nil.insert("before_ts".into(), rmpv::Value::Nil);
    let response = send_and_recv(&mut stream, MessageType::QueryMessages, &cursor_and_nil).await;
    assert_eq!(response.msg_type, MessageType::Result);
    assert!(response.payload["next_cursor"].is_nil());
    server.stop().await;
}

#[tokio::test]
async fn typed_daemon_errors_add_metadata_without_removing_legacy_error_string() {
    let (mut server, sock) = setup_server().await;
    let mut stream = UnixStream::connect(&sock).await.expect("connect");
    let query = HashMap::from([
        ("peer_hash".into(), rmpv::Value::from("22".repeat(16))),
        ("cursor".into(), rmpv::Value::from("stale")),
    ]);
    let response = send_and_recv(&mut stream, MessageType::QueryMessages, &query).await;
    assert_eq!(response.msg_type, MessageType::Error);
    assert_eq!(response.payload["error"].as_str(), Some("conflict: cursor_stale"));
    assert_eq!(response.payload["message"].as_str(), Some("conflict: cursor_stale"));
    assert_eq!(response.payload["kind"].as_str(), Some("conflict"));
    assert_eq!(response.payload["code"].as_str(), Some("cursor_stale"));
    let typed_error: IpcError = rmpv::ext::from_value(response.payload["typed_error"].clone())
        .expect("typed conflict error");
    assert_eq!(typed_error, IpcError::Conflict { message: "cursor_stale".into() });
    server.stop().await;
}

#[tokio::test]
async fn search_messages_serializes_canonical_wire_fields_as_binary() {
    let (mut server, sock) = setup_server().await;
    let mut stream = UnixStream::connect(&sock).await.expect("connect");
    let mut payload = HashMap::new();
    payload.insert("query".into(), rmpv::Value::from("message"));
    let response = send_and_recv(&mut stream, MessageType::QuerySearchMessages, &payload).await;
    let message = response
        .payload
        .get("messages")
        .and_then(rmpv::Value::as_array)
        .and_then(|messages| messages.first())
        .and_then(rmpv::Value::as_map)
        .expect("search message");
    assert_canonical_wire_fields(message);
    server.stop().await;
}

#[tokio::test]
async fn oversized_query_and_search_pages_return_explicit_wire_errors() {
    let (mut server, sock) = setup_server().await;
    let mut stream = UnixStream::connect(&sock).await.expect("connect");
    let query = HashMap::from([(
        "peer_hash".into(),
        rmpv::Value::from("ffffffffffffffffffffffffffffffff"),
    )]);
    let response = send_and_recv(&mut stream, MessageType::QueryMessages, &query).await;
    assert_eq!(response.msg_type, MessageType::Result);

    let search = HashMap::from([("query".into(), rmpv::Value::from("oversized-page"))]);
    let response = send_and_recv(&mut stream, MessageType::QuerySearchMessages, &search).await;
    assert_eq!(response.msg_type, MessageType::Error);
    server.stop().await;
}

#[tokio::test]
async fn huge_message_limits_are_rejected_before_daemon_query() {
    let (mut server, sock) = setup_server().await;
    let mut stream = UnixStream::connect(&sock).await.expect("connect");
    let query = HashMap::from([
        ("peer_hash".into(), rmpv::Value::from("22".repeat(16))),
        ("limit".into(), rmpv::Value::from(u64::MAX)),
    ]);
    let response = send_and_recv(&mut stream, MessageType::QueryMessages, &query).await;
    assert_eq!(response.msg_type, MessageType::Error);
    assert!(response.payload["error"].as_str().is_some_and(|error| error.contains("limit")));

    let search = HashMap::from([
        ("query".into(), rmpv::Value::from("message")),
        ("limit".into(), rmpv::Value::from(u64::MAX)),
    ]);
    let response = send_and_recv(&mut stream, MessageType::QuerySearchMessages, &search).await;
    assert_eq!(response.msg_type, MessageType::Error);
    assert!(response.payload["error"].as_str().is_some_and(|error| error.contains("limit")));
    server.stop().await;
}

#[tokio::test]
async fn query_devices() {
    let (mut server, sock) = setup_server().await;
    let mut stream = UnixStream::connect(&sock).await.expect("connect");
    let resp = send_and_recv(&mut stream, MessageType::QueryDevices, &HashMap::new()).await;
    assert_eq!(resp.msg_type, MessageType::Result);
    let devices = resp.payload.get("devices").and_then(|v| v.as_array());
    assert!(devices.is_some());
    let devices = devices.expect("arr");
    assert_eq!(devices.len(), 1);
    let state = devices[0]
        .as_map()
        .and_then(|map| {
            map.iter().find(|(key, _)| key.as_str() == Some("standard_lxmf_propagation_active"))
        })
        .and_then(|(_, value)| value.as_bool());
    assert_eq!(state, Some(false));
    server.stop().await;
}

#[tokio::test]
async fn query_links_separates_active_and_history_with_connection_generation() {
    let (mut server, sock) = setup_server().await;
    let mut stream = UnixStream::connect(&sock).await.expect("connect");

    let response = send_and_recv(&mut stream, MessageType::QueryLinks, &HashMap::new()).await;
    let active = response.payload["active"].as_array().expect("active links");
    let history = response.payload["history"].as_array().expect("link history");
    fn value<'a>(entry: &'a rmpv::Value, key: &str) -> Option<&'a rmpv::Value> {
        entry
            .as_map()
            .and_then(|map| map.iter().find(|(name, _)| name.as_str() == Some(key)))
            .map(|(_, value)| value)
    }

    assert_eq!(active.len(), 1);
    assert_eq!(history.len(), 1);
    assert_eq!(value(&active[0], "activity").and_then(rmpv::Value::as_str), Some("active"));
    assert_eq!(value(&history[0], "activity").and_then(rmpv::Value::as_str), Some("historical"));
    assert!(value(&active[0], "connection_generation").and_then(rmpv::Value::as_u64).is_some());
    server.stop().await;
}

#[tokio::test]
async fn resource_snapshot_and_cancel_use_typed_wire_contract() {
    let (mut server, sock) = setup_server().await;
    let mut stream = UnixStream::connect(&sock).await.expect("connect");

    let response = send_and_recv(&mut stream, MessageType::QueryResources, &HashMap::new()).await;
    let resource = response.payload["resources"]
        .as_array()
        .and_then(|resources| resources.first())
        .and_then(rmpv::Value::as_map)
        .expect("resource transfer");
    let value = |key: &str| {
        resource.iter().find(|(name, _)| name.as_str() == Some(key)).map(|(_, value)| value)
    };
    assert_eq!(value("state").and_then(rmpv::Value::as_str), Some("transferring"));
    assert_eq!(value("received_bytes").and_then(rmpv::Value::as_u64), Some(512));
    assert!(value("connection_generation").and_then(rmpv::Value::as_u64).is_some());

    let request = HashMap::from([("resource_hash".into(), rmpv::Value::from("ee".repeat(32)))]);
    let response = send_and_recv(&mut stream, MessageType::CmdResourceCancel, &request).await;
    assert_eq!(response.payload["accepted"].as_bool(), Some(true));
    server.stop().await;
}

#[tokio::test]
async fn interface_counters_remain_unsigned_on_the_wire() {
    let (mut server, sock) = setup_server().await;
    let mut stream = UnixStream::connect(&sock).await.expect("connect");
    let response =
        send_and_recv(&mut stream, MessageType::QueryInterfaceStats, &HashMap::new()).await;
    let interface = response
        .payload
        .get("interfaces")
        .and_then(rmpv::Value::as_array)
        .and_then(|interfaces| interfaces.first())
        .and_then(rmpv::Value::as_map)
        .unwrap();
    let item = |key: &str| {
        interface.iter().find(|(name, _)| name.as_str() == Some(key)).map(|(_, value)| value)
    };
    assert_eq!(item("tx_bytes").and_then(rmpv::Value::as_u64), Some(u64::MAX));
    assert_eq!(item("rx_bytes").and_then(rmpv::Value::as_u64), Some(i64::MAX as u64 + 1));
    assert_eq!(item("source").and_then(rmpv::Value::as_str), Some("runtime_interface_registry"));
    assert_eq!(item("age_secs").and_then(rmpv::Value::as_u64), Some(10));
    assert_eq!(item("connection_generation").and_then(rmpv::Value::as_u64), Some(73));
    assert!(item("ipc_connection_generation").and_then(rmpv::Value::as_u64).is_some());
    assert_eq!(item("interface_generation").and_then(rmpv::Value::as_u64), Some(5));
    assert_eq!(item("type").and_then(rmpv::Value::as_str), Some("tcp_server"));
    assert!(item("kind").is_none());
    assert_eq!(item("mode").and_then(rmpv::Value::as_str), Some("full"));
    assert_eq!(item("enabled").and_then(rmpv::Value::as_bool), Some(true));
    assert_eq!(item("host").and_then(rmpv::Value::as_str), Some("127.0.0.1"));
    assert_eq!(item("port").and_then(rmpv::Value::as_u64), Some(4242));
    assert_eq!(item("local_endpoint").and_then(rmpv::Value::as_str), Some("127.0.0.1:4242"));
    assert_eq!(item("remote_endpoint").and_then(rmpv::Value::as_str), Some("192.0.2.1:5252"));
    assert_eq!(
        item("parent_hash").and_then(rmpv::Value::as_str),
        Some("22222222222222222222222222222222")
    );
    assert_eq!(item("connected_peers").and_then(rmpv::Value::as_u64), Some(3));
    assert!(item("peers_connected").is_none());
    server.stop().await;
}

#[tokio::test]
async fn path_observations_preserve_daemon_generation_and_separate_socket_generation() {
    let (mut server, sock) = setup_server().await;
    let mut stream = UnixStream::connect(&sock).await.expect("connect");
    let mut request = HashMap::new();
    request
        .insert("destination_hash".into(), rmpv::Value::from("11111111111111111111111111111111"));
    let single = send_and_recv(&mut stream, MessageType::QueryPathInfo, &request).await;
    let table = send_and_recv(&mut stream, MessageType::QueryPathTable, &HashMap::new()).await;
    let table_path = table
        .payload
        .get("paths")
        .and_then(rmpv::Value::as_array)
        .and_then(|paths| paths.first())
        .and_then(rmpv::Value::as_map)
        .unwrap();
    let table_ipc_generation = table_path
        .iter()
        .find(|(key, _)| key.as_str() == Some("ipc_connection_generation"))
        .and_then(|(_, value)| value.as_u64());

    assert_eq!(
        single.payload.get("source").and_then(rmpv::Value::as_str),
        Some("transport_path_table")
    );
    assert_eq!(single.payload.get("age_secs").and_then(rmpv::Value::as_u64), Some(10));
    assert_eq!(single.payload.get("expires").and_then(rmpv::Value::as_i64), Some(700));
    assert_eq!(single.payload.get("connection_generation").and_then(rmpv::Value::as_u64), Some(73));
    assert_eq!(
        table_path
            .iter()
            .find(|(key, _)| key.as_str() == Some("connection_generation"))
            .and_then(|(_, value)| value.as_u64()),
        Some(73)
    );
    assert_eq!(
        single.payload.get("ipc_connection_generation").and_then(rmpv::Value::as_u64),
        table_ipc_generation
    );
    assert!(table_ipc_generation.is_some());
    assert!(!single.payload.contains_key("correlation_id"));
    server.stop().await;
}

#[tokio::test]
async fn unknown_message_returns_error() {
    let (mut server, sock) = setup_server().await;
    let mut stream = UnixStream::connect(&sock).await.expect("connect");
    let resp = send_and_recv(&mut stream, MessageType::CmdRemoteMessages, &HashMap::new()).await;
    assert_eq!(resp.msg_type, MessageType::Error);
    assert!(resp.payload.get("error").and_then(|v| v.as_str()).is_some());
    server.stop().await;
}

#[tokio::test]
async fn multiple_concurrent_clients() {
    let (mut server, sock) = setup_server().await;
    let s1 = sock.clone();
    let s2 = sock.clone();

    let h1 = tokio::spawn(async move {
        let mut stream = UnixStream::connect(&s1).await.expect("c1");
        let resp = send_and_recv(&mut stream, MessageType::Ping, &HashMap::new()).await;
        assert_eq!(resp.msg_type, MessageType::Pong);
    });
    let h2 = tokio::spawn(async move {
        let mut stream = UnixStream::connect(&s2).await.expect("c2");
        let resp = send_and_recv(&mut stream, MessageType::Ping, &HashMap::new()).await;
        assert_eq!(resp.msg_type, MessageType::Pong);
    });

    h1.await.expect("c1");
    h2.await.expect("c2");
    server.stop().await;
}

#[tokio::test]
async fn stop_removes_socket() {
    let (mut server, sock) = setup_server().await;
    assert!(sock.exists());
    server.stop().await;
    assert!(!sock.exists());
}

#[tokio::test]
async fn subscribe_and_event_push() {
    let (mut server, sock) = setup_server().await;
    let event_tx = server.event_sender();

    let mut stream = UnixStream::connect(&sock).await.expect("connect");

    // Subscribe to devices
    let resp = send_and_recv(&mut stream, MessageType::SubDevices, &HashMap::new()).await;
    assert_eq!(resp.msg_type, MessageType::Result);

    // Push an event
    let mut device = DeviceInfo::default();
    device.destination_hash = "event-device".into();
    device.name = "pushed-node".into();
    device.standard_lxmf_propagation_active = Some(true);
    let _ = event_tx.send(DaemonEvent::Device { device });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Read the pushed event frame
    use tokio::io::AsyncReadExt;
    let mut len_buf = [0u8; 4];
    tokio::time::timeout(tokio::time::Duration::from_secs(2), stream.read_exact(&mut len_buf))
        .await
        .expect("timeout")
        .expect("read len");

    let total = u32::from_be_bytes(len_buf) as usize;
    let mut frame_buf = vec![0u8; total];
    stream.read_exact(&mut frame_buf).await.expect("read");

    let mut full = Vec::with_capacity(4 + total);
    full.extend_from_slice(&len_buf);
    full.extend_from_slice(&frame_buf);
    let event_frame = wire::decode_frame(&full).expect("decode");

    assert_eq!(event_frame.msg_type, MessageType::EventDevice);
    assert_eq!(
        event_frame.payload.get("destination_hash").and_then(|v| v.as_str()),
        Some("event-device")
    );
    assert_eq!(
        event_frame
            .payload
            .get("standard_lxmf_propagation_active")
            .and_then(|value| value.as_bool()),
        Some(true)
    );

    server.stop().await;
}

#[tokio::test]
async fn message_event_serializes_canonical_wire_fields_as_binary() {
    use tokio::io::AsyncReadExt;

    let (mut server, sock) = setup_server().await;
    let event_tx = server.event_sender();
    let mut stream = UnixStream::connect(&sock).await.expect("connect");
    let response = send_and_recv(&mut stream, MessageType::SubMessages, &HashMap::new()).await;
    assert_eq!(response.msg_type, MessageType::Result);
    let _ = event_tx.send(DaemonEvent::Message {
        kind: MessageEventKind::New,
        message: canonical_test_message(),
    });

    let mut len_buf = [0u8; 4];
    tokio::time::timeout(tokio::time::Duration::from_secs(2), stream.read_exact(&mut len_buf))
        .await
        .expect("event timeout")
        .expect("read event length");
    let total = u32::from_be_bytes(len_buf) as usize;
    let mut frame_buf = vec![0u8; total];
    stream.read_exact(&mut frame_buf).await.expect("read event");
    let mut full = Vec::with_capacity(4 + total);
    full.extend_from_slice(&len_buf);
    full.extend_from_slice(&frame_buf);
    let frame = wire::decode_frame(&full).expect("decode event");
    assert_eq!(frame.msg_type, MessageType::EventMessage);
    let fields: Vec<_> = frame
        .payload
        .iter()
        .map(|(key, value)| (rmpv::Value::from(key.as_str()), value.clone()))
        .collect();
    assert_canonical_wire_fields(&fields);
    server.stop().await;
}

#[tokio::test]
async fn maximum_message_event_fits_and_oversized_event_requests_reconciliation() {
    use tokio::io::AsyncReadExt;

    let (mut server, sock) = setup_server().await;
    let event_tx = server.event_sender();
    let mut stream = UnixStream::connect(&sock).await.expect("connect");
    let response = send_and_recv(&mut stream, MessageType::SubMessages, &HashMap::new()).await;
    assert_eq!(response.msg_type, MessageType::Result);

    let _ = event_tx.send(DaemonEvent::Message {
        kind: MessageEventKind::New,
        message: maximum_accepted_message(),
    });
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await.expect("read maximum event length");
    let mut bytes = vec![0; u32::from_be_bytes(len_buf) as usize];
    stream.read_exact(&mut bytes).await.expect("read maximum event");
    let mut frame = len_buf.to_vec();
    frame.extend_from_slice(&bytes);
    assert_eq!(wire::decode_frame(&frame).unwrap().msg_type, MessageType::EventMessage);

    let mut oversized = canonical_test_message();
    oversized.canonical_wire = Some(vec![0; wire::MAX_PAYLOAD_SIZE + 1]);
    let _ = event_tx.send(DaemonEvent::Message { kind: MessageEventKind::New, message: oversized });
    stream.read_exact(&mut len_buf).await.expect("read reconciliation length");
    let mut bytes = vec![0; u32::from_be_bytes(len_buf) as usize];
    stream.read_exact(&mut bytes).await.expect("read reconciliation");
    let mut frame = len_buf.to_vec();
    frame.extend_from_slice(&bytes);
    let frame = wire::decode_frame(&frame).unwrap();
    assert_eq!(frame.msg_type, MessageType::EventMessage);
    assert!(!frame.payload.contains_key("canonical_wire"));
    server.stop().await;
}

#[tokio::test]
async fn route_loss_event_carries_snapshot_and_physical_generation() {
    use tokio::io::AsyncReadExt;

    let (mut server, sock) = setup_server().await;
    let event_tx = server.event_sender();
    let mut stream = UnixStream::connect(&sock).await.expect("connect");
    let response = send_and_recv(&mut stream, MessageType::SubRoutes, &HashMap::new()).await;
    assert_eq!(response.msg_type, MessageType::Result);

    let mut event = RouteEventInfo::default();
    event.kind = RouteEventKind::Lost;
    event.route = test_path();
    event.loss_reason = Some(RouteLossReason::Expired);
    event.observation = test_observation(ObservationSource::TransportPathTable);
    let _ = event_tx.send(DaemonEvent::Route { event });

    let mut len_buf = [0u8; 4];
    tokio::time::timeout(tokio::time::Duration::from_secs(2), stream.read_exact(&mut len_buf))
        .await
        .expect("timeout")
        .expect("read len");
    let total = u32::from_be_bytes(len_buf) as usize;
    let mut frame_buf = vec![0u8; total];
    stream.read_exact(&mut frame_buf).await.expect("read event");
    let mut full = Vec::with_capacity(4 + total);
    full.extend_from_slice(&len_buf);
    full.extend_from_slice(&frame_buf);
    let frame = wire::decode_frame(&full).expect("decode event");

    assert_eq!(frame.msg_type, MessageType::EventRoute);
    assert_eq!(frame.request_id, [0; REQUEST_ID_SIZE]);
    assert_eq!(frame.payload.get("kind").and_then(rmpv::Value::as_str), Some("lost"));
    assert_eq!(frame.payload.get("loss_reason").and_then(rmpv::Value::as_str), Some("expired"));
    assert_eq!(frame.payload.get("expires").and_then(rmpv::Value::as_i64), Some(700));
    assert!(frame.payload.get("connection_generation").and_then(rmpv::Value::as_u64).is_some());
    assert_eq!(
        frame.payload.get("route_connection_generation").and_then(rmpv::Value::as_u64),
        frame.payload.get("connection_generation").and_then(rmpv::Value::as_u64)
    );
    server.stop().await;
}
