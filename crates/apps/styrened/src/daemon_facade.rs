//! DaemonFacade — thin Daemon trait implementation with RBAC enforcement.
//!
//! The IPC-facing dispatch layer. Holds `Arc<AppContext>` and delegates
//! to services after checking capabilities via `PolicyService::has_capability()`.
//!
//! **Call direction**: IPC → DaemonFacade → PolicyService.has_capability() → Service → storage/transport.
//! Services never call DaemonFacade. Services access each other through AppContext accessors.
//!
//! Package I — see ownership-matrix.md §DaemonFacade.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use sha2::Digest as _;
use tokio::sync::broadcast;

use styrene_ipc::error::IpcError;
use styrene_ipc::traits::*;
use styrene_ipc::types::*;

use crate::app_context::AppContext;
use crate::services::AutoReplyMode;
use crate::services::messaging::attachment_record_to_info;
use crate::storage::messages::MessageRecord;
use styrene_rbac::Capability;

const INTERFACE_FRESHNESS_THRESHOLD_SECS: u64 = 30;
const PATH_FRESHNESS_THRESHOLD_SECS: u64 = 300;

pub(crate) struct SessionGeneration {
    current: AtomicU64,
    state: Mutex<SessionGenerationState>,
}

#[derive(Default)]
struct SessionGenerationState {
    initialized: bool,
    interfaces: HashMap<String, u64>,
}

impl SessionGeneration {
    pub(crate) fn new(initial: u64) -> Self {
        Self {
            current: AtomicU64::new(initial.max(1)),
            state: Mutex::new(SessionGenerationState::default()),
        }
    }

    pub(crate) fn current(&self) -> u64 {
        self.current.load(Ordering::Acquire)
    }

    pub(crate) fn observe(
        &self,
        snapshots: &[rns_core::transport::iface::InterfaceSnapshot],
    ) -> u64 {
        self.observe_generations(
            snapshots
                .iter()
                .map(|snapshot| (hex::encode(snapshot.hash.as_slice()), snapshot.generation)),
        )
    }

    fn observe_generations(&self, observed: impl IntoIterator<Item = (String, u64)>) -> u64 {
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let observed = observed.into_iter().collect::<HashMap<_, _>>();
        let mut advance = 0;
        let topology_changed = state.interfaces.len() != observed.len()
            || state.interfaces.keys().any(|hash| !observed.contains_key(hash));
        if state.initialized && topology_changed {
            advance += 1;
        }
        for (hash, generation) in &observed {
            advance += match state.interfaces.get(hash) {
                Some(previous) => (*generation).max(1).saturating_sub((*previous).max(1)),
                None => generation.saturating_sub(1),
            };
        }
        state.interfaces = observed;
        state.initialized = true;
        drop(state);
        if advance > 0 {
            self.current.fetch_add(advance, Ordering::AcqRel).saturating_add(advance)
        } else {
            self.current()
        }
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
        .unwrap_or(0)
}

fn capability_failure_code(
    kind: crate::startup_contract::CapabilityFailureKind,
) -> CapabilityFailureCode {
    match kind {
        crate::startup_contract::CapabilityFailureKind::Unavailable => {
            CapabilityFailureCode::Unavailable
        }
        crate::startup_contract::CapabilityFailureKind::Unauthorized => {
            CapabilityFailureCode::Unauthorized
        }
        crate::startup_contract::CapabilityFailureKind::Degraded => CapabilityFailureCode::Degraded,
        crate::startup_contract::CapabilityFailureKind::Unverified => {
            CapabilityFailureCode::Unverified
        }
    }
}

fn standard_propagation_direction(value: &str) -> StandardPropagationDirection {
    match value {
        "ingress" => StandardPropagationDirection::Ingress,
        "egress" => StandardPropagationDirection::Egress,
        "sync" => StandardPropagationDirection::Sync,
        _ => StandardPropagationDirection::Unknown,
    }
}

fn standard_propagation_stage(value: &str) -> StandardPropagationStage {
    match value {
        "offer" => StandardPropagationStage::Offer,
        "transfer" => StandardPropagationStage::Transfer,
        "get" => StandardPropagationStage::Get,
        "fetch" => StandardPropagationStage::Fetch,
        "download" => StandardPropagationStage::Download,
        "sync" => StandardPropagationStage::Sync,
        "complete" => StandardPropagationStage::Complete,
        _ => StandardPropagationStage::Unknown,
    }
}

fn standard_propagation_state(value: &str) -> StandardPropagationAttemptState {
    match value {
        "running" => StandardPropagationAttemptState::Running,
        "completed" => StandardPropagationAttemptState::Completed,
        "failed" => StandardPropagationAttemptState::Failed,
        "interrupted" => StandardPropagationAttemptState::Interrupted,
        _ => StandardPropagationAttemptState::Unknown,
    }
}

fn standard_propagation_outcome(
    state: StandardPropagationAttemptState,
    failure_code: Option<&str>,
) -> StandardPropagationOutcome {
    match (state, failure_code) {
        (StandardPropagationAttemptState::Running, _) => StandardPropagationOutcome::Pending,
        (StandardPropagationAttemptState::Completed, _) => StandardPropagationOutcome::Completed,
        (StandardPropagationAttemptState::Interrupted, _) => {
            StandardPropagationOutcome::Interrupted
        }
        (StandardPropagationAttemptState::Failed, Some("capacity")) => {
            StandardPropagationOutcome::CapacityRejected
        }
        (StandardPropagationAttemptState::Failed, _) => StandardPropagationOutcome::Failed,
        _ => StandardPropagationOutcome::Unknown,
    }
}

fn standard_propagation_trigger_source(
    trigger: crate::workers::standard_propagation::StandardPropagationSyncTriggerKind,
) -> StandardPropagationTriggerSource {
    use crate::workers::standard_propagation::StandardPropagationSyncTriggerKind;

    match trigger {
        StandardPropagationSyncTriggerKind::InitialConnection => {
            StandardPropagationTriggerSource::InitialConnection
        }
        StandardPropagationSyncTriggerKind::Reconnect => {
            StandardPropagationTriggerSource::Reconnect
        }
        StandardPropagationSyncTriggerKind::ForegroundOpportunity => {
            StandardPropagationTriggerSource::ForegroundOpportunity
        }
        StandardPropagationSyncTriggerKind::BackgroundOpportunity => {
            StandardPropagationTriggerSource::GrantedBackgroundOpportunity
        }
        StandardPropagationSyncTriggerKind::Manual => StandardPropagationTriggerSource::Manual,
    }
}

fn standard_propagation_terminal_outcome(
    outcome: crate::workers::standard_propagation::StandardPropagationSyncTerminalOutcome,
) -> StandardPropagationSyncTerminalOutcome {
    use crate::workers::standard_propagation::StandardPropagationSyncTerminalOutcome as WorkerOutcome;

    match outcome {
        WorkerOutcome::Succeeded => StandardPropagationSyncTerminalOutcome::Succeeded,
        WorkerOutcome::Failed => StandardPropagationSyncTerminalOutcome::Failed,
        WorkerOutcome::TimedOut => StandardPropagationSyncTerminalOutcome::TimedOut,
        WorkerOutcome::Cancelled => StandardPropagationSyncTerminalOutcome::Cancelled,
    }
}

fn standard_propagation_selection_readiness(
    active: bool,
    selection: Option<&crate::storage::standard_propagation::StandardPropagationSelection>,
    peers: &[crate::storage::standard_propagation::StandardPropagationPeerObservation],
) -> StandardPropagationSelectionReadiness {
    let Some(selected_peer) = selection.and_then(|selection| selection.peer) else {
        return StandardPropagationSelectionReadiness::NoSelection;
    };
    let Some(peer) = peers.iter().find(|peer| peer.identity_hash == selected_peer) else {
        return StandardPropagationSelectionReadiness::Unavailable;
    };
    if active && peer.enabled && peer.propagation_destination.is_some() {
        StandardPropagationSelectionReadiness::Ready
    } else {
        StandardPropagationSelectionReadiness::Unavailable
    }
}

fn path_observation(
    snapshot: rns_core::transport::core_transport::path_table::PathSnapshot,
) -> ObservationMetadata {
    let observed_at = snapshot
        .observed_at
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64);
    let age_secs = snapshot.age.as_secs();
    let mut observation = ObservationMetadata::default();
    observation.source = ObservationSource::TransportPathTable;
    observation.observed_at = observed_at;
    observation.age_secs = Some(age_secs);
    observation.freshness_threshold_secs = Some(PATH_FRESHNESS_THRESHOLD_SECS);
    observation.stale = age_secs > PATH_FRESHNESS_THRESHOLD_SECS;
    observation
}

fn path_expiry(
    snapshot: rns_core::transport::core_transport::path_table::PathSnapshot,
) -> Option<i64> {
    snapshot
        .expires_at
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
}

/// Convert an io::Error to IpcError::Internal.
fn internal(e: std::io::Error) -> IpcError {
    IpcError::Internal { message: e.to_string() }
}

fn messaging_error(error: std::io::Error) -> IpcError {
    if error.kind() == std::io::ErrorKind::InvalidInput {
        IpcError::invalid_request(error.to_string())
    } else if error.kind() == std::io::ErrorKind::Unsupported {
        IpcError::Unavailable { reason: error.to_string() }
    } else {
        authoritative_send_error(error.to_string())
    }
}

fn authoritative_send_error(reason: String) -> IpcError {
    if reason.contains("no compatible propagation node")
        || reason.contains("propagation coordinator is unavailable")
        || reason.contains("no selected compatible standard LXMF propagation peer")
    {
        IpcError::Unavailable { reason }
    } else {
        IpcError::Internal { message: reason }
    }
}

fn page_error(error: crate::storage::messages::PageError) -> IpcError {
    match error {
        crate::storage::messages::PageError::InvalidCursor(message) => {
            IpcError::invalid_request(message)
        }
        crate::storage::messages::PageError::CursorStale => {
            IpcError::Conflict { message: "cursor_stale".into() }
        }
        crate::storage::messages::PageError::Internal(message) => IpcError::Internal { message },
        crate::storage::messages::PageError::Storage(error) => {
            IpcError::Internal { message: error.to_string() }
        }
    }
}

fn summary_to_conversation_info(
    summary: crate::storage::messages::ConversationSummary,
) -> ConversationInfo {
    let mut info = ConversationInfo::default();
    info.peer_hash = summary.peer_hash;
    info.peer_name = summary.peer_name;
    info.last_message_timestamp = summary.last_message_timestamp;
    info.last_message_content = summary.last_message_content;
    info.unread_count = summary.unread_count;
    info.message_count = summary.message_count;
    info.pinned = summary.pinned;
    info.muted = summary.muted;
    info
}

fn contact_to_info(contact: crate::storage::messages::ContactRecord) -> ContactInfo {
    let mut info = ContactInfo::default();
    info.peer_hash = contact.peer_hash;
    info.alias = contact.alias;
    info.notes = contact.notes;
    info.created_at = Some(contact.created_at);
    info.updated_at = Some(contact.updated_at);
    info
}

fn mutation_disposition(
    disposition: crate::storage::messages::MutationDisposition,
) -> MessagingDisposition {
    use crate::storage::messages::MutationDisposition as Storage;
    match disposition {
        Storage::Applied => MessagingDisposition::Applied,
        Storage::Unchanged => MessagingDisposition::Unchanged,
        Storage::NotFound => MessagingDisposition::NotFound,
        Storage::TerminalConflict => MessagingDisposition::TerminalConflict,
        Storage::Created => MessagingDisposition::Created,
        Storage::Updated => MessagingDisposition::Updated,
    }
}

/// Convert a MessageRecord to a MessageInfo IPC type.
fn record_to_message_info(
    r: MessageRecord,
    lifecycle: Option<(
        crate::storage::messages::OutboundRouteRecord,
        Vec<crate::storage::messages::OutboundAttemptRecord>,
    )>,
    canonical: Option<crate::storage::messages::CanonicalInboundRecord>,
) -> MessageInfo {
    let mut info = MessageInfo::default();
    info.id = r.id;
    info.source_hash = r.source;
    info.destination_hash = r.destination;
    info.timestamp = r.timestamp;
    info.content = r.content;
    info.title = if r.title.is_empty() { None } else { Some(r.title) };
    info.status = r.receipt_status.unwrap_or_default();
    info.is_outgoing = r.direction == "out";
    if let Some((route, attempts)) = lifecycle {
        info.lifecycle_state = match route.state.as_str() {
            "queued" => styrene_ipc::types::MessageLifecycleState::Queued,
            "sending" => styrene_ipc::types::MessageLifecycleState::Sending,
            "sent" => styrene_ipc::types::MessageLifecycleState::Sent,
            "delivered" => styrene_ipc::types::MessageLifecycleState::Delivered,
            "failed" => styrene_ipc::types::MessageLifecycleState::Failed,
            "cancelled" => styrene_ipc::types::MessageLifecycleState::Cancelled,
            "expired" => styrene_ipc::types::MessageLifecycleState::Expired,
            "rejected" => styrene_ipc::types::MessageLifecycleState::Rejected,
            _ => styrene_ipc::types::MessageLifecycleState::Unknown,
        };
        info.delivery_method = Some(route.actual_method.clone());
        info.requested_delivery_method = Some(route.requested_method);
        info.actual_delivery_method = Some(route.actual_method);
        info.fallback_reason = route.fallback_reason;
        info.correlation_id = Some(route.correlation_id);
        info.attempts =
            attempts.into_iter().map(crate::services::messaging::attempt_record_to_info).collect();
    }
    info.read = r.read;
    if let Some(canonical) = canonical {
        info.lxmf_timestamp = Some(canonical.timestamp);
        info.authentication_state = match canonical.authentication_state.as_str() {
            "verified" => styrene_ipc::types::MessageAuthenticationState::Verified,
            "invalid" => styrene_ipc::types::MessageAuthenticationState::Invalid,
            "unknown_identity" => styrene_ipc::types::MessageAuthenticationState::UnknownIdentity,
            "not_applicable" => styrene_ipc::types::MessageAuthenticationState::NotApplicable,
            _ => styrene_ipc::types::MessageAuthenticationState::Unknown,
        };
        info.stamp_state = match canonical.stamp_state.as_str() {
            "verified" => styrene_ipc::types::MessageStampState::Verified,
            "invalid" => styrene_ipc::types::MessageStampState::Invalid,
            "not_applicable" => styrene_ipc::types::MessageStampState::NotApplicable,
            _ => styrene_ipc::types::MessageStampState::Unknown,
        };
        info.stamp_value = canonical.stamp_value;
        info.stamp_cost = canonical.stamp_target;
    }
    info
}

/// Thin IPC-facing facade implementing the `Daemon` composite trait.
///
/// - Checks RBAC via `policy.has_capability(caller, cap)` before every delegation
/// - Delegates to the appropriate service through AppContext
/// - Maps service errors to IpcError
///
/// Replaces `RpcDaemon` as the IPC-facing type. `StubDaemon` (in `styrene-ipc`)
/// remains available for frontend testing without daemon infrastructure.
pub struct DaemonFacade {
    ctx: Arc<AppContext>,
    mobile_diagnostics: Option<Arc<crate::mobile_diagnostics::MobileDiagnostics>>,
    session_generation: Option<Arc<SessionGeneration>>,
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
        Self { ctx, mobile_diagnostics: None, session_generation: None, caller_identity }
    }

    pub(crate) fn new_mobile(
        ctx: Arc<AppContext>,
        caller_identity: String,
        diagnostics: Arc<crate::mobile_diagnostics::MobileDiagnostics>,
        session_generation: Arc<SessionGeneration>,
    ) -> Self {
        Self {
            ctx,
            mobile_diagnostics: Some(diagnostics),
            session_generation: Some(session_generation),
            caller_identity,
        }
    }

    /// Check a capability and return IpcError if denied.
    fn require(&self, capability: &str) -> Result<(), IpcError> {
        if self.ctx.policy().has_capability(&self.caller_identity, capability) {
            Ok(())
        } else {
            Err(IpcError::Denied { capability: capability.into() })
        }
    }

    fn authoritative_message(&self, message_id: &str) -> Result<Option<MessageInfo>, IpcError> {
        let Some(message) = self.ctx.messaging().get_message(message_id).map_err(internal)? else {
            return Ok(None);
        };
        let lifecycle = self.ctx.messaging().outbound_lifecycle(message_id).map_err(internal)?;
        let canonical = self.ctx.messaging().canonical_inbound(message_id).map_err(internal)?;
        let mut info = record_to_message_info(message, lifecycle, canonical);
        self.hydrate_retry_eligibility(&mut info)?;
        self.hydrate_attachments(&mut info)?;
        self.hydrate_propagation_correlations(&mut info)?;
        self.hydrate_delivery_evidence(&mut info)?;
        Ok(Some(info))
    }

    fn hydrate_retry_eligibility(&self, message: &mut MessageInfo) -> Result<(), IpcError> {
        let Some((eligible, reason)) =
            self.ctx.messaging().retry_eligibility(&message.id).map_err(internal)?
        else {
            return Ok(());
        };
        message.retry_eligible = Some(eligible);
        message.retry_ineligibility_reason = reason;
        Ok(())
    }

    fn hydrate_delivery_evidence(&self, message: &mut MessageInfo) -> Result<(), IpcError> {
        use styrene_ipc::types::{
            MessageDeliveryEvidenceInfo, MessageDeliveryEvidenceKind, MessageDeliveryEvidenceState,
        };
        message.terminal_detail =
            self.ctx.messaging().terminal_detail(&message.id).map_err(internal)?;
        message.delivery_evidence = self
            .ctx
            .messaging()
            .delivery_evidence(&message.id)
            .map_err(internal)?
            .into_iter()
            .map(|record| {
                let mut info = MessageDeliveryEvidenceInfo::default();
                info.kind = match record.kind.as_str() {
                    "packet_receipt" => MessageDeliveryEvidenceKind::PacketReceipt,
                    "resource_completion" => MessageDeliveryEvidenceKind::ResourceCompletion,
                    _ => MessageDeliveryEvidenceKind::Unknown,
                };
                info.hash = record.evidence_hash;
                info.representation = record.representation;
                info.state = match record.state.as_str() {
                    "tracked" => MessageDeliveryEvidenceState::Tracked,
                    "completed" => MessageDeliveryEvidenceState::Completed,
                    "failed" => MessageDeliveryEvidenceState::Failed,
                    "cancelled" => MessageDeliveryEvidenceState::Cancelled,
                    _ => MessageDeliveryEvidenceState::Unknown,
                };
                info.outcome = record.outcome;
                info.attempt = record.attempt_number;
                info.correlation_id = record.correlation_id;
                info.observed_at = record.observed_at;
                info.terminal_at = record.terminal_at;
                info.transferred_bytes = record.transferred_bytes;
                info.total_bytes = record.total_bytes;
                info.progress = record.progress;
                info
            })
            .collect();
        message.projection_complete = true;
        Ok(())
    }

    fn hydrate_attachments(&self, message: &mut MessageInfo) -> Result<(), IpcError> {
        message.attachments = self
            .ctx
            .messaging()
            .list_attachments(&message.id)
            .map_err(internal)?
            .into_iter()
            .map(attachment_record_to_info)
            .collect();
        message.attachment_info =
            (message.attachments.len() == 1).then(|| message.attachments[0].clone());
        Ok(())
    }

    fn hydrate_propagation_correlations(&self, message: &mut MessageInfo) -> Result<(), IpcError> {
        message.propagation_correlations = self
            .ctx
            .store()
            .lock()
            .map_err(|_| IpcError::Internal { message: "messages store lock poisoned".into() })?
            .standard_propagation_links_for_message(&message.id, 64)
            .map_err(|error| internal(std::io::Error::other(error)))?
            .into_iter()
            .map(|link| {
                let mut info = styrene_ipc::types::MessagePropagationCorrelationInfo::default();
                info.relation = link.relation;
                info.transient_id = hex::encode(link.transient_id);
                info.attempt_id = link.attempt_id.map(hex::encode);
                info.peer_hash = link.peer.map(hex::encode);
                info.state = link.state;
                info.created_at = link.created_at;
                info.updated_at = link.updated_at;
                info
            })
            .collect();
        Ok(())
    }

    fn emit_messaging_mutation(&self, outcome: &MessagingOperationOutcome) {
        if matches!(
            outcome.disposition,
            MessagingDisposition::Applied
                | MessagingDisposition::Created
                | MessagingDisposition::Updated
        ) {
            self.ctx.events().emit_messaging_operation(outcome.clone());
        }
    }

    fn conversation_info(
        &self,
        summary: crate::storage::messages::ConversationSummary,
    ) -> Result<ConversationInfo, IpcError> {
        let mut info = summary_to_conversation_info(summary);
        info.peer_name = self
            .ctx
            .messaging()
            .contact(&info.peer_hash)
            .map_err(internal)?
            .and_then(|contact| contact.alias)
            .and_then(|alias| (!alias.trim().is_empty()).then(|| alias.trim().to_owned()))
            .or_else(|| {
                self.ctx
                    .discovery()
                    .peer(&info.peer_hash)
                    .and_then(|peer| peer.display_name)
                    .and_then(|name| (!name.trim().is_empty()).then(|| name.trim().to_owned()))
            })
            .or_else(|| Some(info.peer_hash.chars().take(12).collect()));
        Ok(info)
    }

    fn emit_alias_invalidation(
        &self,
        peer_hash: &str,
        invalidation: Option<crate::storage::messages::ContactAliasInvalidation>,
    ) {
        let Some(invalidation) = invalidation else {
            return;
        };
        let reason = match invalidation {
            crate::storage::messages::ContactAliasInvalidation::Changed => {
                ConversationInvalidationReason::ContactAliasChanged
            }
            crate::storage::messages::ContactAliasInvalidation::Removed => {
                ConversationInvalidationReason::ContactAliasRemoved
            }
        };
        let mut invalidation = ConversationInvalidation::default();
        invalidation.peer_hash = peer_hash.to_owned();
        invalidation.reason = reason;
        self.ctx.events().emit_conversation_invalidation(invalidation);
    }

    async fn conversation_flag_outcome(
        &self,
        peer_hash: &str,
        flag: &str,
        value: bool,
    ) -> Result<MessagingOperationOutcome, IpcError> {
        self.require(Capability::MESSAGING_MANAGE)?;
        let result = self
            .ctx
            .messaging()
            .set_conversation_flag_outcome(peer_hash, flag, value)
            .map_err(messaging_error)?;
        let mut outcome = MessagingOperationOutcome::default();
        outcome.disposition = mutation_disposition(result.disposition);
        outcome.affected_count = result.affected_count;
        outcome.target_id = peer_hash.to_ascii_lowercase();
        outcome.conversation = result.summary.map(summary_to_conversation_info);
        self.emit_messaging_mutation(&outcome);
        Ok(outcome)
    }

    fn not_implemented(method: &str) -> IpcError {
        IpcError::not_implemented(method)
    }

    fn network_operation_capability(
        kind: styrene_ipc::types::NetworkOperationKind,
    ) -> Result<&'static str, IpcError> {
        use styrene_ipc::types::NetworkOperationKind;

        match kind {
            NetworkOperationKind::Announce => Ok(Capability::NETWORK_ANNOUNCE),
            NetworkOperationKind::PathRequest => Ok(Capability::NETWORK_PATH_REQUEST),
            NetworkOperationKind::Probe => Ok(Capability::NETWORK_PROBE),
            NetworkOperationKind::LinkOpen => Ok(Capability::NETWORK_LINK_OPEN),
            NetworkOperationKind::LinkClose => Ok(Capability::NETWORK_LINK_CLOSE),
            _ => Err(IpcError::invalid_request("unsupported network operation kind")),
        }
    }
}

#[async_trait]
impl DaemonIdentity for DaemonFacade {
    async fn query_identity(&self) -> Result<IdentityInfo, IpcError> {
        self.require(Capability::RPC_STATUS)?;
        let svc = self.ctx.identity();
        let runtime_identity = self.ctx.transport().runtime_identity();
        let identity = if let Some((identity, _)) = runtime_identity {
            hex::encode(identity.as_slice())
        } else {
            svc.identity_hash().to_string()
        };
        let dest = if let Some((_, destination)) = runtime_identity {
            hex::encode(destination.as_slice())
        } else {
            svc.delivery_destination_hash().unwrap_or_default()
        };
        let mut info = IdentityInfo::default();
        info.identity_hash = identity;
        info.destination_hash = dest.clone();
        info.lxmf_destination_hash = dest;
        info.display_name = svc.display_name().unwrap_or_default();
        info.icon = svc.icon();
        info.short_name = svc.short_name();
        info.custody = svc.custody();
        Ok(info)
    }

    async fn query_identity_backup_metadata(
        &self,
    ) -> Result<styrene_ipc::types::IdentityBackupMetadata, IpcError> {
        self.require(Capability::RPC_STATUS)?;
        self.ctx.identity().identity_backup_metadata()
    }

    async fn export_identity_backup(
        &self,
    ) -> Result<styrene_ipc::types::IdentityBackupExport, IpcError> {
        self.require(Capability::RPC_CONFIG_UPDATE)?;
        self.ctx.identity().export_identity_backup()
    }

    async fn restore_identity_backup(
        &self,
        backup: styrene_ipc::types::IdentityBackupImport,
    ) -> Result<styrene_ipc::types::IdentityRestoreOutcome, IpcError> {
        self.require(Capability::RPC_CONFIG_UPDATE)?;
        self.ctx.identity().restore_identity_backup(backup)
    }

    async fn set_identity(
        &self,
        display_name: Option<&str>,
        icon: Option<&str>,
        short_name: Option<&str>,
    ) -> Result<bool, IpcError> {
        self.require(Capability::RPC_CONFIG_UPDATE)?;
        let changed = self
            .ctx
            .identity()
            .set_identity_validated(display_name, icon, short_name)
            .map_err(IpcError::invalid_request)?;
        if changed {
            // Re-announce with updated identity
            self.ctx.identity().announce(None).await;
        }
        Ok(changed)
    }

    async fn announce(&self) -> Result<bool, IpcError> {
        self.require(Capability::NETWORK_ANNOUNCE)?;
        self.ctx.identity().announce(None).await;
        self.ctx
            .network_operations()
            .announce_propagation(tokio::time::Instant::now() + std::time::Duration::from_secs(10))
            .await
            .map_err(|error| IpcError::Transport { message: error.to_string() })?;
        Ok(true)
    }
}

#[async_trait]
impl DaemonMessaging for DaemonFacade {
    async fn start_conversation(
        &self,
        peer_hash: &str,
    ) -> Result<MessagingOperationOutcome, IpcError> {
        self.require(Capability::MESSAGING_MANAGE)?;
        let result = self.ctx.messaging().start_conversation(peer_hash).map_err(messaging_error)?;
        let mut outcome = MessagingOperationOutcome::default();
        outcome.disposition = mutation_disposition(result.disposition);
        outcome.affected_count = result.affected_count;
        outcome.target_id = peer_hash.to_ascii_lowercase();
        outcome.conversation =
            result.summary.map(|summary| self.conversation_info(summary)).transpose()?;
        self.emit_messaging_mutation(&outcome);
        Ok(outcome)
    }

    async fn send_chat(&self, request: SendChatRequest) -> Result<MessageId, IpcError> {
        if request
            .delivery_method
            .as_deref()
            .is_some_and(|method| method.trim().eq_ignore_ascii_case("paper"))
        {
            return Err(IpcError::invalid_request(
                "paper export requires send_chat_outcome so the URI is not discarded",
            ));
        }
        let outcome = self.send_chat_outcome(request).await?;
        match outcome.disposition {
            SendChatDisposition::Accepted => Ok(outcome.message_id),
            SendChatDisposition::Failed => Err(authoritative_send_error(
                outcome
                    .terminal_error
                    .unwrap_or_else(|| "authoritative send failed after persistence".into()),
            )),
            SendChatDisposition::PaperExported | SendChatDisposition::Unknown => {
                Err(IpcError::invalid_request("send_chat_outcome is required for this result"))
            }
            _ => Err(IpcError::invalid_request("unsupported send outcome")),
        }
    }

    async fn send_chat_outcome(
        &self,
        request: SendChatRequest,
    ) -> Result<SendChatOutcome, IpcError> {
        self.require(Capability::CHAT_SEND)?;
        if request.content.is_empty() || request.content.len() > MAX_CHAT_CONTENT_BYTES {
            return Err(IpcError::invalid_request("content must be 1..=65536 UTF-8 bytes"));
        }
        let requested_method =
            request.delivery_method.as_deref().unwrap_or("direct").trim().to_ascii_lowercase();
        if !matches!(requested_method.as_str(), "direct" | "opportunistic" | "propagated" | "paper")
        {
            return Err(IpcError::invalid_request("invalid delivery_method"));
        }
        if request.attachment.is_some() && !request.attachments.is_empty() {
            return Err(IpcError::invalid_request(
                "legacy attachment and attachments are mutually exclusive",
            ));
        }
        if request.attachment.is_none() && request.attachment_name.is_some() {
            return Err(IpcError::invalid_request("attachment_name requires attachment"));
        }
        let mut inputs = request.attachments;
        if let Some(bytes) = request.attachment {
            let mut input = AttachmentInput::default();
            input.name = request.attachment_name.unwrap_or_else(|| "attachment.bin".into());
            input.bytes = bytes;
            inputs.push(input);
        }
        if inputs.len() > MAX_CHAT_ATTACHMENTS {
            return Err(IpcError::invalid_request("attachment count exceeds 8"));
        }
        if requested_method == "paper" && !inputs.is_empty() {
            return Err(IpcError::invalid_request("paper delivery does not support attachments"));
        }
        let mut aggregate = 0usize;
        let mut attachments = Vec::with_capacity(inputs.len());
        for input in inputs {
            if input.name.is_empty()
                || input.name.len() > MAX_CHAT_ATTACHMENT_NAME_BYTES
                || input.bytes.len() > MAX_CHAT_ATTACHMENT_BYTES
            {
                return Err(IpcError::invalid_request("invalid attachment name or size"));
            }
            aggregate = aggregate
                .checked_add(input.bytes.len())
                .ok_or_else(|| IpcError::invalid_request("attachment aggregate overflow"))?;
            if aggregate > MAX_CHAT_ATTACHMENT_BYTES {
                return Err(IpcError::invalid_request("attachment aggregate exceeds 768 KiB"));
            }
            let digest = hex::encode(sha2::Sha256::digest(&input.bytes));
            if let Some(expected) = input.expected_sha256.as_deref() {
                if expected.len() != 64
                    || !expected.bytes().all(|byte| byte.is_ascii_hexdigit())
                    || expected.bytes().any(|byte| byte.is_ascii_uppercase())
                {
                    return Err(IpcError::invalid_request(
                        "expected_sha256 must be 64 lowercase hex characters",
                    ));
                }
                if expected != digest {
                    return Err(IpcError::invalid_request("attachment SHA-256 mismatch"));
                }
            }
            attachments.push(crate::storage::messages::AttachmentBlobInput {
                wire_name: input.name,
                data: input.bytes,
                content_type: input.content_type,
                source: "local".into(),
            });
        }
        let mut committed = self
            .ctx
            .messaging()
            .send_chat_outcome_with_attachments(
                &request.peer_hash,
                &request.content,
                request.title.as_deref(),
                Some(&requested_method),
                &attachments,
            )
            .await
            .map_err(messaging_error)?;
        let mut message = committed.message.clone();
        let projection_error = match self.authoritative_message(&committed.message_id) {
            Ok(Some(fresh)) if fresh.id == committed.message_id => {
                message = fresh;
                None
            }
            Ok(Some(fresh)) => Some(format!(
                "persisted send projection ID mismatch: expected {}, observed {}",
                committed.message_id, fresh.id
            )),
            Ok(None) => Some("persisted send projection is unavailable".into()),
            Err(error) => Some(format!("persisted send projection freshness unavailable: {error}")),
        };
        if let Some(error) = projection_error {
            if !committed
                .terminal_error
                .as_deref()
                .is_some_and(|existing| existing.contains(&error))
            {
                committed.terminal_error = Some(match committed.terminal_error {
                    Some(existing) => format!("{existing}; {error}"),
                    None => error,
                });
            }
            committed.disposition = crate::services::messaging::SendCommitDisposition::Failed;
            committed.paper_uri = None;
        }
        let mut outcome = SendChatOutcome::default();
        outcome.disposition = match committed.disposition {
            crate::services::messaging::SendCommitDisposition::Accepted => {
                SendChatDisposition::Accepted
            }
            crate::services::messaging::SendCommitDisposition::Failed => {
                SendChatDisposition::Failed
            }
            crate::services::messaging::SendCommitDisposition::PaperExported => {
                SendChatDisposition::PaperExported
            }
        };
        outcome.message_id = committed.message_id;
        outcome.message = message;
        outcome.requested_method = committed.requested_method;
        outcome.actual_method = committed.actual_method;
        outcome.fallback_reason = committed.fallback_reason;
        outcome.terminal_error = committed.terminal_error;
        outcome.paper_uri = committed.paper_uri;
        Ok(outcome)
    }

    async fn set_draft(
        &self,
        peer_hash: &str,
        content: &str,
    ) -> Result<ConversationDraft, IpcError> {
        self.require(Capability::MESSAGING_MANAGE)?;
        let draft = self.ctx.messaging().set_draft(peer_hash, content).map_err(messaging_error)?;
        let mut result = ConversationDraft::default();
        result.peer_hash = draft.peer_hash;
        result.content = draft.content;
        result.updated_at = draft.updated_at;
        result.revision = draft.revision;
        Ok(result)
    }

    async fn draft(&self, peer_hash: &str) -> Result<Option<ConversationDraft>, IpcError> {
        self.require(Capability::MESSAGING_MANAGE)?;
        Ok(self.ctx.messaging().draft(peer_hash).map_err(messaging_error)?.map(|draft| {
            let mut result = ConversationDraft::default();
            result.peer_hash = draft.peer_hash;
            result.content = draft.content;
            result.updated_at = draft.updated_at;
            result.revision = draft.revision;
            result
        }))
    }

    async fn clear_draft(&self, peer_hash: &str) -> Result<MessagingDisposition, IpcError> {
        self.require(Capability::MESSAGING_MANAGE)?;
        Ok(if self.ctx.messaging().clear_draft(peer_hash).map_err(messaging_error)? {
            MessagingDisposition::Applied
        } else {
            MessagingDisposition::Unchanged
        })
    }

    async fn clear_draft_if_revision(
        &self,
        peer_hash: &str,
        revision: u64,
    ) -> Result<MessagingDisposition, IpcError> {
        self.require(Capability::MESSAGING_MANAGE)?;
        Ok(
            if self
                .ctx
                .messaging()
                .clear_draft_if_revision(peer_hash, revision)
                .map_err(messaging_error)?
            {
                MessagingDisposition::Applied
            } else {
                MessagingDisposition::Unchanged
            },
        )
    }

    async fn mark_read(&self, peer_hash: &str) -> Result<u64, IpcError> {
        Ok(self.mark_read_outcome(peer_hash).await?.affected_count)
    }

    async fn mark_read_outcome(
        &self,
        peer_hash: &str,
    ) -> Result<MessagingOperationOutcome, IpcError> {
        self.require(Capability::MESSAGING_MANAGE)?;
        let result = self.ctx.messaging().mark_read_outcome(peer_hash).map_err(messaging_error)?;
        let mut outcome = MessagingOperationOutcome::default();
        outcome.disposition = mutation_disposition(result.disposition);
        outcome.affected_count = result.affected_count;
        outcome.target_id = peer_hash.to_ascii_lowercase();
        outcome.conversation = result.summary.map(summary_to_conversation_info);
        self.emit_messaging_mutation(&outcome);
        Ok(outcome)
    }

    async fn delete_conversation(&self, peer_hash: &str) -> Result<u64, IpcError> {
        Ok(self.delete_conversation_outcome(peer_hash).await?.affected_count)
    }

    async fn delete_conversation_outcome(
        &self,
        peer_hash: &str,
    ) -> Result<MessagingOperationOutcome, IpcError> {
        self.require(Capability::MESSAGING_MANAGE)?;
        let result =
            self.ctx.messaging().delete_conversation_outcome(peer_hash).map_err(messaging_error)?;
        let mut outcome = MessagingOperationOutcome::default();
        outcome.disposition = mutation_disposition(result.disposition);
        outcome.affected_count = result.affected_count;
        outcome.target_id = peer_hash.to_ascii_lowercase();
        outcome.conversation = result.summary.map(summary_to_conversation_info);
        outcome.terminal_state = result.terminal_state;
        self.emit_messaging_mutation(&outcome);
        Ok(outcome)
    }

    async fn delete_message(&self, message_id: &str) -> Result<bool, IpcError> {
        Ok(self.delete_message_outcome(message_id).await?.disposition
            == MessagingDisposition::Applied)
    }

    async fn delete_message_outcome(
        &self,
        message_id: &str,
    ) -> Result<MessagingOperationOutcome, IpcError> {
        self.require(Capability::MESSAGING_MANAGE)?;
        let result =
            self.ctx.messaging().delete_message_outcome(message_id).map_err(messaging_error)?;
        let mut outcome = MessagingOperationOutcome::default();
        outcome.disposition = mutation_disposition(result.disposition);
        outcome.affected_count = result.affected_count;
        outcome.target_id = message_id.into();
        outcome.terminal_state = result.terminal_state;
        if outcome.disposition == MessagingDisposition::TerminalConflict {
            outcome.message = self.authoritative_message(message_id)?;
        }
        self.emit_messaging_mutation(&outcome);
        Ok(outcome)
    }

    async fn retry_message(&self, message_id: &str) -> Result<bool, IpcError> {
        Ok(self.retry_message_outcome(message_id).await?.disposition
            == MessagingDisposition::Applied)
    }

    async fn retry_message_outcome(
        &self,
        message_id: &str,
    ) -> Result<MessagingOperationOutcome, IpcError> {
        self.require(Capability::MESSAGING_LIFECYCLE)?;
        use crate::services::messaging::RetryMessageOutcome as Retry;
        let result = self
            .ctx
            .messaging()
            .retry_message_outcome(message_id)
            .await
            .map_err(messaging_error)?;
        let (disposition, correlated_id, terminal_state) = match result {
            Retry::Created(id) => (MessagingDisposition::Applied, Some(id), None),
            Retry::Existing(id) => (MessagingDisposition::Unchanged, Some(id), None),
            Retry::NotFound => (MessagingDisposition::NotFound, None, None),
            Retry::TerminalConflict(state) => {
                (MessagingDisposition::TerminalConflict, None, Some(state))
            }
        };
        let message = match correlated_id.as_deref() {
            Some(id) => self.authoritative_message(id)?,
            None => self.authoritative_message(message_id)?,
        };
        let mut outcome = MessagingOperationOutcome::default();
        outcome.disposition = disposition;
        outcome.affected_count = u64::from(disposition == MessagingDisposition::Applied);
        outcome.target_id = message_id.into();
        outcome.correlated_id = correlated_id;
        outcome.message = message;
        outcome.terminal_state = terminal_state;
        self.emit_messaging_mutation(&outcome);
        Ok(outcome)
    }

    async fn cancel_message(&self, message_id: &str) -> Result<bool, IpcError> {
        Ok(self.cancel_message_outcome(message_id).await?.disposition
            == MessagingDisposition::Applied)
    }

    async fn cancel_message_outcome(
        &self,
        message_id: &str,
    ) -> Result<MessagingOperationOutcome, IpcError> {
        self.require(Capability::MESSAGING_LIFECYCLE)?;
        use crate::services::messaging::CancelMessageOutcome as Cancel;
        let result = self
            .ctx
            .messaging()
            .cancel_outbound_outcome(message_id)
            .await
            .map_err(messaging_error)?;
        let (disposition, terminal_state) = match result {
            Cancel::Applied(state) => (MessagingDisposition::Applied, Some(state)),
            Cancel::AlreadyCancelled => {
                (MessagingDisposition::AlreadyCancelled, Some("cancelled".into()))
            }
            Cancel::NotFound => (MessagingDisposition::NotFound, None),
            Cancel::TerminalConflict(state) => {
                (MessagingDisposition::TerminalConflict, Some(state))
            }
        };
        let mut outcome = MessagingOperationOutcome::default();
        outcome.disposition = disposition;
        outcome.affected_count = u64::from(disposition == MessagingDisposition::Applied);
        outcome.target_id = message_id.into();
        outcome.message = self.authoritative_message(message_id)?;
        outcome.terminal_state = terminal_state;
        self.emit_messaging_mutation(&outcome);
        Ok(outcome)
    }

    async fn query_conversations(
        &self,
        unread_only: bool,
    ) -> Result<Vec<ConversationInfo>, IpcError> {
        self.require(Capability::MESSAGING_HISTORY_READ)?;
        let summaries = self.ctx.messaging().list_conversations(unread_only).map_err(internal)?;
        summaries.into_iter().map(|summary| self.conversation_info(summary)).collect()
    }

    async fn query_conversation_page(
        &self,
        unread_only: bool,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<ConversationPage, IpcError> {
        self.require(Capability::MESSAGING_HISTORY_READ)?;
        let page = self
            .ctx
            .messaging()
            .conversation_page(unread_only, limit as usize, cursor)
            .map_err(page_error)?;
        let mut result = ConversationPage::default();
        result.conversations = page
            .items
            .into_iter()
            .map(|summary| self.conversation_info(summary))
            .collect::<Result<Vec<_>, _>>()?;
        result.next_cursor = page.next_cursor;
        Ok(result)
    }

    async fn query_messages(
        &self,
        peer_hash: &str,
        limit: u32,
        before_ts: Option<i64>,
    ) -> Result<Vec<MessageInfo>, IpcError> {
        self.require(Capability::MESSAGING_HISTORY_READ)?;
        let snapshots = self
            .ctx
            .messaging()
            .message_projection_snapshot_for_peer(peer_hash, limit as usize, before_ts)
            .map_err(messaging_error)?;
        snapshots
            .into_iter()
            .map(|snapshot| {
                let mut info = record_to_message_info(
                    snapshot.message,
                    snapshot.lifecycle,
                    snapshot.canonical,
                );
                self.hydrate_retry_eligibility(&mut info)?;
                self.hydrate_attachments(&mut info)?;
                self.hydrate_propagation_correlations(&mut info)?;
                self.hydrate_delivery_evidence(&mut info)?;
                Ok(info)
            })
            .collect()
    }

    async fn query_message(&self, message_id: &str) -> Result<Option<MessageInfo>, IpcError> {
        self.require(Capability::MESSAGING_HISTORY_READ)?;
        if message_id.is_empty() || message_id.len() > 128 {
            return Err(IpcError::invalid_request(
                "message_id must contain between 1 and 128 bytes",
            ));
        }
        self.authoritative_message(message_id)
    }

    async fn query_message_page(
        &self,
        peer_hash: &str,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<MessagePage, IpcError> {
        self.require(Capability::MESSAGING_HISTORY_READ)?;
        let page = self
            .ctx
            .messaging()
            .message_projection_page_for_peer(peer_hash, limit as usize, cursor)
            .map_err(page_error)?;
        let mut result = MessagePage::default();
        result.messages = page
            .items
            .into_iter()
            .map(|snapshot| {
                let mut info = record_to_message_info(
                    snapshot.message,
                    snapshot.lifecycle,
                    snapshot.canonical,
                );
                self.hydrate_retry_eligibility(&mut info)?;
                self.hydrate_attachments(&mut info)?;
                self.hydrate_propagation_correlations(&mut info)?;
                self.hydrate_delivery_evidence(&mut info)?;
                Ok(info)
            })
            .collect::<Result<Vec<_>, IpcError>>()?;
        result.next_cursor = page.next_cursor;
        Ok(result)
    }

    async fn search_messages(
        &self,
        query: &str,
        peer_hash: Option<&str>,
        limit: u32,
    ) -> Result<Vec<MessageInfo>, IpcError> {
        Ok(self.search_messages_outcome(query, peer_hash, limit).await?.messages)
    }

    async fn search_messages_outcome(
        &self,
        query: &str,
        peer_hash: Option<&str>,
        limit: u32,
    ) -> Result<MessageSearchOutcome, IpcError> {
        self.require(Capability::MESSAGING_HISTORY_READ)?;
        let snapshot = self
            .ctx
            .messaging()
            .search_message_projection_outcome(query, peer_hash, limit as usize)
            .map_err(messaging_error)?;
        let messages = snapshot
            .items
            .into_iter()
            .map(|snapshot| {
                let mut info = record_to_message_info(
                    snapshot.message,
                    snapshot.lifecycle,
                    snapshot.canonical,
                );
                self.hydrate_retry_eligibility(&mut info)?;
                self.hydrate_attachments(&mut info)?;
                self.hydrate_propagation_correlations(&mut info)?;
                self.hydrate_delivery_evidence(&mut info)?;
                Ok(info)
            })
            .collect::<Result<Vec<_>, IpcError>>()?;
        let mut outcome = MessageSearchOutcome::default();
        outcome.returned_count = messages.len() as u32;
        outcome.matched_count = snapshot.matched_count;
        outcome.truncated = snapshot.truncated;
        outcome.messages = messages;
        outcome.order = "timestamp_desc_id_desc".into();
        outcome.query = query.into();
        outcome.peer_hash = peer_hash.map(str::to_owned);
        outcome.limit = limit;
        Ok(outcome)
    }

    async fn query_attachment(&self, message_id: &str) -> Result<Vec<u8>, IpcError> {
        self.require(Capability::MESSAGING_HISTORY_READ)?;
        let attachments = self.ctx.messaging().list_attachments(message_id).map_err(internal)?;
        if attachments.len() != 1 || attachments[0].integrity != "verified" {
            return Err(IpcError::not_found("attachment", message_id));
        }
        let mut data = Vec::with_capacity(attachments[0].byte_len as usize);
        let mut offset = 0usize;
        loop {
            let chunk = self
                .ctx
                .messaging()
                .query_attachment_chunk(message_id, 0, offset, 256 * 1024)
                .map_err(internal)?
                .ok_or_else(|| IpcError::not_found("attachment", message_id))?;
            data.extend_from_slice(&chunk.data);
            offset = chunk.next_offset;
            if chunk.done {
                return Ok(data);
            }
        }
    }

    async fn list_attachments(&self, message_id: &str) -> Result<Vec<AttachmentInfo>, IpcError> {
        self.require(Capability::MESSAGING_HISTORY_READ)?;
        self.ctx
            .messaging()
            .list_attachments(message_id)
            .map_err(internal)
            .map(|values| values.into_iter().map(attachment_record_to_info).collect())
    }

    async fn query_attachment_chunk(
        &self,
        message_id: &str,
        ordinal: u8,
        offset: u64,
        max_bytes: u32,
    ) -> Result<AttachmentChunk, IpcError> {
        self.require(Capability::MESSAGING_HISTORY_READ)?;
        if max_bytes == 0 || max_bytes > 256 * 1024 {
            return Err(IpcError::invalid_request("max_bytes must be between 1 and 262144"));
        }
        let offset = usize::try_from(offset)
            .map_err(|_| IpcError::invalid_request("attachment offset exceeds platform range"))?;
        let chunk = self
            .ctx
            .messaging()
            .query_attachment_chunk(message_id, ordinal, offset, max_bytes as usize)
            .map_err(internal)?
            .ok_or_else(|| IpcError::not_found("attachment", message_id))?;
        let mut result = AttachmentChunk::default();
        result.attachment = attachment_record_to_info(chunk.attachment);
        result.data = chunk.data;
        result.next_offset = chunk.next_offset as u64;
        result.done = chunk.done;
        Ok(result)
    }

    async fn cancel_attachment_transfer(
        &self,
        message_id: &str,
    ) -> Result<MessagingOperationOutcome, IpcError> {
        self.require(Capability::MESSAGING_LIFECYCLE)?;
        let attachments = self.ctx.messaging().list_attachments(message_id).map_err(internal)?;
        let Some(transfer) = attachments.iter().find_map(|attachment| {
            attachment.transfer_state.as_ref().map(|state| {
                (
                    attachment.representation.as_deref().unwrap_or(""),
                    state.as_str(),
                    attachment.direction.as_deref().unwrap_or(""),
                )
            })
        }) else {
            return Err(IpcError::not_found("attachment transfer", message_id));
        };
        if transfer.2 != "outbound" {
            let mut outcome = MessagingOperationOutcome::default();
            outcome.disposition = MessagingDisposition::TerminalConflict;
            outcome.target_id = message_id.into();
            outcome.terminal_state = Some(transfer.1.into());
            return Ok(outcome);
        }
        if transfer.0 != "resource" {
            let mut outcome = MessagingOperationOutcome::default();
            outcome.disposition = MessagingDisposition::TerminalConflict;
            outcome.target_id = message_id.into();
            outcome.terminal_state = Some("packet_dispatched".into());
            return Ok(outcome);
        }
        if matches!(transfer.1, "completed" | "failed" | "cancelled") {
            let mut outcome = MessagingOperationOutcome::default();
            outcome.disposition = if transfer.1 == "cancelled" {
                MessagingDisposition::AlreadyCancelled
            } else {
                MessagingDisposition::TerminalConflict
            };
            outcome.target_id = message_id.into();
            outcome.terminal_state = Some(transfer.1.into());
            return Ok(outcome);
        }
        self.cancel_message_outcome(message_id).await
    }

    async fn query_attachment_transfer(
        &self,
        message_id: &str,
    ) -> Result<AttachmentTransferInfo, IpcError> {
        self.require(Capability::MESSAGING_HISTORY_READ)?;
        self.list_attachments(message_id)
            .await?
            .into_iter()
            .find_map(|attachment| attachment.transfer.map(|transfer| *transfer))
            .ok_or_else(|| IpcError::not_found("attachment transfer", message_id))
    }

    async fn set_contact(
        &self,
        peer_hash: &str,
        alias: Option<&str>,
        notes: Option<&str>,
    ) -> Result<ContactInfo, IpcError> {
        self.set_contact_outcome(peer_hash, alias, notes).await?.contact.ok_or_else(|| {
            IpcError::Internal { message: "contact outcome missing projection".into() }
        })
    }

    async fn set_contact_outcome(
        &self,
        peer_hash: &str,
        alias: Option<&str>,
        notes: Option<&str>,
    ) -> Result<MessagingOperationOutcome, IpcError> {
        self.require(Capability::MESSAGING_MANAGE)?;
        let result = self
            .ctx
            .messaging()
            .set_contact_outcome(peer_hash, alias, notes)
            .map_err(messaging_error)?;
        let alias_invalidation = result.alias_invalidation;
        let mut outcome = MessagingOperationOutcome::default();
        outcome.disposition = mutation_disposition(result.disposition);
        outcome.affected_count = result.affected_count;
        outcome.target_id = peer_hash.to_ascii_lowercase();
        outcome.contact = result.contact.map(contact_to_info);
        self.emit_messaging_mutation(&outcome);
        self.emit_alias_invalidation(&outcome.target_id, alias_invalidation);
        Ok(outcome)
    }

    async fn remove_contact(&self, peer_hash: &str) -> Result<bool, IpcError> {
        Ok(self.remove_contact_outcome(peer_hash).await?.disposition
            == MessagingDisposition::Applied)
    }

    async fn remove_contact_outcome(
        &self,
        peer_hash: &str,
    ) -> Result<MessagingOperationOutcome, IpcError> {
        self.require(Capability::MESSAGING_MANAGE)?;
        let result =
            self.ctx.messaging().remove_contact_outcome(peer_hash).map_err(messaging_error)?;
        let alias_invalidation = result.alias_invalidation;
        let mut outcome = MessagingOperationOutcome::default();
        outcome.disposition = mutation_disposition(result.disposition);
        outcome.affected_count = result.affected_count;
        outcome.target_id = peer_hash.to_ascii_lowercase();
        self.emit_messaging_mutation(&outcome);
        self.emit_alias_invalidation(&outcome.target_id, alias_invalidation);
        Ok(outcome)
    }

    async fn query_contacts(&self) -> Result<Vec<ContactInfo>, IpcError> {
        self.require(Capability::MESSAGING_HISTORY_READ)?;
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
        self.require(Capability::RPC_STATUS)?;
        Ok(self.ctx.discovery().resolve_name(name, prefix))
    }

    async fn pin_conversation(&self, peer_hash: &str) -> Result<bool, IpcError> {
        Ok(self.pin_conversation_outcome(peer_hash).await?.disposition
            == MessagingDisposition::Applied)
    }

    async fn pin_conversation_outcome(
        &self,
        peer_hash: &str,
    ) -> Result<MessagingOperationOutcome, IpcError> {
        self.conversation_flag_outcome(peer_hash, "pinned", true).await
    }

    async fn unpin_conversation(&self, peer_hash: &str) -> Result<bool, IpcError> {
        Ok(self.unpin_conversation_outcome(peer_hash).await?.disposition
            == MessagingDisposition::Applied)
    }

    async fn unpin_conversation_outcome(
        &self,
        peer_hash: &str,
    ) -> Result<MessagingOperationOutcome, IpcError> {
        self.conversation_flag_outcome(peer_hash, "pinned", false).await
    }

    async fn mute_conversation(&self, peer_hash: &str) -> Result<bool, IpcError> {
        Ok(self.mute_conversation_outcome(peer_hash).await?.disposition
            == MessagingDisposition::Applied)
    }

    async fn mute_conversation_outcome(
        &self,
        peer_hash: &str,
    ) -> Result<MessagingOperationOutcome, IpcError> {
        self.conversation_flag_outcome(peer_hash, "muted", true).await
    }

    async fn unmute_conversation(&self, peer_hash: &str) -> Result<bool, IpcError> {
        Ok(self.unmute_conversation_outcome(peer_hash).await?.disposition
            == MessagingDisposition::Applied)
    }

    async fn unmute_conversation_outcome(
        &self,
        peer_hash: &str,
    ) -> Result<MessagingOperationOutcome, IpcError> {
        self.conversation_flag_outcome(peer_hash, "muted", false).await
    }
}

#[async_trait]
impl DaemonStatus for DaemonFacade {
    async fn query_status(&self) -> Result<DaemonStatusInfo, IpcError> {
        self.require(Capability::RPC_STATUS)?;
        let status = self.ctx.status();
        let mut info = DaemonStatusInfo::default();
        info.uptime = status.uptime_secs();
        info.daemon_version = env!("CARGO_PKG_VERSION").to_string();
        info.rns_initialized = self.ctx.transport().is_connected();
        info.lxmf_initialized = self.ctx.transport().is_connected();
        info.device_count = self.ctx.discovery().peer_count() as u32;
        let interfaces = self.ctx.transport().interface_snapshots().await;
        let connection_generation = self.session_generation.as_ref().map_or_else(
            || interfaces.iter().map(|interface| interface.generation).max(),
            |generation| Some(generation.observe(&interfaces)),
        );
        info.interface_count = interfaces.len() as u32;
        info.propagation_enabled = status.propagation_enabled();
        if let Some(contract) = self.ctx.startup_contract() {
            info.standard_lxmf_propagation_destination_registered = contract.has_component(
                crate::startup_contract::components::STANDARD_LXMF_PROPAGATION_DESTINATION,
            );
            info.standard_lxmf_propagation_active = contract
                .advertises(crate::startup_contract::capabilities::STANDARD_LXMF_PROPAGATION.id());
        }
        if let Ok((count, size)) = self.ctx.propagation().stats() {
            info.propagation_count = count as u32;
            info.propagation_size_bytes = size;
        }
        info.transport_enabled = self.ctx.transport().is_connected();
        if let Some(contract) = self.ctx.startup_contract() {
            let active = contract.active_capabilities(
                self.ctx.policy().authorized_capabilities(&self.caller_identity),
            );
            let mut capabilities = styrene_ipc::types::ActiveCapabilitiesInfo::default();
            capabilities.version = styrene_ipc::types::ACTIVE_CAPABILITIES_VERSION;
            capabilities.generation = connection_generation
                .or((contract.runtime() == crate::startup_contract::RuntimeKind::Mobile)
                    .then_some(1));
            capabilities.runtime = active.runtime().iter().map(|id| (*id).to_string()).collect();
            capabilities.degraded = active
                .degraded()
                .iter()
                .map(|degraded| {
                    let mut info = styrene_ipc::types::DegradedCapabilityInfo::default();
                    info.id = degraded.id().to_string();
                    info.reason = degraded.reason().to_string();
                    info.reason_code = active
                        .failures()
                        .iter()
                        .find(|failure| failure.id() == degraded.id())
                        .map(|failure| capability_failure_code(failure.kind()))
                        .unwrap_or(CapabilityFailureCode::Degraded);
                    info
                })
                .collect();
            capabilities.failures = active
                .failures()
                .iter()
                .map(|failure| {
                    let mut info = styrene_ipc::types::CapabilityFailureInfo::default();
                    info.id = failure.id().to_string();
                    info.code = capability_failure_code(failure.kind());
                    info.retryable = failure.retryable();
                    info
                })
                .collect();
            capabilities.authorized_operations = active.authorized_operations().to_vec();
            info.active_capabilities = Some(capabilities);
        }
        Ok(info)
    }

    async fn mobile_diagnostics(&self) -> Result<MobileDiagnosticSnapshot, IpcError> {
        self.require(Capability::RPC_STATUS)?;
        self.mobile_diagnostics.as_ref().map(|diagnostics| diagnostics.snapshot()).ok_or_else(
            || IpcError::Unavailable { reason: "mobile diagnostics unavailable".into() },
        )
    }

    async fn export_mobile_diagnostics(&self) -> Result<MobileDiagnosticExport, IpcError> {
        self.require(Capability::RPC_STATUS)?;
        self.mobile_diagnostics
            .as_ref()
            .ok_or_else(|| IpcError::Unavailable {
                reason: "mobile diagnostics unavailable".into(),
            })?
            .export()
            .map_err(|error| IpcError::Internal { message: error.to_string() })
    }

    async fn query_standard_propagation(&self) -> Result<StandardPropagationSnapshot, IpcError> {
        self.require(Capability::RPC_STATUS)?;
        let mut snapshot = StandardPropagationSnapshot::default();
        snapshot.version = STANDARD_PROPAGATION_SNAPSHOT_VERSION;
        snapshot.selection_readiness = StandardPropagationSelectionReadiness::Unavailable;
        snapshot.sync_readiness = StandardPropagationSyncReadiness::Unavailable;
        let Some(runtime) = self.ctx.standard_propagation() else {
            return Ok(snapshot);
        };
        snapshot.registered = runtime.is_registered();
        snapshot.selection_readiness = StandardPropagationSelectionReadiness::NoSelection;
        let runtime_policy = runtime.policy();
        let storage_policy = crate::storage::standard_propagation::StandardPropagationPolicy {
            queue_max_count: runtime_policy.queue_max_count,
            queue_max_bytes: runtime_policy.queue_max_bytes,
            expiry_secs: runtime_policy.expiry_secs,
        };
        let mut store = self.ctx.store().lock().map_err(|_| IpcError::Unavailable {
            reason: "standard propagation store lock poisoned".into(),
        })?;
        let observation = store
            .standard_propagation_observation(unix_now(), storage_policy)
            .map_err(|error| IpcError::Internal { message: error.to_string() })?;
        drop(store);
        snapshot.active = runtime.active();

        let mut policy = StandardPropagationPolicyInfo::default();
        policy.target_cost = runtime_policy.target_cost;
        policy.flexibility = runtime_policy.flexibility;
        policy.peering_cost = runtime_policy.peering_cost;
        policy.transfer_limit_kb = runtime_policy.transfer_limit_kb as u64;
        policy.sync_limit_kb = runtime_policy.sync_limit_kb as u64;
        policy.queue_max_count = runtime_policy.queue_max_count as u64;
        policy.queue_max_bytes = runtime_policy.queue_max_bytes as u64;
        policy.expiry_secs = u64::try_from(runtime_policy.expiry_secs).map_err(|_| {
            IpcError::Internal { message: "negative standard propagation expiry".into() }
        })?;
        policy.throttle_secs = u64::try_from(runtime_policy.throttle_secs).map_err(|_| {
            IpcError::Internal { message: "negative standard propagation throttle".into() }
        })?;
        policy.max_offer_links = u32::try_from(runtime_policy.max_offer_links).unwrap_or(u32::MAX);
        if snapshot.registered {
            snapshot.policy = Some(policy);
        }
        snapshot.observed_at = Some(observation.observed_at);
        snapshot.queue.queued_count = observation.queue.queued_count as u64;
        snapshot.queue.queued_bytes = observation.queue.queued_bytes as u64;
        snapshot.queue.acknowledged_count = observation.queue.acknowledged_count as u64;
        snapshot.queue.expired_count = observation.queue.expired_count as u64;
        snapshot.queue.terminal_count =
            snapshot.queue.acknowledged_count.saturating_add(snapshot.queue.expired_count);
        snapshot.selection_readiness = standard_propagation_selection_readiness(
            snapshot.active,
            observation.selection.as_ref(),
            &observation.peers,
        );
        snapshot.selection = observation.selection.map(|selection| {
            let mut info = StandardPropagationSelectionInfo::default();
            info.peer_hash = selection.peer.map(hex::encode);
            info.mode = selection.mode;
            info.selected_at = selection.selected_at;
            info
        });
        if let Some(sync) = self.ctx.standard_propagation_sync() {
            let sync_policy = sync.policy();
            let sync_telemetry = sync.telemetry();
            snapshot.automatic_sync_enabled = Some(sync_policy.automatic);
            snapshot.automatic_sync_cooldown_secs = Some(sync_policy.cooldown.as_secs());
            snapshot.sync_deadline_secs = Some(sync_policy.deadline.as_secs());
            snapshot.trigger_capabilities = [
                (
                    StandardPropagationTriggerSource::InitialConnection,
                    StandardPropagationPlatformCapability::AutomaticForeground,
                    StandardPropagationOpportunityState::Available,
                ),
                (
                    StandardPropagationTriggerSource::Reconnect,
                    StandardPropagationPlatformCapability::AutomaticForeground,
                    StandardPropagationOpportunityState::Available,
                ),
                (
                    StandardPropagationTriggerSource::ForegroundOpportunity,
                    StandardPropagationPlatformCapability::AutomaticForeground,
                    StandardPropagationOpportunityState::Available,
                ),
                (
                    StandardPropagationTriggerSource::GrantedBackgroundOpportunity,
                    StandardPropagationPlatformCapability::AutomaticBackground,
                    if sync_policy.automatic {
                        StandardPropagationOpportunityState::Available
                    } else {
                        StandardPropagationOpportunityState::Denied
                    },
                ),
                (
                    StandardPropagationTriggerSource::Manual,
                    StandardPropagationPlatformCapability::Manual,
                    StandardPropagationOpportunityState::Available,
                ),
            ]
            .into_iter()
            .map(|(source, platform_capability, opportunity)| {
                let mut info = StandardPropagationTriggerCapabilityInfo::default();
                info.source = source;
                info.platform_capability = platform_capability;
                info.opportunity = opportunity;
                info
            })
            .collect();
            snapshot.active_sync = sync_telemetry.active.map(|active| {
                let mut info = StandardPropagationActiveSyncInfo::default();
                info.trigger = standard_propagation_trigger_source(active.trigger);
                info.started_at = active.started_at;
                info
            });
            snapshot.last_synchronization = sync_telemetry.last_completed.map(|completed| {
                let mut info = StandardPropagationLastSynchronizationInfo::default();
                info.trigger = standard_propagation_trigger_source(completed.trigger);
                info.started_at = completed.started_at;
                info.finished_at = completed.finished_at;
                info.outcome = standard_propagation_terminal_outcome(completed.outcome);
                info.new_messages = u32::try_from(completed.new_messages).unwrap_or(u32::MAX);
                info
            });
            snapshot.cooldown_remaining_secs =
                Some(sync_telemetry.cooldown_remaining.as_secs().saturating_add(u64::from(
                    sync_telemetry.cooldown_remaining.subsec_nanos() > 0,
                )));
            snapshot.sync_readiness = if snapshot.active_sync.is_some() {
                StandardPropagationSyncReadiness::InFlight
            } else if snapshot.cooldown_remaining_secs.unwrap_or(0) > 0 {
                StandardPropagationSyncReadiness::CoolingDown
            } else if snapshot.selection_readiness == StandardPropagationSelectionReadiness::Ready {
                StandardPropagationSyncReadiness::Ready
            } else {
                StandardPropagationSyncReadiness::Unavailable
            };
        }
        snapshot.peers = observation
            .peers
            .into_iter()
            .map(|peer| {
                let mut info = StandardPropagationPeerObservation::default();
                info.peer_hash = hex::encode(peer.identity_hash);
                info.propagation_destination_hash = peer.propagation_destination.map(hex::encode);
                info.configured = peer.configured;
                info.enabled = peer.enabled;
                info.first_seen_at = peer.first_seen_at;
                info.last_seen_at = peer.last_seen_at;
                info.retry_at = peer.retry_at;
                info.backoff_count = peer.backoff_count as u64;
                info.offered_count = peer.offered_count as u64;
                info.wanted_count = peer.wanted_count as u64;
                info.accepted_count = peer.accepted_count as u64;
                info.accepted_bytes = peer.accepted_bytes as u64;
                info.failure_count = peer.failure_count as u64;
                info.transfer_limit_kb = peer.transfer_limit_kb.map(|value| value as u64);
                info.sync_limit_kb = peer.sync_limit_kb.map(|value| value as u64);
                info.stamp_cost = peer.stamp_cost;
                info.stamp_flexibility = peer.stamp_flexibility;
                info.peering_cost = peer.peering_cost;
                info
            })
            .collect();
        snapshot.attempts = observation
            .attempts
            .into_iter()
            .map(|attempt| {
                let state = standard_propagation_state(&attempt.state);
                let mut info = StandardPropagationAttemptObservation::default();
                info.attempt_id = hex::encode(attempt.attempt_id);
                info.correlation_id = hex::encode(attempt.correlation_id);
                info.peer_hash = attempt.peer.map(hex::encode);
                info.direction = standard_propagation_direction(&attempt.direction);
                info.stage = standard_propagation_stage(&attempt.stage);
                info.state = state;
                info.outcome = standard_propagation_outcome(state, attempt.failure_code.as_deref());
                info.started_at = attempt.started_at;
                info.updated_at = attempt.updated_at;
                info.deadline_at = attempt.deadline_at;
                info.offered_count = attempt.offered_count as u64;
                info.wanted_count = attempt.wanted_count as u64;
                info.accepted_count = attempt.accepted_count as u64;
                info.accepted_bytes = attempt.accepted_bytes as u64;
                info.failure_code = attempt.failure_code;
                info
            })
            .collect();
        snapshot.checkpoints = observation
            .checkpoints
            .into_iter()
            .map(|checkpoint| {
                let mut info = StandardPropagationCheckpointObservation::default();
                info.peer_hash = hex::encode(checkpoint.peer);
                info.direction = standard_propagation_direction(&checkpoint.direction);
                info.completed_stage = standard_propagation_stage(&checkpoint.completed_stage);
                info.item_count = checkpoint.item_count as u64;
                info.byte_count = checkpoint.byte_count as u64;
                info.last_attempt_id = checkpoint.last_attempt.map(hex::encode);
                info.updated_at = checkpoint.updated_at;
                info
            })
            .collect();
        snapshot.failures = observation
            .failures
            .into_iter()
            .map(|failure| {
                let mut info = StandardPropagationFailureObservation::default();
                info.code = failure.code;
                info.occurred_at = failure.occurred_at;
                info.peer_hash = failure.peer.map(hex::encode);
                info.attempt_id = failure.attempt_id.map(hex::encode);
                info
            })
            .collect();
        snapshot.peers_truncated = observation.peers_truncated;
        snapshot.attempts_truncated = observation.attempts_truncated;
        snapshot.checkpoints_truncated = observation.checkpoints_truncated;
        snapshot.failures_truncated = observation.failures_truncated;
        Ok(snapshot)
    }

    async fn propagation_snapshot(
        &self,
        query: styrene_ipc::types::PropagationQuery,
    ) -> Result<PropagationSnapshot, IpcError> {
        self.require(Capability::RPC_STATUS)?;
        let service = self.ctx.propagation();
        let after = query
            .cursor
            .as_deref()
            .map(|cursor| {
                let (timestamp, id) = cursor
                    .split_once(':')
                    .filter(|(_, id)| !id.is_empty())
                    .ok_or_else(|| IpcError::invalid_request("invalid propagation cursor"))?;
                let timestamp = timestamp
                    .parse::<i64>()
                    .map_err(|_| IpcError::invalid_request("invalid propagation cursor"))?;
                Ok::<_, IpcError>((timestamp, id))
            })
            .transpose()?;
        let limit = usize::try_from(query.limit.clamp(1, 200)).unwrap_or(200);
        let (count, size) = service.stats().map_err(internal)?;
        let mut snapshot = PropagationSnapshot::default();
        snapshot.enabled = service.is_enabled();
        snapshot.queue_count = u32::try_from(count).unwrap_or(u32::MAX);
        snapshot.queue_size_bytes = size;
        snapshot.expiry_secs = service.expiry_secs();
        let mut queue = service.inventory(limit.saturating_add(1), after).map_err(internal)?;
        if queue.len() > limit {
            queue.truncate(limit);
            snapshot.next_cursor =
                queue.last().map(|entry| format!("{}:{}", entry.received_at, entry.id));
        }
        snapshot.queue = queue;
        // Peer synchronization and configured capacity have no authoritative
        // daemon contracts yet; support flags keep that absence explicit.
        snapshot.peer_state_supported = false;
        snapshot.sync_state_supported = false;
        Ok(snapshot)
    }

    async fn query_config(&self) -> Result<ConfigSnapshot, IpcError> {
        self.require(Capability::RPC_STATUS)?;
        let mut snapshot = ConfigSnapshot::default();
        let config_svc = self.ctx.config();
        snapshot
            .values
            .insert("role".into(), serde_json::json!(config_svc.node_role().to_string()));
        if let Some(path) = config_svc.config_path() {
            snapshot
                .values
                .insert("config_path".into(), serde_json::json!(path.display().to_string()));
        }
        let interfaces = config_svc.interfaces();
        if !interfaces.is_empty() {
            snapshot.values.insert("interface_count".into(), serde_json::json!(interfaces.len()));
        }
        Ok(snapshot)
    }

    async fn query_devices(&self, _styrene_only: bool) -> Result<Vec<DeviceInfo>, IpcError> {
        self.require(Capability::RPC_STATUS)?;
        Ok(self.ctx.discovery().devices())
    }

    async fn query_path_table(&self) -> Result<Vec<PathInfo>, IpcError> {
        self.require(Capability::RPC_STATUS)?;
        let entries = self.ctx.transport().path_snapshots().await;
        Ok(entries
            .into_iter()
            .map(|snapshot| {
                let mut info = PathInfo::default();
                info.destination_hash = hex::encode(snapshot.destination.as_slice());
                info.hops = Some(snapshot.hops as u32);
                info.next_hop = Some(hex::encode(snapshot.received_from.as_slice()));
                info.interface = Some(hex::encode(snapshot.iface.as_slice()));
                info.expires = path_expiry(snapshot);
                info.observation = path_observation(snapshot);
                info
            })
            .collect())
    }

    async fn query_path_info(&self, dest_hash: &str) -> Result<PathInfo, IpcError> {
        self.require(Capability::RPC_STATUS)?;
        let dest_bytes: [u8; 16] = hex::decode(dest_hash)
            .map_err(|e| IpcError::invalid_request(format!("invalid hash: {e}")))?
            .try_into()
            .map_err(|_| IpcError::invalid_request("hash must be 16 bytes"))?;
        let dest = rns_core::hash::AddressHash::new(dest_bytes);

        let path = self.ctx.transport().query_path_snapshot(&dest).await;
        let mut info = PathInfo::default();
        info.destination_hash = dest_hash.to_string();
        if let Some(snapshot) = path {
            info.hops = Some(snapshot.hops as u32);
            info.next_hop = Some(hex::encode(snapshot.received_from.as_slice()));
            info.interface = Some(hex::encode(snapshot.iface.as_slice()));
            info.expires = path_expiry(snapshot);
            info.observation = path_observation(snapshot);
        }
        Ok(info)
    }

    async fn query_auto_reply(&self) -> Result<AutoReplyConfig, IpcError> {
        self.require(Capability::RPC_STATUS)?;
        let config = self.ctx.auto_reply().config();
        let mut ar = AutoReplyConfig::default();
        ar.mode = match config.mode {
            AutoReplyMode::Disabled => "disabled".into(),
            AutoReplyMode::All => "all".into(),
            AutoReplyMode::FirstOnly => "first_only".into(),
            AutoReplyMode::Echo => "echo".into(),
        };
        ar.message = if config.message.is_empty() { None } else { Some(config.message) };
        ar.cooldown_secs = Some(config.cooldown.as_secs());
        Ok(ar)
    }

    async fn set_auto_reply(
        &self,
        mode: &str,
        message: Option<&str>,
        cooldown_secs: Option<u64>,
    ) -> Result<bool, IpcError> {
        self.require(Capability::RPC_CONFIG_UPDATE)?;
        let auto_reply_mode = match mode {
            "disabled" | "off" => AutoReplyMode::Disabled,
            "all" => AutoReplyMode::All,
            "first_only" | "first" => AutoReplyMode::FirstOnly,
            "echo" => AutoReplyMode::Echo,
            _ => {
                return Err(IpcError::InvalidRequest {
                    message: format!("unknown auto-reply mode: {mode}"),
                });
            }
        };
        let config = crate::services::auto_reply::AutoReplyConfig {
            mode: auto_reply_mode,
            message: message.unwrap_or_default().to_string(),
            cooldown: std::time::Duration::from_secs(cooldown_secs.unwrap_or(300)),
        };
        self.ctx.config().set_auto_reply((&config).into()).map_err(internal)?;
        self.ctx.auto_reply().set_config(config);
        Ok(true)
    }

    async fn save_config(&self, config: ConfigSnapshot) -> Result<bool, IpcError> {
        self.require(Capability::RPC_CONFIG_UPDATE)?;
        self.ctx.config().apply_snapshot(&config).map_err(internal)?;
        Ok(true)
    }

    async fn block_peer(&self, identity_hash: &str) -> Result<bool, IpcError> {
        self.require(Capability::POLICY_UPDATE)?;
        // Prevent self-block: blocking the daemon's own identity would lock out local IPC.
        if self.caller_identity.starts_with(identity_hash)
            || identity_hash.starts_with(&self.caller_identity)
        {
            return Err(IpcError::invalid_request("cannot block the daemon's own identity"));
        }
        self.ctx
            .policy()
            .block(identity_hash, self.ctx.store())
            .map_err(|e| IpcError::Internal { message: format!("block_peer failed: {e}") })
    }

    async fn unblock_peer(&self, identity_hash: &str) -> Result<bool, IpcError> {
        self.require(Capability::POLICY_UPDATE)?;
        self.ctx
            .policy()
            .unblock(identity_hash, self.ctx.store())
            .map_err(|e| IpcError::Internal { message: format!("unblock_peer failed: {e}") })
    }

    async fn blocked_peers(&self) -> Result<Vec<String>, IpcError> {
        self.require(Capability::RPC_STATUS)?;
        let store = self.ctx.store();
        let store = store.lock().unwrap();
        store
            .blocked_peers()
            .map_err(|e| IpcError::Internal { message: format!("blocked_peers failed: {e}") })
    }

    async fn list_interfaces(&self) -> Result<Vec<InterfaceDetail>, IpcError> {
        self.require(Capability::RPC_STATUS)?;
        let snapshots = self.ctx.transport().interface_snapshots().await;
        let connection_generation = self.session_generation.as_ref().map_or_else(
            || snapshots.iter().map(|interface| interface.generation).max().unwrap_or(1),
            |generation| generation.observe(&snapshots),
        );
        let observed_at = unix_now();
        Ok(snapshots
            .into_iter()
            .map(|snapshot| {
                use rns_core::transport::iface::{
                    InterfaceEndpoint, InterfaceKind, InterfaceState,
                };

                let mut d = InterfaceDetail::default();
                d.hash = hex::encode(snapshot.hash.as_slice());
                d.kind = snapshot.kind.as_str().into();
                d.name = format!("{}-{}", d.kind, &d.hash[..8]);
                d.mode = snapshot.mode.as_str().into();
                d.status = snapshot.state.as_str().into();
                d.enabled = snapshot.state != InterfaceState::Closed;
                d.local_endpoint =
                    snapshot.local_endpoint.as_ref().map(|endpoint| match endpoint {
                        InterfaceEndpoint::Socket(address) => address.to_string(),
                        InterfaceEndpoint::Device { path, baud_rate } => {
                            format!("{path}@{baud_rate}")
                        }
                    });
                d.remote_endpoint =
                    snapshot.remote_endpoint.as_ref().map(|endpoint| match endpoint {
                        InterfaceEndpoint::Socket(address) => address.to_string(),
                        InterfaceEndpoint::Device { path, baud_rate } => {
                            format!("{path}@{baud_rate}")
                        }
                    });
                d.parent_hash = snapshot.parent.map(|parent| hex::encode(parent.as_slice()));
                let compatibility_endpoint = if snapshot.kind == InterfaceKind::TcpClient {
                    snapshot.remote_endpoint.as_ref().or(snapshot.local_endpoint.as_ref())
                } else {
                    snapshot.local_endpoint.as_ref().or(snapshot.remote_endpoint.as_ref())
                };
                match compatibility_endpoint {
                    Some(InterfaceEndpoint::Socket(address)) => {
                        d.host = Some(address.ip().to_string());
                        d.port = Some(address.port());
                    }
                    Some(InterfaceEndpoint::Device { path, .. }) => d.host = Some(path.clone()),
                    None => {}
                }
                d.tx_bytes = snapshot.tx_bytes;
                d.rx_bytes = snapshot.rx_bytes;
                d.peers_connected = snapshot.connected_peers;
                d.observation = ObservationMetadata::at(
                    ObservationSource::RuntimeInterfaceRegistry,
                    Some(observed_at),
                    observed_at,
                    INTERFACE_FRESHNESS_THRESHOLD_SECS,
                );
                d.observation.connection_generation = Some(connection_generation);
                d.observation.interface_generation = Some(snapshot.generation);
                d.failure = match snapshot.state {
                    InterfaceState::Retrying => {
                        let mut failure = InterfaceFailureInfo::default();
                        failure.code = InterfaceFailureCode::Retrying;
                        failure.retryable = true;
                        Some(failure)
                    }
                    InterfaceState::Closed => {
                        let mut failure = InterfaceFailureInfo::default();
                        failure.code = InterfaceFailureCode::Closed;
                        Some(failure)
                    }
                    InterfaceState::Unknown => {
                        let mut failure = InterfaceFailureInfo::default();
                        failure.code = InterfaceFailureCode::UnknownState;
                        failure.retryable = true;
                        Some(failure)
                    }
                    _ => None,
                };
                d
            })
            .collect())
    }

    async fn search_peers(&self, query: &str, limit: u32) -> Result<Vec<DeviceInfo>, IpcError> {
        self.require(Capability::RPC_STATUS)?;
        let query_lower = query.to_lowercase();
        let all_nodes = self
            .ctx
            .discovery()
            .node_store()
            .list(None)
            .map_err(|e| IpcError::Internal { message: e.to_string() })?;

        let matched: Vec<DeviceInfo> = all_nodes
            .into_iter()
            .filter(|n| {
                n.identity_hash.starts_with(query)
                    || n.display_name
                        .as_ref()
                        .is_some_and(|name| name.to_lowercase().contains(&query_lower))
            })
            .take(limit as usize)
            .map(|n| {
                let mut d = DeviceInfo::default();
                d.destination_hash = n.identity_hash.clone();
                d.identity_hash = n.identity_hash;
                d.name = n.display_name.unwrap_or_default();
                d.last_announce = Some(n.last_seen);
                d.announce_count = n.announce_count as u32;
                d
            })
            .collect();
        Ok(matched)
    }

    async fn bookmark_peer(&self, identity_hash: &str) -> Result<bool, IpcError> {
        self.require(Capability::RPC_STATUS)?;
        self.ctx.discovery().bookmark_peer(identity_hash).map_err(internal)?;
        Ok(true)
    }

    async fn unbookmark_peer(&self, identity_hash: &str) -> Result<bool, IpcError> {
        self.require(Capability::RPC_STATUS)?;
        self.ctx
            .discovery()
            .node_store()
            .set_bookmarked(identity_hash, false)
            .map_err(|e| IpcError::Internal { message: e.to_string() })?;
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
        self.require(Capability::RPC_STATUS)?;
        self.ctx.fleet().device_status(dest, timeout).await.map_err(internal)
    }

    async fn exec(
        &self,
        dest: &str,
        cmd: &str,
        args: Vec<String>,
        timeout: Option<u64>,
    ) -> Result<ExecResult, IpcError> {
        self.require(Capability::RPC_EXEC)?;
        self.ctx.fleet().exec(dest, cmd, &args, timeout).await.map_err(internal)
    }

    async fn reboot_device(
        &self,
        dest: &str,
        delay: Option<u64>,
        timeout: Option<u64>,
    ) -> Result<RebootResult, IpcError> {
        self.require(Capability::RPC_REBOOT)?;
        self.ctx.fleet().reboot_device(dest, delay, timeout).await.map_err(internal)
    }

    async fn self_update(
        &self,
        _dest: &str,
        _version: Option<&str>,
        _timeout: Option<u64>,
    ) -> Result<SelfUpdateResult, IpcError> {
        self.require(Capability::RPC_SELF_UPDATE)?;
        Err(Self::not_implemented("self_update"))
    }

    async fn remote_inbox(
        &self,
        dest: &str,
        limit: u32,
        timeout: Option<u64>,
    ) -> Result<Vec<ConversationInfo>, IpcError> {
        self.require(Capability::RPC_STATUS)?;
        self.ctx.fleet().remote_inbox(dest, limit, timeout).await.map_err(internal)
    }

    async fn remote_messages(
        &self,
        dest: &str,
        peer_hash: &str,
        limit: u32,
        timeout: Option<u64>,
    ) -> Result<Vec<MessageInfo>, IpcError> {
        self.require(Capability::RPC_STATUS)?;
        self.ctx.fleet().remote_messages(dest, peer_hash, limit, timeout).await.map_err(internal)
    }

    async fn terminal_open(&self, request: TerminalOpenRequest) -> Result<SessionId, IpcError> {
        self.require(Capability::RPC_EXEC)?;
        #[cfg(feature = "terminal")]
        {
            self.ctx
                .terminal()
                .open(request.shell.as_deref(), request.rows, request.cols)
                .map_err(|e| IpcError::Internal { message: e })
        }
        #[cfg(not(feature = "terminal"))]
        {
            let _ = request;
            Err(IpcError::Internal { message: "terminal not available on this platform".into() })
        }
    }

    async fn terminal_input(&self, session_id: &str, data: &[u8]) -> Result<bool, IpcError> {
        self.require(Capability::RPC_EXEC)?;
        #[cfg(feature = "terminal")]
        {
            self.ctx
                .terminal()
                .input(session_id, data)
                .await
                .map(|_| true)
                .map_err(|e| IpcError::Internal { message: e })
        }
        #[cfg(not(feature = "terminal"))]
        {
            let _ = (session_id, data);
            Err(IpcError::Internal { message: "terminal not available on this platform".into() })
        }
    }

    async fn terminal_resize(
        &self,
        session_id: &str,
        rows: u16,
        cols: u16,
    ) -> Result<bool, IpcError> {
        self.require(Capability::RPC_EXEC)?;
        #[cfg(feature = "terminal")]
        {
            self.ctx
                .terminal()
                .resize(session_id, rows, cols)
                .map(|_| true)
                .map_err(|e| IpcError::Internal { message: e })
        }
        #[cfg(not(feature = "terminal"))]
        {
            let _ = (session_id, rows, cols);
            Err(IpcError::Internal { message: "terminal not available on this platform".into() })
        }
    }

    async fn terminal_close(&self, session_id: &str) -> Result<bool, IpcError> {
        self.require(Capability::RPC_EXEC)?;
        #[cfg(feature = "terminal")]
        {
            self.ctx
                .terminal()
                .close(session_id)
                .map(|_| true)
                .map_err(|e| IpcError::Internal { message: e })
        }
        #[cfg(not(feature = "terminal"))]
        {
            let _ = session_id;
            Err(IpcError::Internal { message: "terminal not available on this platform".into() })
        }
    }

    async fn fleet_apply(
        &self,
        dest: &str,
        profile_bytes: Vec<u8>,
        verify: bool,
        timeout: Option<u64>,
    ) -> Result<ConfigApplyResult, IpcError> {
        self.require(Capability::RPC_FLEET_APPLY)?;
        self.ctx.fleet().apply(dest, &profile_bytes, verify, timeout).await.map_err(internal)
    }

    async fn fleet_grant(
        &self,
        identity_hash: &str,
        role: &str,
        label: &str,
        grants: Vec<String>,
    ) -> Result<bool, IpcError> {
        self.require(Capability::RPC_EXEC)?; // Admin-level operation

        let rbac_role = styrene_rbac::Role::from_name(role)
            .ok_or_else(|| IpcError::invalid_request(format!("unknown role: {role}")))?;

        // Prevent privilege escalation: caller cannot grant a role higher than their own.
        let caller_role = self.ctx.policy().resolve_role(&self.caller_identity);
        if rbac_role > caller_role {
            return Err(IpcError::Unavailable {
                reason: format!(
                    "cannot grant role {} (higher than caller's {})",
                    rbac_role.as_str(),
                    caller_role.as_str(),
                ),
            });
        }

        // Prevent granting capabilities the caller doesn't hold.
        for cap in &grants {
            if !self.ctx.policy().has_capability(&self.caller_identity, cap) {
                return Err(IpcError::Unavailable {
                    reason: format!("cannot grant capability {} (caller does not hold it)", cap,),
                });
            }
        }

        let entry = styrene_rbac::RosterEntry::new(identity_hash, rbac_role)
            .with_label(label)
            .with_grants(grants);
        self.ctx
            .policy()
            .grant(entry, self.ctx.store())
            .map_err(|e| IpcError::Internal { message: e })?;
        Ok(true)
    }

    async fn fleet_revoke(&self, identity_hash: &str) -> Result<bool, IpcError> {
        self.require(Capability::RPC_EXEC)?; // Admin-level operation

        // Prevent self-revocation: revoking the daemon's own Admin would lock out local IPC.
        if identity_hash.eq_ignore_ascii_case(&self.caller_identity) {
            return Err(IpcError::invalid_request(
                "cannot revoke the daemon's own role (self-lockout protection)",
            ));
        }

        // Prevent revoking identities with a higher role than the caller.
        let caller_role = self.ctx.policy().resolve_role(&self.caller_identity);
        let target_role = self.ctx.policy().resolve_role(identity_hash);
        if target_role > caller_role {
            return Err(IpcError::Unavailable {
                reason: format!(
                    "cannot revoke {} (role {} is higher than caller's {})",
                    identity_hash,
                    target_role.as_str(),
                    caller_role.as_str(),
                ),
            });
        }

        self.ctx
            .policy()
            .revoke(identity_hash, self.ctx.store())
            .map_err(|e| IpcError::Internal { message: e })
    }
}

#[async_trait]
impl DaemonEvents for DaemonFacade {
    async fn link_snapshot(&self) -> Result<styrene_ipc::types::LinkSnapshot, IpcError> {
        self.require(Capability::RPC_STATUS)?;
        let lifecycle = self.ctx.transport().link_lifecycle_snapshot().await;
        let active =
            lifecycle.active.into_iter().map(crate::workers::link::link_event_from_state).collect();
        let history = lifecycle
            .history
            .into_iter()
            .map(crate::workers::link::link_event_from_state)
            .collect();
        self.ctx.events().reconcile_links(active, history);
        Ok(self.ctx.events().link_snapshot())
    }

    async fn subscribe_messages(
        &self,
        peer_hashes: &[String],
    ) -> Result<broadcast::Receiver<DaemonEvent>, IpcError> {
        self.require(Capability::RPC_STATUS)?;
        Ok(self.ctx.events().subscribe_messages(peer_hashes))
    }

    async fn subscribe_devices(&self) -> Result<broadcast::Receiver<DaemonEvent>, IpcError> {
        self.require(Capability::RPC_STATUS)?;
        Ok(self.ctx.events().subscribe_devices())
    }

    async fn subscribe_links(&self) -> Result<broadcast::Receiver<DaemonEvent>, IpcError> {
        self.require(Capability::RPC_STATUS)?;
        Ok(self.ctx.events().subscribe_links())
    }

    async fn subscribe_routes(&self) -> Result<broadcast::Receiver<DaemonEvent>, IpcError> {
        self.require(Capability::RPC_STATUS)?;
        Ok(self.ctx.events().subscribe_routes())
    }

    async fn subscribe_requests(&self) -> Result<broadcast::Receiver<DaemonEvent>, IpcError> {
        self.require(Capability::RPC_STATUS)?;
        let mut source = self.ctx.transport().subscribe_request_observations();
        let (tx, rx) = broadcast::channel(64);
        tokio::spawn(async move {
            while let Ok(event) = daemon_request_event(source.recv().await) {
                let _ = tx.send(event);
            }
        });
        Ok(rx)
    }

    async fn start_request(
        &self,
        request: styrene_ipc::types::StartRequestInfo,
    ) -> Result<styrene_ipc::types::RequestObservationInfo, IpcError> {
        self.require(Capability::NETWORK_REQUEST)?;
        self.ctx
            .transport()
            .start_request(request)
            .await
            .map_err(|error| IpcError::Transport { message: error.to_string() })
    }

    async fn request_receipt(
        &self,
        request_id: &str,
    ) -> Result<Option<styrene_ipc::types::RequestObservationInfo>, IpcError> {
        self.require(Capability::RPC_STATUS)?;
        self.ctx
            .transport()
            .request_receipt(request_id)
            .await
            .map_err(|error| IpcError::Transport { message: error.to_string() })
    }

    async fn request_receipts(
        &self,
    ) -> Result<Vec<styrene_ipc::types::RequestObservationInfo>, IpcError> {
        self.require(Capability::RPC_STATUS)?;
        self.ctx
            .transport()
            .request_receipts()
            .await
            .map_err(|error| IpcError::Transport { message: error.to_string() })
    }

    async fn cancel_request(
        &self,
        request_id: &str,
    ) -> Result<styrene_ipc::types::RequestObservationInfo, IpcError> {
        self.require(Capability::NETWORK_REQUEST_CANCEL)?;
        self.ctx
            .transport()
            .cancel_request(request_id)
            .await
            .map_err(|error| IpcError::Transport { message: error.to_string() })
    }

    async fn resource_transfers(
        &self,
    ) -> Result<Vec<styrene_ipc::types::ResourceTransferInfo>, IpcError> {
        self.require(Capability::RPC_STATUS)?;
        Ok(self.ctx.events().resource_transfers())
    }

    async fn cancel_resource(&self, resource_hash: &str) -> Result<bool, IpcError> {
        self.require(Capability::NETWORK_RESOURCE_CANCEL)?;
        let bytes = hex::decode(resource_hash)
            .map_err(|_| IpcError::invalid_request("resource hash must be hexadecimal"))?;
        let hash: [u8; rns_core::hash::HASH_SIZE] = bytes
            .try_into()
            .map_err(|_| IpcError::invalid_request("resource hash has invalid length"))?;
        self.ctx
            .transport()
            .cancel_resource(rns_core::hash::Hash::new(hash))
            .await
            .map_err(|error| IpcError::Transport { message: error.to_string() })
    }

    async fn subscribe_resources(&self) -> Result<broadcast::Receiver<DaemonEvent>, IpcError> {
        self.require(Capability::RPC_STATUS)?;
        Ok(self.ctx.events().subscribe_resources())
    }

    async fn subscribe_network_operations(
        &self,
    ) -> Result<broadcast::Receiver<DaemonEvent>, IpcError> {
        self.require(Capability::RPC_STATUS)?;
        Ok(self.ctx.events().subscribe_network_operations())
    }

    async fn start_network_operation(
        &self,
        request: styrene_ipc::types::StartNetworkOperationInfo,
    ) -> Result<styrene_ipc::types::NetworkOperationInfo, IpcError> {
        let capability = Self::network_operation_capability(request.kind)?;
        if let Err(error) = self.require(capability) {
            return self
                .ctx
                .network_operations()
                .denied(request, error.to_string())
                .map_err(IpcError::invalid_request);
        }
        self.ctx.network_operations().start(request).map_err(IpcError::invalid_request)
    }

    async fn network_operation(
        &self,
        operation_id: &str,
    ) -> Result<Option<styrene_ipc::types::NetworkOperationInfo>, IpcError> {
        self.require(Capability::RPC_STATUS)?;
        Ok(self.ctx.network_operations().get(operation_id))
    }

    async fn network_operations(
        &self,
    ) -> Result<Vec<styrene_ipc::types::NetworkOperationInfo>, IpcError> {
        self.require(Capability::RPC_STATUS)?;
        Ok(self.ctx.network_operations().list())
    }

    async fn cancel_network_operation(
        &self,
        operation_id: &str,
    ) -> Result<styrene_ipc::types::NetworkOperationInfo, IpcError> {
        let operation = self
            .ctx
            .network_operations()
            .get(operation_id)
            .ok_or_else(|| IpcError::not_found("network operation", operation_id))?;
        self.require(Self::network_operation_capability(operation.kind)?)?;
        self.ctx.network_operations().cancel(operation_id).await.map_err(IpcError::invalid_request)
    }
}

fn daemon_request_event(
    result: Result<
        crate::transport::mesh_transport::RequestLifecycleEvent,
        broadcast::error::RecvError,
    >,
) -> Result<DaemonEvent, ()> {
    match result {
        Ok(crate::transport::mesh_transport::RequestLifecycleEvent::Observation(event)) => {
            Ok(DaemonEvent::Request { event: *event })
        }
        Ok(crate::transport::mesh_transport::RequestLifecycleEvent::ReconcileRequired {
            dropped,
        })
        | Err(broadcast::error::RecvError::Lagged(dropped)) => {
            Ok(DaemonEvent::RequestReconcileRequired { dropped })
        }
        Err(broadcast::error::RecvError::Closed) => Err(()),
    }
}

#[async_trait]
impl DaemonTunnel for DaemonFacade {
    async fn list_tunnels(&self) -> Result<Vec<TunnelInfo>, IpcError> {
        self.require(Capability::TUNNEL_STATUS)?;
        let peers = self.ctx.tunnel().active_peers();
        let mut tunnels = Vec::with_capacity(peers.len());
        for peer in peers {
            if let Some(state) = self.ctx.tunnel().get_peer_state(&peer) {
                let mut info = TunnelInfo::default();
                info.peer_hash = peer;
                info.backend = String::from("wireguard");
                info.state = String::from("established");
                info.remote_endpoint = Some(state.endpoint.clone());
                info.established_at = Some(state.established_at);
                tunnels.push(info);
            }
        }
        Ok(tunnels)
    }

    async fn tunnel_status(&self, peer_hash: &str) -> Result<TunnelInfo, IpcError> {
        self.require(Capability::TUNNEL_STATUS)?;
        if let Some(state) = self.ctx.tunnel().get_peer_state(peer_hash) {
            let mut info = TunnelInfo::default();
            info.peer_hash = peer_hash.to_string();
            info.backend = String::from("wireguard");
            info.state = String::from("established");
            info.remote_endpoint = Some(state.endpoint.clone());
            info.established_at = Some(state.established_at);
            return Ok(info);
        }
        if let Some(operation) = self.ctx.tunnel().latest_operation(peer_hash) {
            let mut info = TunnelInfo::default();
            info.peer_hash = peer_hash.to_string();
            info.backend = String::from("wireguard");
            info.state = operation.state;
            return Ok(info);
        }
        Err(IpcError::not_found("tunnel", peer_hash))
    }

    async fn tunnel_rekey(&self, peer_hash: &str) -> Result<bool, IpcError> {
        self.require(Capability::TUNNEL_ESTABLISH)?;
        self.ctx
            .tunnel()
            .teardown_tunnel(peer_hash)
            .await
            .map_err(|e| IpcError::Internal { message: e })?;
        // If re-establishment fails, the tunnel is now down. The caller gets
        // an error and must manually re-establish.
        if let Err(e) = self.ctx.tunnel().initiate_tunnel(peer_hash).await {
            return Err(IpcError::Internal {
                message: format!("tunnel torn down but re-establish failed: {e}"),
            });
        }
        Ok(true)
    }

    async fn tunnel_teardown(&self, peer_hash: &str) -> Result<bool, IpcError> {
        self.require(Capability::TUNNEL_TEARDOWN)?;
        self.ctx
            .tunnel()
            .teardown_tunnel(peer_hash)
            .await
            .map_err(|e| IpcError::Internal { message: e })?;
        Ok(true)
    }

    async fn list_tunnel_sas(&self, _peer_hash: &str) -> Result<Vec<TunnelSaInfo>, IpcError> {
        self.require(Capability::TUNNEL_STATUS)?;
        Ok(Vec::new())
    }

    async fn tunnel_establish(&self, peer_hash: &str) -> Result<String, IpcError> {
        crate::daemon_diagnostic!("[tunnel-ipc] dispatch entered peer={peer_hash}");
        self.require(Capability::TUNNEL_ESTABLISH)?;
        crate::daemon_diagnostic!("[tunnel-ipc] capability check completed peer={peer_hash}");
        let operation_id = self
            .ctx
            .tunnel_arc()
            .queue_tunnel(peer_hash)
            .map_err(|e| IpcError::Internal { message: e })?;
        crate::daemon_diagnostic!(
            "[tunnel-ipc] queue_tunnel returned peer={} operation={}",
            peer_hash,
            operation_id
        );
        Ok(operation_id)
    }

    async fn tunnel_operation(&self, peer_hash: &str) -> Result<TunnelOperationInfo, IpcError> {
        self.require(Capability::TUNNEL_STATUS)?;
        self.ctx
            .tunnel()
            .latest_operation(peer_hash)
            .ok_or_else(|| IpcError::not_found("tunnel operation", peer_hash))
    }
}

#[async_trait]
impl DaemonPages for DaemonFacade {
    async fn browse_page(
        &self,
        host: &str,
        path: &str,
        timeout: Option<u64>,
    ) -> Result<PageContent, IpcError> {
        self.browse_page_for_owner(0, host, path, timeout).await
    }

    async fn browse_page_for_owner(
        &self,
        owner: u64,
        host: &str,
        path: &str,
        timeout: Option<u64>,
    ) -> Result<PageContent, IpcError> {
        self.require(Capability::PAGE_BROWSE)?;

        // Local page serving (host is empty, "local", or our own identity hash)
        let local =
            host.is_empty() || host == "local" || host == self.ctx.identity().identity_hash();
        let address =
            styrene_ipc::PageAddress::from_request_parts(if local { "" } else { host }, path)
                .map_err(|error| IpcError::invalid_request(error.to_string()))?;
        let (validated_host, validated_path) = address.parts();
        let mut request = PageNavigationRequest::default();
        request.target = Some(if validated_host.is_empty() {
            validated_path.to_string()
        } else {
            format!("{validated_host}:{validated_path}")
        });
        request.timeout_secs = timeout;
        self.navigate_page_for_owner(owner, request).await
    }

    async fn navigate_page(&self, request: PageNavigationRequest) -> Result<PageContent, IpcError> {
        self.navigate_page_for_owner(0, request).await
    }

    async fn navigate_page_for_owner(
        &self,
        owner: u64,
        request: PageNavigationRequest,
    ) -> Result<PageContent, IpcError> {
        self.require(Capability::PAGE_BROWSE)?;
        self.ctx
            .native_browse()
            .navigate_for_owner(owner, request, self.ctx.identity().identity_hash(), |path| {
                self.ctx.pages().handle_request(path)
            })
            .await
            .map_err(IpcError::invalid_request)
    }

    async fn close_page_session(&self, session_id: &str) -> Result<PageNavigationInfo, IpcError> {
        self.close_page_session_for_owner(0, session_id).await
    }

    async fn close_page_session_for_owner(
        &self,
        owner: u64,
        session_id: &str,
    ) -> Result<PageNavigationInfo, IpcError> {
        self.require(Capability::PAGE_BROWSE)?;
        self.ctx
            .native_browse()
            .close_session_for_owner(owner, session_id)
            .await
            .map_err(IpcError::invalid_request)
    }

    async fn start_file_download(
        &self,
        request: FileDownloadRequest,
    ) -> Result<FileDownloadInfo, IpcError> {
        self.start_file_download_for_owner(0, request).await
    }

    async fn start_file_download_for_owner(
        &self,
        owner: u64,
        request: FileDownloadRequest,
    ) -> Result<FileDownloadInfo, IpcError> {
        self.require(Capability::PAGE_BROWSE)?;
        if let Some(checksum) = request.expected_sha256.as_deref()
            && (checksum.len() != 64 || !checksum.as_bytes().iter().all(u8::is_ascii_hexdigit))
        {
            return Err(IpcError::invalid_request(
                "expected SHA-256 must be 64 hexadecimal characters",
            ));
        }
        self.ctx
            .native_browse()
            .start_download_for_owner(owner, request)
            .await
            .map_err(IpcError::invalid_request)
    }

    async fn file_download(&self, download_id: &str) -> Result<FileDownloadInfo, IpcError> {
        self.file_download_for_owner(0, download_id).await
    }

    async fn file_download_for_owner(
        &self,
        owner: u64,
        download_id: &str,
    ) -> Result<FileDownloadInfo, IpcError> {
        self.require(Capability::PAGE_BROWSE)?;
        self.ctx
            .native_browse()
            .download_for_owner(owner, download_id)
            .await
            .ok_or_else(|| IpcError::not_found("file download", download_id))
    }

    async fn cancel_file_download(&self, download_id: &str) -> Result<FileDownloadInfo, IpcError> {
        self.cancel_file_download_for_owner(0, download_id).await
    }

    async fn cancel_file_download_for_owner(
        &self,
        owner: u64,
        download_id: &str,
    ) -> Result<FileDownloadInfo, IpcError> {
        self.require(Capability::PAGE_BROWSE)?;
        self.ctx
            .native_browse()
            .cancel_download_for_owner(owner, download_id)
            .await
            .ok_or_else(|| IpcError::not_found("file download", download_id))
    }

    async fn save_file_download(
        &self,
        download_id: &str,
        destination: &str,
    ) -> Result<FileDownloadInfo, IpcError> {
        self.save_file_download_for_owner(0, download_id, destination).await
    }

    async fn save_file_download_for_owner(
        &self,
        owner: u64,
        download_id: &str,
        destination: &str,
    ) -> Result<FileDownloadInfo, IpcError> {
        self.require(Capability::PAGE_BROWSE)?;
        if destination.trim().is_empty() {
            return Err(IpcError::invalid_request("save destination is empty"));
        }
        self.ctx
            .native_browse()
            .save_download_for_owner(owner, download_id, std::path::Path::new(destination))
            .await
            .map_err(IpcError::invalid_request)
    }

    async fn cleanup_page_owner(&self, owner: u64) -> Result<(), IpcError> {
        self.ctx
            .native_browse()
            .cleanup_owner(owner)
            .await
            .map_err(|message| IpcError::Internal { message })
    }

    async fn list_pages(
        &self,
        host: &str,
        _timeout: Option<u64>,
    ) -> Result<Vec<PageInfo>, IpcError> {
        self.require(Capability::PAGE_BROWSE)?;

        if host.is_empty() || host == "local" || host == self.ctx.identity().identity_hash() {
            let pages = self.ctx.pages().native_inventory();
            return Ok(pages
                .into_iter()
                .map(|(entry, handler_active)| {
                    let mut info = PageInfo::default();
                    info.kind = if entry.request_path.starts_with("/file/") {
                        "file".into()
                    } else {
                        "page".into()
                    };
                    info.path = entry.request_path;
                    info.host_hash = self.ctx.identity().identity_hash().to_string();
                    info.dynamic = entry.dynamic;
                    info.restricted = entry.restricted;
                    info.handler_active = handler_active;
                    info
                })
                .collect());
        }

        styrene_ipc::NomadNetHost::parse(host)
            .map_err(|error| IpcError::invalid_request(error.to_string()))?;

        Err(IpcError::not_implemented("remote page listing"))
    }

    async fn page_hosts(&self) -> Result<Vec<DeviceInfo>, IpcError> {
        self.require(Capability::PAGE_BROWSE)?;
        Ok(self
            .ctx
            .discovery()
            .devices()
            .into_iter()
            .filter(|device| {
                device.discovered_capabilities.contains(&DiscoveredCapability::NativeNomadNetHost)
            })
            .collect())
    }
}

// DaemonFacade automatically implements `Daemon` because it implements
// all seven sub-traits.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::messages::MessagesStore;
    use crate::transport::mesh_transport::MeshTransport;
    use crate::transport::null_transport::NullTransport;
    use std::sync::Mutex;
    use styrene_ipc::traits::Daemon;

    #[test]
    fn session_generation_advances_and_prunes_removed_interfaces() {
        let generation = SessionGeneration::new(1);
        assert_eq!(generation.observe_generations([("first".into(), 1)]), 1);

        assert_eq!(generation.observe_generations([]), 2);
        assert!(generation.state.lock().unwrap().interfaces.is_empty());

        assert_eq!(generation.observe_generations([("second".into(), 1)]), 3);
        let state = generation.state.lock().unwrap();
        assert_eq!(state.interfaces.len(), 1);
        assert_eq!(state.interfaces.get("second"), Some(&1));
    }

    #[test]
    fn session_generation_treats_new_hash_as_replacement() {
        let generation = SessionGeneration::new(1);
        generation.observe_generations([("first".into(), 1)]);

        assert_eq!(generation.observe_generations([("replacement".into(), 1)]), 2);
        let state = generation.state.lock().unwrap();
        assert!(!state.interfaces.contains_key("first"));
        assert_eq!(state.interfaces.get("replacement"), Some(&1));
    }

    #[test]
    fn stale_page_cursor_maps_to_typed_conflict() {
        assert_eq!(
            page_error(crate::storage::messages::PageError::CursorStale),
            IpcError::Conflict { message: "cursor_stale".into() }
        );
    }

    fn make_facade() -> DaemonFacade {
        let transport: Arc<dyn MeshTransport> = Arc::new(NullTransport::new());
        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let ctx = Arc::new(AppContext::new(transport, "test-identity".into(), store));
        DaemonFacade::new(ctx, "test-caller".into())
    }

    fn make_facade_for_role(
        role: styrene_rbac::Role,
        transport: Arc<dyn MeshTransport>,
    ) -> DaemonFacade {
        let caller = "aaaaaaaa11111111bbbbbbbb22222222";
        let mut policy = styrene_rbac::RbacPolicy::default();
        policy.add_entry(styrene_rbac::RosterEntry::new(caller, role));
        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let node_store = Arc::new(styrene_services::node_store::NodeStore::in_memory().unwrap());
        let ctx = Arc::new(AppContext::with_policy(
            transport,
            "test-identity".into(),
            store,
            node_store,
            crate::services::PolicyService::new(policy),
        ));
        DaemonFacade::new(ctx, caller.into())
    }

    #[test]
    fn facade_implements_daemon_trait() {
        let facade = make_facade();
        // Verify it can be used as Arc<dyn Daemon>
        let _: Arc<dyn Daemon> = Arc::new(facade);
    }

    #[tokio::test]
    async fn offline_local_manage_commits_before_event_and_noops_emit_nothing() {
        let facade =
            make_facade_for_role(styrene_rbac::Role::Admin, Arc::new(NullTransport::new()));
        let peer = "12".repeat(16);
        let record = MessageRecord {
            id: "offline-mark".into(),
            source: peer.clone(),
            destination: "34".repeat(16),
            title: String::new(),
            content: "unread".into(),
            timestamp: 1,
            direction: "in".into(),
            fields: None,
            receipt_status: None,
            read: false,
        };
        assert!(facade.ctx.messaging().accept_inbound_record(&record).unwrap());
        let mut events = facade.ctx.events().subscribe_daemon_events();

        let outcome = facade.mark_read_outcome(&peer).await.unwrap();
        assert_eq!(outcome.disposition, MessagingDisposition::Applied);
        assert_eq!(outcome.conversation.as_ref().unwrap().unread_count, 0);
        assert_eq!(facade.ctx.messaging().list_conversations(false).unwrap()[0].unread_count, 0);
        assert!(matches!(
            events.recv().await.unwrap(),
            DaemonEvent::MessagingOperation { outcome }
                if outcome.disposition == MessagingDisposition::Applied
        ));

        assert_eq!(
            facade.mark_read_outcome(&peer).await.unwrap().disposition,
            MessagingDisposition::Unchanged
        );
        assert!(matches!(events.try_recv(), Err(broadcast::error::TryRecvError::Empty)));
        assert!(facade.mark_read_outcome("invalid").await.is_err());
        assert!(matches!(events.try_recv(), Err(broadcast::error::TryRecvError::Empty)));
    }

    #[tokio::test]
    async fn complete_message_projection_uses_backend_retry_eligibility_not_status_text() {
        let facade =
            make_facade_for_role(styrene_rbac::Role::Admin, Arc::new(NullTransport::new()));
        let record = MessageRecord {
            id: "inbound-retry-projection".into(),
            source: "12".repeat(16),
            destination: "34".repeat(16),
            title: String::new(),
            content: "not retryable".into(),
            timestamp: 1,
            direction: "in".into(),
            fields: None,
            receipt_status: Some("failed: display-only text".into()),
            read: false,
        };
        assert!(facade.ctx.messaging().accept_inbound_record(&record).unwrap());

        let projected = facade.query_message(&record.id).await.unwrap().unwrap();

        assert!(projected.projection_complete);
        assert_eq!(projected.status, "failed: display-only text");
        assert_eq!(projected.retry_eligible, Some(false));
        assert_eq!(
            projected.retry_ineligibility_reason,
            Some(styrene_ipc::types::MessageRetryIneligibilityReason::Inbound)
        );
    }

    #[tokio::test]
    async fn drafts_round_trip_idempotently_and_require_messaging_manage() {
        let facade =
            make_facade_for_role(styrene_rbac::Role::Admin, Arc::new(NullTransport::new()));
        let peer = "56".repeat(16);
        let first = facade.set_draft(&peer, "retained").await.unwrap();
        let replaced = facade.set_draft(&peer, "retained").await.unwrap();
        assert_eq!(first.content, replaced.content);
        assert_eq!(facade.draft(&peer).await.unwrap().unwrap().content, "retained");
        assert_eq!(facade.clear_draft(&peer).await.unwrap(), MessagingDisposition::Applied);
        assert_eq!(facade.clear_draft(&peer).await.unwrap(), MessagingDisposition::Unchanged);

        let denied =
            make_facade_for_role(styrene_rbac::Role::Monitor, Arc::new(NullTransport::new()));
        assert!(matches!(denied.set_draft(&peer, "secret").await, Err(IpcError::Denied { .. })));
    }

    #[tokio::test]
    async fn poisoned_post_commit_projection_returns_typed_failed_id() {
        let caller = "aa".repeat(16);
        let mut policy = styrene_rbac::RbacPolicy::default();
        policy.add_entry(styrene_rbac::RosterEntry::new(&caller, styrene_rbac::Role::Admin));
        let transport = Arc::new(crate::transport::mock_transport::MockTransport::new_default());
        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let nodes = Arc::new(styrene_services::node_store::NodeStore::in_memory().unwrap());
        let ctx = Arc::new(AppContext::with_policy(
            transport,
            "test-identity".into(),
            store,
            nodes,
            crate::services::PolicyService::new(policy),
        ));
        ctx.set_signer(Arc::new(rns_core::identity::PrivateIdentity::new_from_name(
            "projection-poison",
        )));
        ctx.messaging().inject_post_commit_failure(true);
        let facade = DaemonFacade::new(ctx, caller);
        let mut request = SendChatRequest::default();
        request.peer_hash = "48".repeat(16);
        request.content = "projection failure".into();
        request.delivery_method = Some("direct".into());

        let outcome = facade.send_chat_outcome(request).await.unwrap();

        assert_eq!(outcome.disposition, SendChatDisposition::Failed);
        assert!(!outcome.message_id.is_empty());
        assert_eq!(outcome.message.id, outcome.message_id);
        assert!(
            outcome
                .terminal_error
                .as_deref()
                .is_some_and(|error| error.contains("injected post-commit failure"))
        );
        assert!(outcome.paper_uri.is_none());
    }

    #[tokio::test]
    async fn history_and_manage_are_denied_to_default_remote_peers_without_writes() {
        let facade = make_facade_for_role(styrene_rbac::Role::Peer, Arc::new(NullTransport::new()));
        let peer = "56".repeat(16);
        let record = MessageRecord {
            id: "denied-mark".into(),
            source: peer.clone(),
            destination: "78".repeat(16),
            title: String::new(),
            content: "private".into(),
            timestamp: 1,
            direction: "in".into(),
            fields: None,
            receipt_status: None,
            read: false,
        };
        assert!(facade.ctx.messaging().accept_inbound_record(&record).unwrap());
        let read = facade.query_messages(&peer, 10, None).await;
        assert!(matches!(read, Err(IpcError::Denied { .. })));
        let exact_read = facade.query_message("denied-mark").await;
        assert!(matches!(exact_read, Err(IpcError::Denied { .. })));
        let mutation = facade.mark_read_outcome(&peer).await;
        assert!(matches!(mutation, Err(IpcError::Denied { .. })));
        assert!(!facade.ctx.messaging().get_message("denied-mark").unwrap().unwrap().read);
    }

    #[test]
    fn daemon_request_forwarding_lag_requires_reconciliation() {
        assert!(matches!(
            daemon_request_event(Err(broadcast::error::RecvError::Lagged(9))),
            Ok(DaemonEvent::RequestReconcileRequired { dropped: 9 })
        ));
    }

    #[test]
    fn path_observation_uses_event_time_and_monotonic_freshness() {
        use rns_core::hash::{AddressHash, Hash};
        use rns_core::transport::core_transport::path_table::PathSnapshot;

        let hash = AddressHash::new_from_hash(&Hash::new_from_slice(b"path"));
        let snapshot = PathSnapshot {
            destination: hash,
            hops: 1,
            received_from: hash,
            iface: hash,
            age: std::time::Duration::from_secs(PATH_FRESHNESS_THRESHOLD_SECS + 1),
            observed_at: std::time::UNIX_EPOCH + std::time::Duration::from_secs(100),
            lifetime: std::time::Duration::from_secs(600),
            expires_at: std::time::UNIX_EPOCH + std::time::Duration::from_secs(700),
        };
        let observation = path_observation(snapshot);

        assert_eq!(observation.observed_at, Some(100));
        assert_eq!(observation.age_secs, Some(PATH_FRESHNESS_THRESHOLD_SECS + 1));
        assert!(observation.stale);
        assert_eq!(path_expiry(snapshot), Some(700));
    }

    #[tokio::test]
    async fn link_snapshot_queries_transport_state_and_keeps_event_history_separate() {
        use crate::transport::mock_transport::MockTransport;
        use rns_core::hash::AddressHash;
        use rns_core::transport::destination_ext::link::{LinkStateSnapshot, LinkStatus};

        let transport = Arc::new(MockTransport::new_default());
        transport.set_link_snapshots(vec![LinkStateSnapshot {
            id: AddressHash::new([1; 16]),
            address_hash: AddressHash::new([2; 16]),
            interface: Some(AddressHash::new([3; 16])),
            rtt: Some(std::time::Duration::from_millis(9)),
            status: LinkStatus::Active,
            remote_identity: None,
            observed_at: std::time::SystemTime::now(),
            age: std::time::Duration::from_secs(1),
            close_reason: None,
        }]);
        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let ctx = Arc::new(AppContext::new(transport, "test-identity".into(), store));
        let mut historical =
            styrene_ipc::types::LinkEvent::new("old-link", "old-peer", "closed", None);
        historical.kind = styrene_ipc::types::LinkEventKind::Teardown;
        ctx.events().emit_link_event(historical);
        let facade = DaemonFacade::new(ctx, "test-caller".into());

        let snapshot = facade.link_snapshot().await.expect("link snapshot");

        assert_eq!(snapshot.active.len(), 1);
        assert_eq!(snapshot.active[0].link_id, hex::encode([1; 16]));
        assert_eq!(snapshot.history.len(), 1);
        assert_eq!(snapshot.history[0].link_id, "old-link");
    }

    #[tokio::test]
    async fn query_identity_returns_identity_hash() {
        let facade = make_facade();
        let info = facade.query_identity().await.unwrap();
        assert_eq!(info.identity_hash, "test-identity");
    }

    #[tokio::test]
    async fn announce_succeeds() {
        let facade = make_facade_for_role(
            styrene_rbac::Role::Operator,
            Arc::new(crate::transport::mock_transport::MockTransport::new_default()),
        );
        let result = facade.announce().await.unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn identity_announce_waits_for_hub_propagation_announce() {
        let facade = make_facade_for_role(
            styrene_rbac::Role::Operator,
            Arc::new(crate::transport::mock_transport::MockTransport::new_default()),
        );
        let (trigger, mut requests) =
            crate::standard_propagation::standard_propagation_announce_test_channel();
        facade.ctx.network_operations().set_propagation_announce_trigger(trigger);

        let announce = tokio::spawn(async move { facade.announce().await });
        let response = requests.recv().await.expect("propagation announce request");
        response
            .send(Ok(rns_core::transport::core_transport::SendPacketOutcome::SentBroadcast))
            .expect("identity announce still waiting");

        assert!(announce.await.unwrap().unwrap());
    }

    #[tokio::test]
    #[allow(clippy::field_reassign_with_default)]
    async fn peer_and_monitor_status_access_cannot_start_network_mutations() {
        use crate::transport::mock_transport::MockTransport;
        use styrene_ipc::types::{NetworkOperationKind, NetworkOperationOutcome};

        for role in [styrene_rbac::Role::Peer, styrene_rbac::Role::Monitor] {
            let transport = Arc::new(MockTransport::new_default());
            let facade = make_facade_for_role(role, transport.clone());
            assert!(facade.query_status().await.is_ok());
            for kind in [
                NetworkOperationKind::Announce,
                NetworkOperationKind::PathRequest,
                NetworkOperationKind::Probe,
                NetworkOperationKind::LinkOpen,
                NetworkOperationKind::LinkClose,
            ] {
                let mut request = styrene_ipc::types::StartNetworkOperationInfo::default();
                request.kind = kind;
                request.timeout_ms = 100;
                if matches!(
                    kind,
                    NetworkOperationKind::PathRequest | NetworkOperationKind::LinkOpen
                ) {
                    request.destination_hash = Some("11".repeat(16));
                }
                if matches!(kind, NetworkOperationKind::Probe | NetworkOperationKind::LinkClose) {
                    request.link_id = Some("22".repeat(16));
                }
                let denied = facade.start_network_operation(request).await.expect("typed denial");
                assert_eq!(denied.outcome, Some(NetworkOperationOutcome::Denied));
            }
            assert!(transport.calls().is_empty(), "read-only role mutated transport");
        }

        let transport = Arc::new(MockTransport::new_default());
        let facade = make_facade_for_role(styrene_rbac::Role::Operator, transport.clone());
        let mut request = styrene_ipc::types::StartNetworkOperationInfo::default();
        request.kind = NetworkOperationKind::Announce;
        request.timeout_ms = 100;
        let started = facade.start_network_operation(request).await.expect("operator start");
        tokio::task::yield_now().await;
        let completed = facade
            .network_operation(&started.operation_id)
            .await
            .unwrap()
            .expect("retained operation");
        assert_eq!(completed.outcome, Some(NetworkOperationOutcome::Dispatched));
        assert!(matches!(
            transport.calls().as_slice(),
            [crate::transport::mock_transport::MockCall::Announce { .. }]
        ));
    }

    #[tokio::test]
    async fn query_status_returns_basic_info() {
        let facade = make_facade();
        let status = facade.query_status().await.unwrap();
        assert!(!status.rns_initialized); // NullTransport
        assert_eq!(status.device_count, 0);
    }

    fn test_standard_propagation_policy()
    -> crate::standard_propagation::StandardPropagationRuntimePolicy {
        crate::standard_propagation::StandardPropagationRuntimePolicy {
            target_cost: 16,
            flexibility: 3,
            peering_cost: 18,
            transfer_limit_kb: 256,
            sync_limit_kb: 4000,
            queue_max_count: 4096,
            queue_max_bytes: 16 * 1024 * 1024,
            expiry_secs: 30 * 24 * 60 * 60,
            throttle_secs: 180,
            max_offer_links: 3,
        }
    }

    #[test]
    fn propagation_selection_requires_an_active_usable_selected_peer() {
        use crate::storage::standard_propagation::{
            StandardPropagationPeerObservation, StandardPropagationSelection,
        };

        let identity = [1; 16];
        let selection = StandardPropagationSelection {
            peer: Some(identity),
            mode: "manual".into(),
            selected_at: 1,
        };
        let mut peer = StandardPropagationPeerObservation {
            identity_hash: identity,
            propagation_destination: Some([2; 16]),
            configured: true,
            enabled: true,
            transfer_limit_kb: Some(256),
            sync_limit_kb: Some(4_000),
            stamp_cost: Some(16),
            stamp_flexibility: Some(3),
            peering_cost: Some(18),
            first_seen_at: 1,
            last_seen_at: 1,
            retry_at: None,
            backoff_count: 0,
            offered_count: 0,
            wanted_count: 0,
            accepted_count: 0,
            accepted_bytes: 0,
            failure_count: 0,
        };

        assert_eq!(
            standard_propagation_selection_readiness(true, None, &[]),
            StandardPropagationSelectionReadiness::NoSelection
        );
        assert_eq!(
            standard_propagation_selection_readiness(true, Some(&selection), &[]),
            StandardPropagationSelectionReadiness::Unavailable
        );
        assert_eq!(
            standard_propagation_selection_readiness(false, Some(&selection), &[peer.clone()]),
            StandardPropagationSelectionReadiness::Unavailable
        );
        peer.enabled = false;
        assert_eq!(
            standard_propagation_selection_readiness(true, Some(&selection), &[peer.clone()]),
            StandardPropagationSelectionReadiness::Unavailable
        );
        peer.enabled = true;
        peer.propagation_destination = None;
        assert_eq!(
            standard_propagation_selection_readiness(true, Some(&selection), &[peer.clone()]),
            StandardPropagationSelectionReadiness::Unavailable
        );
        peer.propagation_destination = Some([2; 16]);
        assert_eq!(
            standard_propagation_selection_readiness(true, Some(&selection), &[peer]),
            StandardPropagationSelectionReadiness::Ready
        );
    }

    #[tokio::test]
    async fn standard_propagation_query_distinguishes_absent_client_and_host_runtime() {
        let facade =
            make_facade_for_role(styrene_rbac::Role::Monitor, Arc::new(NullTransport::new()));
        let absent = facade.query_standard_propagation().await.unwrap();
        assert_eq!(absent.version, STANDARD_PROPAGATION_SNAPSHOT_VERSION);
        assert!(!absent.registered);
        assert!(!absent.active);
        assert_eq!(absent.selection_readiness, StandardPropagationSelectionReadiness::Unavailable);
        assert_eq!(absent.sync_readiness, StandardPropagationSyncReadiness::Unavailable);
        assert_eq!(absent.automatic_sync_enabled, None);
        assert!(absent.policy.is_none());
        assert!(absent.peers.is_empty());
        assert!(absent.attempts.is_empty());

        facade.ctx.publish_standard_propagation(
            crate::standard_propagation::StandardPropagationRuntimeObservation::client(),
        );
        let client = facade.query_standard_propagation().await.unwrap();
        assert!(!client.registered);
        assert!(client.active);
        assert_eq!(client.selection_readiness, StandardPropagationSelectionReadiness::NoSelection);
        assert_eq!(client.sync_readiness, StandardPropagationSyncReadiness::Unavailable);
        assert_eq!(client.automatic_sync_enabled, None);
        assert_eq!(client.last_synchronization, None);
        assert!(client.policy.is_none());
        assert!(client.observed_at.is_some());

        facade.ctx.publish_standard_propagation(
            crate::standard_propagation::StandardPropagationRuntimeObservation::registered(
                test_standard_propagation_policy(),
            ),
        );
        let present = facade.query_standard_propagation().await.unwrap();
        assert!(present.registered);
        assert!(!present.active);
        assert_eq!(present.selection_readiness, StandardPropagationSelectionReadiness::NoSelection);
        assert_eq!(present.sync_readiness, StandardPropagationSyncReadiness::Unavailable);
        assert_eq!(present.automatic_sync_enabled, None);
        assert_eq!(present.cooldown_remaining_secs, None);
        let policy = present.policy.unwrap();
        assert_eq!(policy.target_cost, 16);
        assert_eq!(policy.flexibility, 3);
        assert_eq!(policy.peering_cost, 18);
        assert_eq!(policy.transfer_limit_kb, 256);
        assert_eq!(policy.sync_limit_kb, 4000);
        assert_eq!(policy.queue_max_count, 4096);
        assert_eq!(policy.queue_max_bytes, 16 * 1024 * 1024);
        assert_eq!(policy.expiry_secs, 30 * 24 * 60 * 60);
        assert_eq!(policy.throttle_secs, 180);
        assert_eq!(policy.max_offer_links, 3);
        assert!(present.observed_at.is_some());
    }

    #[tokio::test]
    async fn standard_propagation_query_denies_unauthorized_and_returns_no_partial_on_poison() {
        let blocked = {
            use crate::services::PolicyService;
            let mut policy = styrene_rbac::RbacPolicy::default();
            policy.block("deadbeef");
            let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
            let ctx = Arc::new(AppContext::with_policy(
                Arc::new(NullTransport::new()),
                "daemon".into(),
                store,
                Arc::new(styrene_services::node_store::NodeStore::in_memory().unwrap()),
                PolicyService::new(policy),
            ));
            DaemonFacade::new(ctx, "deadbeef11112222333344445555aaaa".into())
        };
        assert!(matches!(blocked.query_standard_propagation().await, Err(IpcError::Denied { .. })));

        let facade =
            make_facade_for_role(styrene_rbac::Role::Monitor, Arc::new(NullTransport::new()));
        facade.ctx.publish_standard_propagation(
            crate::standard_propagation::StandardPropagationRuntimeObservation::registered(
                test_standard_propagation_policy(),
            ),
        );
        let store = Arc::clone(facade.ctx.store());
        let _ = std::thread::spawn(move || {
            let _guard = store.lock().unwrap();
            panic!("poison standard propagation store");
        })
        .join();
        assert!(matches!(
            facade.query_standard_propagation().await,
            Err(IpcError::Unavailable { .. })
        ));

        let failed =
            make_facade_for_role(styrene_rbac::Role::Monitor, Arc::new(NullTransport::new()));
        failed.ctx.publish_standard_propagation(
            crate::standard_propagation::StandardPropagationRuntimeObservation::registered(
                test_standard_propagation_policy(),
            ),
        );
        failed
            .ctx
            .store()
            .lock()
            .unwrap()
            .standard_propagation_fail_observation_for_test()
            .unwrap();
        assert!(matches!(
            failed.query_standard_propagation().await,
            Err(IpcError::Internal { .. })
        ));
    }

    #[tokio::test]
    async fn query_auto_reply_returns_disabled() {
        let facade = make_facade();
        let config = facade.query_auto_reply().await.unwrap();
        assert_eq!(config.mode, "disabled");
    }

    #[tokio::test]
    async fn set_auto_reply_updates_config() {
        let dir = tempfile::tempdir().unwrap();
        let facade =
            make_facade_for_role(styrene_rbac::Role::Admin, Arc::new(NullTransport::new()));
        facade.ctx.config().load_or_default(&dir.path().join("config.toml")).unwrap();
        facade.set_auto_reply("all", Some("I'm away"), Some(600)).await.unwrap();
        let config = facade.query_auto_reply().await.unwrap();
        assert_eq!(config.mode, "all");
        assert_eq!(config.message, Some("I'm away".into()));
        assert_eq!(config.cooldown_secs, Some(600));
    }

    #[tokio::test]
    async fn set_auto_reply_accepts_echo() {
        let dir = tempfile::tempdir().unwrap();
        let facade =
            make_facade_for_role(styrene_rbac::Role::Admin, Arc::new(NullTransport::new()));
        facade.ctx.config().load_or_default(&dir.path().join("config.toml")).unwrap();
        facade.set_auto_reply("echo", None, Some(0)).await.unwrap();
        let config = facade.query_auto_reply().await.unwrap();
        assert_eq!(config.mode, "echo");
        assert_eq!(config.cooldown_secs, Some(0));
    }

    #[tokio::test]
    async fn set_auto_reply_invalid_mode_returns_error() {
        let facade =
            make_facade_for_role(styrene_rbac::Role::Admin, Arc::new(NullTransport::new()));
        let result = facade.set_auto_reply("bogus", None, None).await;
        assert!(matches!(result, Err(IpcError::InvalidRequest { .. })));
    }

    #[tokio::test]
    async fn not_implemented_methods_return_correct_error() {
        let facade = make_facade();
        // Validation rejects an empty destination before transport dispatch.
        let result = facade.send_chat(SendChatRequest::default()).await;
        assert!(matches!(result, Err(IpcError::InvalidRequest { .. })));

        // list_tunnels returns Ok(empty) because TunnelService is wired but has no peers.
        let result = facade.list_tunnels().await;
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    #[allow(clippy::field_reassign_with_default)]
    async fn send_chat_rejects_noncanonical_or_mismatched_attachment_digest_before_service() {
        for expected in ["ABC", &"00".repeat(32)] {
            let facade = make_facade();
            let mut input = AttachmentInput::default();
            input.name = "digest.bin".into();
            input.bytes = vec![1, 2, 3];
            input.expected_sha256 = Some(expected.into());
            let mut request = SendChatRequest::default();
            request.peer_hash = "11".repeat(16);
            request.content = "body".into();
            request.attachments.push(input);
            assert!(matches!(
                facade.send_chat(request).await,
                Err(IpcError::InvalidRequest { .. })
            ));
        }
    }

    #[tokio::test]
    #[allow(clippy::field_reassign_with_default)]
    async fn blocked_caller_gets_denied() {
        use crate::services::PolicyService;
        use styrene_rbac::RbacPolicy;

        let mut policy = RbacPolicy::default();
        policy.block("deadbeef"); // block prefix

        let transport: Arc<dyn MeshTransport> = Arc::new(NullTransport::new());
        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let node_store = Arc::new(styrene_services::node_store::NodeStore::in_memory().unwrap());
        let ctx = Arc::new(AppContext::with_policy(
            transport,
            "daemon".into(),
            store,
            node_store,
            PolicyService::new(policy),
        ));

        // Caller whose hash starts with blocked prefix
        let facade = DaemonFacade::new(ctx, "deadbeef11112222333344445555aaaa".into());
        let result = facade.query_status().await;
        assert!(matches!(&result, Err(IpcError::Denied { .. })));
        assert!(!result.unwrap_err().is_retryable());

        let mut request = styrene_ipc::types::StartNetworkOperationInfo::default();
        request.kind = styrene_ipc::types::NetworkOperationKind::Announce;
        request.timeout_ms = 1_000;
        let denied = facade.start_network_operation(request).await.expect("typed denial");
        assert_eq!(denied.outcome, Some(styrene_ipc::types::NetworkOperationOutcome::Denied));
        assert_eq!(denied.observation.correlation_id, Some(denied.operation_id));
    }

    #[tokio::test]
    async fn peer_cannot_exec() {
        let transport: Arc<dyn MeshTransport> = Arc::new(NullTransport::new());
        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let ctx = Arc::new(AppContext::new(transport, "daemon".into(), store));
        // Default role is Peer — can chat/status but not exec
        let facade = DaemonFacade::new(ctx, "aaaa1111bbbb2222cccc3333dddd4444".into());

        let result = facade.exec("dest", "ls", vec![], None).await;
        assert!(matches!(result, Err(IpcError::Denied { .. })));
    }

    #[tokio::test]
    async fn query_devices_returns_announces() {
        let transport: Arc<dyn MeshTransport> = Arc::new(NullTransport::new());
        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let ctx = Arc::new(AppContext::new(transport, "daemon".into(), store));

        // Add some devices through discovery
        ctx.discovery()
            .accept_announce_with_details("node1".into(), 1000, Some("TestNode".into()), None, None)
            .unwrap();

        let facade = DaemonFacade::new(ctx, "caller".into());
        let devices = facade.query_devices(false).await.unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "TestNode");
    }
}
