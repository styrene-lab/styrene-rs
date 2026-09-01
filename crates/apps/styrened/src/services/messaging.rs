//! MessagingService — conversations, contacts, chat, sending, receipts, attachments.
//!
//! Owns: 3.1 conversations, 3.2 contacts, 3.3 chat handling, 3.4 sending,
//! 3.5 read receipts, 3.6 attachments. Also owns receipt correlation map
//! (packet_hash → message_id).
//! Package: F
//!
//! Composes existing modules:
//! - `MessagesStore` for persistence (messages table)
//! - `inbound_delivery::decode_inbound_payload()` for inbound message decoding
//! - `lxmf_bridge::build_wire_message()` for outbound wire format
//! - `receipt_bridge` helpers for receipt correlation
//!
//! The delivery pipeline (MeshTransport → link → fallback) lives here
//! per the decided split (Option C). MessagingService orchestrates:
//! transport.request_path → poll resolve_identity → send_via_link → fallback send_raw.

use crate::inbound_delivery::{InboundDecodeDiagnostics, decode_canonical_inbound_payload};
use crate::lxmf_bridge;
use crate::services::router::{
    DeliveryMethod, LifecycleEvidence, OutboundState, RetryQueueResult, RetryStartResult,
    RouterCoordinator, WireRepresentation,
};
use crate::storage::messages::{
    AttachmentBlobInput, AttemptRouteObservationRecord, MessageAttachmentRecord, MessageRecord,
    MessagesStore, OutboundAttemptRecord, OutboundRouteRecord,
};
use crate::transport::mesh_transport::{
    DispatchGate, LinkRepresentation, MeshTransport, TransportError,
};
use lxmf::inbound_decode::InboundPayloadMode;
use rns_core::destination::{DestinationDesc, DestinationName};
use rns_core::hash::AddressHash;
use sha2::Digest as _;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use styrene_ipc::types::{
    AttachmentInfo, AttachmentTransferInfo, MessageInfo, MessageRetryIneligibilityReason,
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

const TICKET_TTL_SECS: i64 = lxmf::stamps::TICKET_EXPIRY_SECS;
const TICKET_RENEWAL_SECS: i64 = lxmf::stamps::TICKET_RENEW_SECS;
const STAMP_GENERATION_TIMEOUT: Duration = Duration::from_secs(30);

fn current_unix_time_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
        .unwrap_or(0)
}
const MAX_RECEIPT_CORRELATIONS: usize = 4096;

pub(crate) fn attempt_record_to_info(
    attempt: OutboundAttemptRecord,
) -> styrene_ipc::types::MessageAttemptInfo {
    let mut info = styrene_ipc::types::MessageAttemptInfo::default();
    info.message_id = attempt.message_id;
    info.number = attempt.attempt_number;
    info.started_unix_ms = attempt.started_unix_ms;
    info.deadline_unix_ms = attempt.deadline_unix_ms;
    info.state = attempt.state;
    if let Some(observation) = attempt.route_observation {
        info.bearer = observation.bearer;
        info.route.outcome = if observation.outcome == "observed" {
            styrene_ipc::types::MessageAttemptRouteOutcome::Observed
        } else {
            styrene_ipc::types::MessageAttemptRouteOutcome::Unknown
        };
        info.route.connection_generation = observation.connection_generation;
        info.route.observed_at = observation.observed_at;
        info.route.next_hop = observation.next_hop;
        info.route.hops = observation.hops;
        info.route.stale = observation.stale;
        info.route.interface = observation.interface_id.map(|id| {
            let mut interface = styrene_ipc::types::MessageAttemptInterfaceObservation::default();
            interface.id = id;
            interface.kind = observation.interface_kind.unwrap_or_else(|| "unknown".into());
            interface.generation = observation.interface_generation.unwrap_or_default();
            interface
        });
    }
    info
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendCommitDisposition {
    Accepted,
    Failed,
    PaperExported,
}

#[derive(Clone, PartialEq)]
pub struct SendCommitOutcome {
    pub message_id: String,
    pub message: MessageInfo,
    pub disposition: SendCommitDisposition,
    pub requested_method: String,
    pub actual_method: String,
    pub fallback_reason: Option<String>,
    pub terminal_error: Option<String>,
    pub paper_uri: Option<String>,
}

impl std::fmt::Debug for SendCommitOutcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SendCommitOutcome")
            .field("message_id", &self.message_id)
            .field("disposition", &self.disposition)
            .field("requested_method", &self.requested_method)
            .field("actual_method", &self.actual_method)
            .field("fallback_reason", &self.fallback_reason)
            .field("terminal_error", &self.terminal_error)
            .field("paper_uri", &self.paper_uri.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

impl SendCommitOutcome {
    fn from_plan(
        message_id: &str,
        message: MessageInfo,
        plan: &crate::services::router::DeliveryPlan,
    ) -> Self {
        Self {
            message_id: message_id.into(),
            message,
            disposition: SendCommitDisposition::Accepted,
            requested_method: plan.requested_method.as_str().into(),
            actual_method: plan.actual_method.as_str().into(),
            fallback_reason: plan.fallback_reason.clone(),
            terminal_error: None,
            paper_uri: None,
        }
    }

    fn failed(mut self, error: impl Into<String>) -> Self {
        self.disposition = SendCommitDisposition::Failed;
        self.terminal_error = Some(error.into());
        self
    }
}

pub(crate) fn attachment_record_to_info(record: MessageAttachmentRecord) -> AttachmentInfo {
    let mut info = AttachmentInfo::default();
    info.ordinal = record.ordinal;
    info.id = if record.source == "issue" {
        format!("attachment-issue:{}", record.message_id)
    } else {
        format!("sha256:{}", hex::encode(record.digest))
    };
    info.name = record.wire_name;
    info.content_type = record.content_type.unwrap_or_default();
    info.size = record.byte_len;
    info.checksum = info.id.clone();
    info.availability = record.availability;
    info.integrity = record.integrity;
    if record.source == "issue" {
        let mut transfer = AttachmentTransferInfo::default();
        transfer.message_id = record.message_id;
        transfer.state = "failed".into();
        transfer.error = record.transfer_error;
        info.transfer = Some(Box::new(transfer));
        return info;
    }
    if let (Some(transfer_id), Some(representation), Some(direction), Some(state)) =
        (record.transfer_id, record.representation, record.direction, record.transfer_state)
    {
        let mut transfer = AttachmentTransferInfo::default();
        transfer.message_id = record.message_id;
        transfer.transfer_id = transfer_id;
        transfer.resource_hash = record.resource_hash.map(hex::encode);
        transfer.representation = representation;
        transfer.direction = direction;
        transfer.state = state;
        transfer.transferred = record.transferred;
        transfer.total = record.total;
        transfer.checksum_verified = record.checksum_verified;
        transfer.cancellable = transfer.representation == "resource"
            && transfer.direction == "outbound"
            && matches!(transfer.state.as_str(), "queued" | "transferring");
        transfer.error = record.transfer_error;
        info.transfer = Some(Box::new(transfer));
    }
    info
}

fn committed_projection_from_plan(
    record: &MessageRecord,
    plan: &crate::services::router::DeliveryPlan,
    attachments: &[AttachmentBlobInput],
) -> MessageInfo {
    let mut info = MessageInfo::default();
    info.id = record.id.clone();
    info.source_hash = record.source.clone();
    info.destination_hash = record.destination.clone();
    info.timestamp = record.timestamp;
    info.content = record.content.clone();
    info.title = (!record.title.is_empty()).then(|| record.title.clone());
    info.status = record.receipt_status.clone().unwrap_or_default();
    info.is_outgoing = true;
    info.delivery_method = Some(plan.actual_method.as_str().into());
    info.requested_delivery_method = Some(plan.requested_method.as_str().into());
    info.actual_delivery_method = Some(plan.actual_method.as_str().into());
    info.fallback_reason = plan.fallback_reason.clone();
    info.correlation_id = Some(plan.correlation_id.clone());
    info.read = record.read;
    info.attachments = attachments
        .iter()
        .enumerate()
        .map(|(ordinal, attachment)| {
            let mut info = AttachmentInfo::default();
            info.ordinal = u8::try_from(ordinal).unwrap_or(u8::MAX);
            info.id = format!("sha256:{}", hex::encode(sha2::Sha256::digest(&attachment.data)));
            info.name = attachment.wire_name.clone();
            info.content_type = attachment.content_type.clone().unwrap_or_default();
            info.size = attachment.data.len().try_into().unwrap_or(u64::MAX);
            info.checksum = info.id.clone();
            info.availability = "available".into();
            info.integrity = "verified".into();
            info
        })
        .collect();
    info.attachment_info = (info.attachments.len() == 1).then(|| info.attachments[0].clone());
    info
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutboundOperationPhase {
    Preparing,
    Dispatching(LinkRepresentation),
    Accepted(LinkRepresentation, Option<rns_core::hash::Hash>),
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy)]
struct OutboundOperationState {
    phase: OutboundOperationPhase,
    cancel_requested: bool,
}

struct OutboundOperation {
    state: Mutex<OutboundOperationState>,
    cancellation: CancellationToken,
    changed: Notify,
}

impl OutboundOperation {
    fn new() -> Self {
        Self {
            state: Mutex::new(OutboundOperationState {
                phase: OutboundOperationPhase::Preparing,
                cancel_requested: false,
            }),
            cancellation: CancellationToken::new(),
            changed: Notify::new(),
        }
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, OutboundOperationState> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn begin_dispatch(&self, representation: LinkRepresentation) -> Result<(), TransportError> {
        let mut state = self.lock_state();
        if state.cancel_requested {
            state.phase = OutboundOperationPhase::Cancelled;
            self.changed.notify_waiters();
            return Err(TransportError::Cancelled);
        }
        if state.phase != OutboundOperationPhase::Preparing {
            return Err(TransportError::SendFailed(
                "outbound operation is not dispatchable".into(),
            ));
        }
        state.phase = OutboundOperationPhase::Dispatching(representation);
        self.changed.notify_waiters();
        Ok(())
    }

    fn complete_dispatch(&self, result: &Result<(String, WireRepresentation), TransportError>) {
        let mut state = self.lock_state();
        if state.cancel_requested {
            state.phase = OutboundOperationPhase::Cancelled;
            self.changed.notify_waiters();
            return;
        }
        match result {
            Ok((hash, WireRepresentation::Resource)) => {
                let resource_hash = hex::decode(hash)
                    .ok()
                    .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
                    .map(rns_core::hash::Hash::new);
                state.phase =
                    OutboundOperationPhase::Accepted(LinkRepresentation::Resource, resource_hash);
            }
            Ok((_, WireRepresentation::Packet)) => {
                state.phase = OutboundOperationPhase::Accepted(LinkRepresentation::Packet, None);
            }
            Ok((_, WireRepresentation::Paper)) | Err(_) => {
                state.phase = if state.cancel_requested {
                    OutboundOperationPhase::Cancelled
                } else {
                    OutboundOperationPhase::Failed
                };
            }
        }
        self.changed.notify_waiters();
    }

    fn begin_fallback_dispatch(&self) -> Result<(), TransportError> {
        let mut state = self.lock_state();
        if state.cancel_requested {
            state.phase = OutboundOperationPhase::Cancelled;
            self.changed.notify_waiters();
            return Err(TransportError::Cancelled);
        }
        if !matches!(state.phase, OutboundOperationPhase::Dispatching(_)) {
            return Err(TransportError::SendFailed(
                "outbound operation is not eligible for packet fallback".into(),
            ));
        }
        state.phase = OutboundOperationPhase::Dispatching(LinkRepresentation::Packet);
        self.changed.notify_waiters();
        Ok(())
    }

    fn cancel_before_dispatch(&self) -> bool {
        let mut state = self.lock_state();
        if state.phase != OutboundOperationPhase::Preparing {
            return false;
        }
        state.cancel_requested = true;
        self.cancellation.cancel();
        self.changed.notify_waiters();
        true
    }

    fn request_resource_cancel(&self) -> Option<rns_core::hash::Hash> {
        let mut state = self.lock_state();
        match state.phase {
            OutboundOperationPhase::Dispatching(LinkRepresentation::Resource) => {
                state.cancel_requested = true;
                self.changed.notify_waiters();
                None
            }
            OutboundOperationPhase::Accepted(LinkRepresentation::Resource, hash) => hash,
            _ => None,
        }
    }

    async fn wait_for_cancel_handoff(&self) -> OutboundOperationPhase {
        loop {
            let notified = self.changed.notified();
            let state = *self.lock_state();
            if !matches!(
                state.phase,
                OutboundOperationPhase::Preparing
                    | OutboundOperationPhase::Dispatching(LinkRepresentation::Resource)
            ) {
                return state.phase;
            }
            notified.await;
        }
    }
}

fn canonical_peer_hash(peer_hash: &str) -> Result<String, std::io::Error> {
    let bytes: [u8; 16] = hex::decode(peer_hash)
        .map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "peer hash must be 32 hexadecimal characters",
            )
        })?
        .try_into()
        .map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "peer hash must encode 16 bytes")
        })?;
    Ok(hex::encode(bytes))
}

fn source_from_wire(data: &[u8], mode: InboundPayloadMode) -> Option<[u8; 16]> {
    let start = match mode {
        InboundPayloadMode::FullWire => 16,
        InboundPayloadMode::DestinationStripped => 0,
    };
    let source = data.get(start..start + 16)?;
    source.try_into().ok()
}

fn canonical_immutable_matches(
    stored: &crate::storage::messages::CanonicalInboundRecord,
    decoded: &crate::storage::messages::CanonicalInboundRecord,
) -> bool {
    stored.message_id == decoded.message_id
        && stored.source == decoded.source
        && stored.destination == decoded.destination
        && stored.title == decoded.title
        && stored.content == decoded.content
        && stored.timestamp.to_bits() == decoded.timestamp.to_bits()
        && stored.fields_msgpack.as_deref() == decoded.fields_msgpack.as_deref()
        && stored.signature == decoded.signature
        && stored.stamp == decoded.stamp
        && stored.wire == decoded.wire
}

/// Outcome of one inbound LXMF acceptance attempt.
#[derive(Debug)]
pub enum InboundAcceptOutcome {
    Accepted(MessageRecord),
    Duplicate { message_id: String },
    Rejected { diagnostics: InboundDecodeDiagnostics },
    StorageError { message_id: String, error: std::io::Error },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryMessageOutcome {
    Created(String),
    Existing(String),
    NotFound,
    TerminalConflict(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CancelMessageOutcome {
    Applied(String),
    AlreadyCancelled,
    NotFound,
    TerminalConflict(String),
}

#[derive(Default)]
struct ReceiptState {
    mappings: HashMap<String, (String, String)>,
}

const MAX_PENDING_RESOURCE_OBSERVATIONS: usize = 4096;

#[derive(Debug, Clone, Copy)]
struct PendingResourceObservation {
    received: u64,
    total: u64,
}

#[derive(Default)]
struct PendingResourceState {
    values: HashMap<[u8; 32], PendingResourceObservation>,
    order: VecDeque<[u8; 32]>,
}

/// Service managing chat messaging, conversations, and contacts.
pub struct MessagingService {
    store: Arc<Mutex<MessagesStore>>,
    /// Receipt correlation: packet_hash → message_id.
    /// Populated by send operations, consumed by receipt callbacks.
    receipts: Mutex<ReceiptState>,
    pending_resources: Mutex<PendingResourceState>,
    operations: Mutex<HashMap<String, Arc<OutboundOperation>>>,
    lifecycle_guard: Mutex<()>,
    events: std::sync::OnceLock<Arc<crate::services::EventService>>,
    /// Transport for outbound delivery (set once via set_signer).
    transport: std::sync::OnceLock<Arc<dyn MeshTransport>>,
    /// Signing key for LXMF wire messages (set once via set_signer).
    signer: std::sync::OnceLock<Arc<rns_core::identity::PrivateIdentity>>,
    standard_propagation:
        std::sync::OnceLock<Arc<crate::standard_propagation::StandardPropagationCoordinator>>,
    router: RouterCoordinator,
    retry_lock: tokio::sync::Mutex<()>,
    #[cfg(test)]
    post_commit_failure: Mutex<Option<bool>>,
}

impl MessagingService {
    async fn observed_route(
        &self,
        message_id: &str,
        attempt_number: u32,
        destination: &AddressHash,
    ) -> Option<AttemptRouteObservationRecord> {
        let transport = self.transport.get()?;
        let snapshot = transport.query_path_snapshot(destination).await?;
        let interface = transport
            .interface_snapshots()
            .await
            .into_iter()
            .find(|interface| interface.hash == snapshot.iface && interface.generation > 0)?;
        let observed_at = snapshot
            .observed_at
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64);
        Some(AttemptRouteObservationRecord {
            message_id: message_id.into(),
            attempt_number,
            outcome: "observed".into(),
            connection_generation: Some(interface.generation),
            observed_at,
            next_hop: Some(hex::encode(snapshot.received_from.as_slice())),
            hops: Some(snapshot.hops.into()),
            stale: std::time::SystemTime::now() > snapshot.expires_at,
            interface_id: Some(hex::encode(interface.hash.as_slice())),
            interface_kind: Some(interface.kind.as_str().into()),
            interface_generation: Some(interface.generation),
            bearer: None,
        })
    }

    async fn begin_attempt(
        &self,
        message_id: &str,
        destination: &AddressHash,
    ) -> Result<crate::services::router::OutboundAttempt, std::io::Error> {
        let number = self
            .router
            .message(message_id)
            .map(|message| message.total_attempts.saturating_add(1))
            .and_then(|number| u32::try_from(number).ok())
            .ok_or_else(|| std::io::Error::other("outbound LXMF attempt is unavailable"))?;
        let route = self.observed_route(message_id, number, destination).await;
        self.router.begin_attempt_with_route(message_id, route.as_ref())
    }

    fn lock_store(&self) -> Result<std::sync::MutexGuard<'_, MessagesStore>, std::io::Error> {
        self.store.lock().map_err(|_| std::io::Error::other("messages store lock poisoned"))
    }

    fn lock_lifecycle(&self) -> std::sync::MutexGuard<'_, ()> {
        self.lifecycle_guard.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn outbound_projection(&self, message_id: &str) -> Result<MessageInfo, std::io::Error> {
        let store = self.lock_store()?;
        let record = store
            .get_message(message_id)
            .map_err(std::io::Error::other)?
            .ok_or_else(|| std::io::Error::other("committed outbound message is unavailable"))?;
        let route = store
            .outbound_route(message_id)
            .map_err(std::io::Error::other)?
            .ok_or_else(|| std::io::Error::other("committed outbound route is unavailable"))?;
        let attempts = store.outbound_attempts(message_id).map_err(std::io::Error::other)?;
        let attachments =
            store.list_message_attachments(message_id).map_err(std::io::Error::other)?;
        let propagation = store
            .standard_propagation_links_for_message(message_id, 64)
            .map_err(std::io::Error::other)?;
        drop(store);

        let mut info = MessageInfo::default();
        info.id = record.id;
        info.source_hash = record.source;
        info.destination_hash = record.destination;
        info.timestamp = record.timestamp;
        info.content = record.content;
        info.title = (!record.title.is_empty()).then_some(record.title);
        info.status = record.receipt_status.unwrap_or_default();
        info.is_outgoing = record.direction == "out";
        info.delivery_method = Some(route.actual_method.clone());
        info.requested_delivery_method = Some(route.requested_method);
        info.actual_delivery_method = Some(route.actual_method);
        info.fallback_reason = route.fallback_reason;
        info.correlation_id = Some(route.correlation_id);
        info.attempts = attempts.into_iter().map(attempt_record_to_info).collect();
        info.read = record.read;
        info.attachments = attachments.into_iter().map(attachment_record_to_info).collect();
        info.attachment_info = (info.attachments.len() == 1).then(|| info.attachments[0].clone());
        info.propagation_correlations = propagation
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
        Ok(info)
    }

    fn operation(&self, message_id: &str) -> Option<Arc<OutboundOperation>> {
        self.operations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(message_id)
            .cloned()
    }

    fn remove_operation(&self, message_id: &str) {
        self.operations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(message_id);
    }

    fn remove_message_correlations(&self, message_id: &str) {
        self.receipts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .mappings
            .retain(|_, (mapped_message_id, _)| mapped_message_id != message_id);
        self.remove_operation(message_id);
    }

    fn release_ticket_offer_reservation(
        &self,
        reservation: Option<&crate::storage::messages::LxmfTicketOfferReservation>,
    ) {
        let Some(reservation) = reservation else {
            return;
        };
        let result = self.lock_store().and_then(|store| {
            store
                .release_ticket_offer_reservation(
                    &reservation.ticket.peer,
                    &reservation.reservation_id,
                )
                .map(|_| ())
                .map_err(std::io::Error::other)
        });
        if let Err(error) = result {
            crate::daemon_diagnostic!(
                "[messaging] failed to release ticket offer reservation: {error}"
            );
        }
    }

    /// Create with a shared store reference.
    pub fn with_store(store: Arc<Mutex<MessagesStore>>) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
            .unwrap_or(0);
        let ticket_startup_error = match store.lock() {
            Ok(guard) => {
                guard.reconcile_ticket_offer_startup(now).err().map(|error| error.to_string())
            }
            Err(_) => Some("messages store lock poisoned during ticket reconciliation".into()),
        };
        let router = RouterCoordinator::new(store.clone());
        if let Some(error) = ticket_startup_error {
            router.record_initialization_error(format!(
                "LXMF ticket offer reconciliation failed: {error}"
            ));
        }
        Self {
            store,
            receipts: Mutex::new(ReceiptState::default()),
            pending_resources: Mutex::new(PendingResourceState::default()),
            operations: Mutex::new(HashMap::new()),
            lifecycle_guard: Mutex::new(()),
            events: std::sync::OnceLock::new(),
            transport: std::sync::OnceLock::new(),
            signer: std::sync::OnceLock::new(),
            standard_propagation: std::sync::OnceLock::new(),
            router,
            retry_lock: tokio::sync::Mutex::new(()),
            #[cfg(test)]
            post_commit_failure: Mutex::new(None),
        }
    }

    #[cfg(test)]
    pub(crate) fn inject_post_commit_failure(&self, poison_store: bool) {
        *self.post_commit_failure.lock().unwrap() = Some(poison_store);
    }

    /// Wire transport and signer for outbound delivery (called once during bootstrap).
    pub fn set_signer(
        &self,
        transport: Arc<dyn MeshTransport>,
        signer: Arc<rns_core::identity::PrivateIdentity>,
    ) {
        let _ = self.transport.set(transport);
        let _ = self.signer.set(signer.clone());
        if let Some(transport) = self.transport.get() {
            let _ = self.standard_propagation.set(Arc::new(
                crate::standard_propagation::StandardPropagationCoordinator::new(
                    transport.clone(),
                    self.store.clone(),
                    signer,
                ),
            ));
        }
    }

    /// Set the propagation hub delivery hash for offline peer fallback.
    /// When direct delivery fails ("peer not announced"), messages are
    /// routed to this hub via PropagationIngest instead of failing.
    pub fn set_propagation_hub(
        &self,
        _hub_delivery_hash: String,
        _fleet: Arc<crate::services::FleetService>,
    ) {
        // Styrene CBOR propagation is not standard LXMF Propagated delivery.
        // Keep the composition API while declining to install it as a router fallback.
    }

    pub fn set_events(&self, events: Arc<crate::services::EventService>) {
        let _ = self.events.set(events);
    }

    pub(crate) fn standard_propagation_sync_telemetry(
        &self,
    ) -> Result<Option<serde_json::Value>, String> {
        self.store
            .lock()
            .map_err(|_| "standard propagation store lock poisoned".to_string())?
            .get_standard_propagation_sync_telemetry()
            .map_err(|error| error.to_string())
    }

    pub(crate) fn retain_standard_propagation_sync_telemetry(
        &self,
        telemetry: &serde_json::Value,
    ) -> Result<(), String> {
        self.store
            .lock()
            .map_err(|_| "standard propagation store lock poisoned".to_string())?
            .put_standard_propagation_sync_telemetry(telemetry)
            .map_err(|error| error.to_string())
    }

    pub async fn sync_standard_propagation_once(
        &self,
        deadline: std::time::Instant,
        cancellation: CancellationToken,
    ) -> Result<usize, TransportError> {
        self.standard_propagation
            .get()
            .ok_or(TransportError::Unavailable)?
            .sync_once(self, deadline, cancellation)
            .await
    }

    pub async fn resume_standard_propagation_outbound_once(
        &self,
        cancellation: CancellationToken,
    ) -> Result<usize, TransportError> {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
            .unwrap_or(0);
        let jobs = self
            .lock_store()
            .map_err(|error| TransportError::SendFailed(error.to_string()))?
            .standard_propagation_recoverable_outbound_jobs(now_ms, 64)
            .map_err(|error| TransportError::SendFailed(error.to_string()))?;
        let mut completed = 0usize;
        for (message_id, deadline_ms) in jobs {
            if cancellation.is_cancelled() {
                return Err(TransportError::Cancelled);
            }
            self.router
                .begin_attempt(&message_id)
                .map_err(|error| TransportError::SendFailed(error.to_string()))?;
            let remaining_ms = u64::try_from(deadline_ms.saturating_sub(now_ms)).unwrap_or(0);
            if remaining_ms == 0 {
                continue;
            }
            let deadline = std::time::Instant::now() + Duration::from_millis(remaining_ms);
            let job = self
                .lock_store()
                .map_err(|error| TransportError::SendFailed(error.to_string()))?
                .standard_propagation_client_job(&message_id)
                .map_err(|error| TransportError::SendFailed(error.to_string()))?
                .ok_or_else(|| TransportError::SendFailed("propagation job disappeared".into()))?;
            if job.state == "preparing" {
                let destination = AddressHash::new(job.destination);
                self.transport
                    .get()
                    .ok_or(TransportError::Unavailable)?
                    .request_path(&destination)
                    .await;
                let resolution_deadline =
                    deadline.min(std::time::Instant::now() + Duration::from_secs(12));
                let recipient = loop {
                    if cancellation.is_cancelled() {
                        return Err(TransportError::Cancelled);
                    }
                    if let Some(identity) = self
                        .transport
                        .get()
                        .ok_or(TransportError::Unavailable)?
                        .resolve_identity(&destination)
                        .await
                    {
                        break identity;
                    }
                    if std::time::Instant::now() >= resolution_deadline {
                        return Err(TransportError::SendFailed(
                            "propagated LXMF recipient identity unavailable during recovery".into(),
                        ));
                    }
                    tokio::select! {
                        () = cancellation.cancelled() => {
                            return Err(TransportError::Cancelled);
                        }
                        () = tokio::time::sleep(Duration::from_millis(25)) => {}
                    }
                };
                let coordinator =
                    self.standard_propagation.get().ok_or(TransportError::Unavailable)?.clone();
                let materialize_id = message_id.clone();
                let materialize_cancellation = cancellation.clone();
                tokio::task::spawn_blocking(move || {
                    coordinator.materialize_outbound(
                        &materialize_id,
                        &recipient,
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
                            .unwrap_or(0),
                        deadline,
                        &materialize_cancellation,
                    )
                })
                .await
                .map_err(|error| {
                    TransportError::SendFailed(format!("propagation recovery worker: {error}"))
                })??;
            }
            let now_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
                .unwrap_or(0);
            self.lock_store()
                .map_err(|error| TransportError::SendFailed(error.to_string()))?
                .standard_propagation_resume_outbound_attempt(
                    &message_id,
                    now_secs,
                    now_secs.saturating_add(
                        i64::try_from(
                            deadline.saturating_duration_since(std::time::Instant::now()).as_secs(),
                        )
                        .unwrap_or(i64::MAX),
                    ),
                )
                .map_err(|error| TransportError::SendFailed(error.to_string()))?;
            let _acceptance = self
                .standard_propagation
                .get()
                .ok_or(TransportError::Unavailable)?
                .upload(&message_id, deadline, cancellation.clone(), None)
                .await?;
            self.lock_store()
                .map_err(|error| TransportError::SendFailed(error.to_string()))?
                .standard_propagation_mark_upload_accepted(
                    &message_id,
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
                        .unwrap_or(0),
                )
                .map_err(|error| TransportError::SendFailed(error.to_string()))?;
            self.router
                .finish(&message_id, OutboundState::Sent, "sent: propagated")
                .map_err(|error| TransportError::SendFailed(error.to_string()))?;
            self.emit_status(&message_id, "sent: propagated", OutboundState::Sent, None);
            completed += 1;
        }
        Ok(completed)
    }

    /// Create a stub for tests (in-memory store).
    pub fn new() -> Self {
        let store = Arc::new(Mutex::new(MessagesStore::in_memory().expect("in-memory store")));
        Self::with_store(store)
    }

    // --- Outbound delivery ---

    /// Send a chat message via the delivery pipeline.
    ///
    /// Pipeline: build LXMF wire → persist → request_path → poll identity →
    /// send_via_link → track receipt. Returns the message ID on successful queue.
    pub async fn send_chat(
        &self,
        peer_hash: &str,
        content: &str,
        title: Option<&str>,
    ) -> Result<String, std::io::Error> {
        self.send_chat_with_method(peer_hash, content, title, None).await
    }

    pub async fn send_chat_with_fields(
        &self,
        peer_hash: &str,
        content: &str,
        title: Option<&str>,
        fields: serde_json::Value,
    ) -> Result<String, std::io::Error> {
        Ok(self
            .send_chat_outcome_with_route(
                peer_hash,
                content,
                title,
                None,
                None,
                None,
                &[],
                Some(fields),
            )
            .await?
            .message_id)
    }

    pub async fn send_chat_with_method(
        &self,
        peer_hash: &str,
        content: &str,
        title: Option<&str>,
        requested_method: Option<&str>,
    ) -> Result<String, std::io::Error> {
        Ok(self
            .send_chat_outcome_with_route(
                peer_hash,
                content,
                title,
                requested_method,
                None,
                None,
                &[],
                None,
            )
            .await?
            .message_id)
    }

    pub async fn send_chat_with_attachments(
        &self,
        peer_hash: &str,
        content: &str,
        title: Option<&str>,
        requested_method: Option<&str>,
        attachments: &[AttachmentBlobInput],
    ) -> Result<String, std::io::Error> {
        Ok(self
            .send_chat_outcome_with_route(
                peer_hash,
                content,
                title,
                requested_method,
                None,
                None,
                attachments,
                None,
            )
            .await?
            .message_id)
    }

    pub async fn send_chat_outcome_with_attachments(
        &self,
        peer_hash: &str,
        content: &str,
        title: Option<&str>,
        requested_method: Option<&str>,
        attachments: &[AttachmentBlobInput],
    ) -> Result<SendCommitOutcome, std::io::Error> {
        self.send_chat_outcome_with_route(
            peer_hash,
            content,
            title,
            requested_method,
            None,
            None,
            attachments,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn send_chat_outcome_with_route(
        &self,
        peer_hash: &str,
        content: &str,
        title: Option<&str>,
        requested_method: Option<&str>,
        correlation_id: Option<&str>,
        retry_of: Option<&str>,
        attachments: &[AttachmentBlobInput],
        fields: Option<serde_json::Value>,
    ) -> Result<SendCommitOutcome, std::io::Error> {
        let transport = self.transport.get().cloned().ok_or_else(|| {
            std::io::Error::other("transport not available — call set_signer() first")
        })?;
        let signer = self.signer.get().cloned().ok_or_else(|| {
            std::io::Error::other("signer not available — call set_signer() first")
        })?;

        let peer_hash = canonical_peer_hash(peer_hash)?;
        let dest_bytes: [u8; 16] = hex::decode(&peer_hash)
            .map_err(std::io::Error::other)?
            .try_into()
            .map_err(|_| std::io::Error::other("canonical peer hash must be 16 bytes"))?;
        let dest_hash = AddressHash::new(dest_bytes);

        // Build LXMF wire message
        let source_hash = transport.destination_hash();
        let mut source_bytes = [0u8; 16];
        source_bytes.copy_from_slice(source_hash.as_slice());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs().min(i64::MAX as u64) as i64)
            .unwrap_or(0);
        let (stamp_cost, outbound_ticket, ticket_offer) = {
            let store = self.lock_store()?;
            store.expire_lxmf_tickets(now).map_err(std::io::Error::other)?;
            let stamp_cost =
                store.peer_stamp_cost_at(&peer_hash, now).map_err(std::io::Error::other)?;
            let outbound_ticket = store
                .active_lxmf_ticket(&peer_hash, "received", now)
                .map_err(std::io::Error::other)?
                .map(|record| hex::encode(record.ticket));
            let mut issued = store
                .active_lxmf_ticket(&peer_hash, "issued", now)
                .map_err(std::io::Error::other)?;
            if issued
                .as_ref()
                .is_none_or(|ticket| ticket.expires_at.saturating_sub(now) <= TICKET_RENEWAL_SECS)
            {
                use rand_core::RngCore as _;
                let mut ticket = vec![0u8; lxmf::stamps::TICKET_LENGTH];
                rand_core::OsRng.fill_bytes(&mut ticket);
                let record = crate::storage::messages::LxmfTicketRecord {
                    peer: peer_hash.to_string(),
                    ticket,
                    expires_at: now.saturating_add(TICKET_TTL_SECS),
                    direction: "issued".into(),
                };
                store.upsert_lxmf_ticket(&record).map_err(std::io::Error::other)?;
                issued = Some(record);
            }
            let reservation = if let Some(ticket) = issued {
                use rand_core::RngCore as _;
                let mut reservation = [0u8; 16];
                rand_core::OsRng.fill_bytes(&mut reservation);
                let reservation_id = hex::encode(reservation);
                if store
                    .reserve_ticket_offer(&peer_hash, &reservation_id, now)
                    .map_err(std::io::Error::other)?
                {
                    Some(crate::storage::messages::LxmfTicketOfferReservation {
                        reservation_id,
                        ticket,
                    })
                } else {
                    None
                }
            } else {
                None
            };
            (stamp_cost, outbound_ticket, reservation)
        };
        if attachments.len() > lxmf::attachments::MAX_ATTACHMENT_COUNT
            || attachments.iter().any(|entry| {
                entry.wire_name.is_empty()
                    || entry.wire_name.len() > lxmf::attachments::MAX_ATTACHMENT_NAME_BYTES
                    || entry.data.len() > lxmf::attachments::MAX_ATTACHMENT_BYTES
            })
            || attachments.iter().map(|entry| entry.data.len()).sum::<usize>()
                > lxmf::attachments::MAX_ATTACHMENT_BYTES
        {
            self.release_ticket_offer_reservation(ticket_offer.as_ref());
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid or oversized LXMF attachments",
            ));
        }
        if requested_method.is_some_and(|method| method.eq_ignore_ascii_case("paper"))
            && !attachments.is_empty()
        {
            self.release_ticket_offer_reservation(ticket_offer.as_ref());
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "paper LXMF attachments are unsupported",
            ));
        }
        let wire_fields = if attachments.is_empty() {
            fields.clone()
        } else {
            let mut value = fields.clone().unwrap_or_else(|| serde_json::json!({}));
            let Some(map) = value.as_object_mut() else {
                self.release_ticket_offer_reservation(ticket_offer.as_ref());
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "LXMF fields must be an object",
                ));
            };
            map.insert(
                "attachments".into(),
                serde_json::json!(
                    attachments
                        .iter()
                        .map(|entry| serde_json::json!({
                            "name": entry.wire_name,
                            "data": entry.data,
                        }))
                        .collect::<Vec<_>>()
                ),
            );
            Some(value)
        };
        let title = title.unwrap_or("").to_string();
        let title_for_wire = title.clone();
        let content_owned = content.to_string();
        let outbound_ticket_owned = outbound_ticket.clone();
        let issued_ticket_owned = ticket_offer.as_ref().map(|offer| offer.ticket.clone());
        let stamp_deadline = std::time::Instant::now() + STAMP_GENERATION_TIMEOUT;
        let payload_result = tokio::task::spawn_blocking(move || {
            lxmf_bridge::build_wire_message_with_stamp_control(
                source_bytes,
                dest_bytes,
                &title_for_wire,
                &content_owned,
                wire_fields,
                &signer,
                stamp_cost,
                outbound_ticket_owned.as_deref(),
                issued_ticket_owned
                    .as_ref()
                    .map(|ticket| (ticket.expires_at, ticket.ticket.as_slice())),
                || std::time::Instant::now() >= stamp_deadline,
            )
        })
        .await;
        let payload = match payload_result {
            Ok(Ok(payload)) => payload,
            Ok(Err(error)) => {
                self.release_ticket_offer_reservation(ticket_offer.as_ref());
                return Err(std::io::Error::other(format!("wire encode: {error}")));
            }
            Err(error) => {
                self.release_ticket_offer_reservation(ticket_offer.as_ref());
                return Err(std::io::Error::other(format!("stamp worker failed: {error}")));
            }
        };

        // Generate message ID as SHA-256(dest || source || payload_without_stamp),
        // matching the inbound decoder's wire_message_id_hex computation.
        let msg_id = lxmf::inbound_decode::outbound_message_id_hex(&payload)
            .unwrap_or_else(|| hex::encode(sha2::Sha256::digest(&payload)));
        let record = MessageRecord {
            id: msg_id.clone(),
            source: hex::encode(source_hash.as_slice()),
            destination: peer_hash.to_string(),
            title: title.clone(),
            content: content.to_string(),
            timestamp: now,
            direction: "out".to_string(),
            fields: if attachments.is_empty() {
                fields
            } else {
                let mut value = fields.unwrap_or_else(|| serde_json::json!({}));
                if let Some(map) = value.as_object_mut() {
                    map.insert(
                        "5".into(),
                        serde_json::json!(
                            attachments
                                .iter()
                                .enumerate()
                                .map(|(ordinal, entry)| serde_json::json!({
                                    "ordinal": ordinal,
                                    "name": entry.wire_name,
                                    "size": entry.data.len(),
                                    "data": "stored_attachment",
                                }))
                                .collect::<Vec<_>>()
                        ),
                    );
                }
                Some(value)
            },
            receipt_status: Some("queued".to_string()),
            read: true, // Outgoing messages are always "read"
        };
        let opportunistic_payload =
            rns_core::transport::delivery::strip_destination_prefix(&payload, &dest_bytes);
        let propagated =
            requested_method.is_some_and(|method| method.trim().eq_ignore_ascii_case("propagated"));
        let propagation_preparation = if propagated {
            let coordinator = self.standard_propagation.get().cloned().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "no compatible propagation node is selected",
                )
            })?;
            Some(
                coordinator
                    .prepare_outbound(&msg_id, &payload, now)
                    .map_err(|error| std::io::Error::other(error.to_string()))?,
            )
        } else {
            None
        };
        let projection_attachments = attachments.to_vec();
        let _lifecycle = self.lock_lifecycle();
        let queued = if let (Some(correlation_id), Some(retry_of)) = (correlation_id, retry_of) {
            let queued = if let Some(propagation) = propagation_preparation.as_ref() {
                self.router.queue_retry_propagated_with_ticket_offer_and_attachments(
                    &record,
                    requested_method,
                    payload.len(),
                    opportunistic_payload.len(),
                    correlation_id,
                    retry_of,
                    ticket_offer.as_ref(),
                    attachments,
                    propagation,
                )
            } else {
                self.router.queue_retry_with_ticket_offer_and_attachments(
                    &record,
                    requested_method,
                    payload.len(),
                    opportunistic_payload.len(),
                    correlation_id,
                    retry_of,
                    ticket_offer.as_ref(),
                    attachments,
                )
            };
            match queued {
                Ok(RetryQueueResult::Queued(plan)) => Ok(plan),
                Ok(RetryQueueResult::Existing(route)) => {
                    self.release_ticket_offer_reservation(ticket_offer.as_ref());
                    let message = self.outbound_projection(&route.message_id)?;
                    return Ok(SendCommitOutcome {
                        message_id: route.message_id,
                        message,
                        disposition: SendCommitDisposition::Accepted,
                        requested_method: route.requested_method,
                        actual_method: route.actual_method,
                        fallback_reason: route.fallback_reason,
                        terminal_error: None,
                        paper_uri: None,
                    });
                }
                Err(error) => Err(error),
            }
        } else {
            if let Some(propagation) = propagation_preparation.as_ref() {
                self.router.queue_propagated_with_ticket_offer_and_attachments(
                    &record,
                    requested_method,
                    payload.len(),
                    opportunistic_payload.len(),
                    correlation_id,
                    ticket_offer.as_ref(),
                    attachments,
                    propagation,
                    &payload,
                )
            } else {
                self.router.queue_with_ticket_offer_and_attachments(
                    &record,
                    requested_method,
                    payload.len(),
                    opportunistic_payload.len(),
                    correlation_id,
                    ticket_offer.as_ref(),
                    attachments,
                    &payload,
                )
            }
        };
        let mut plan = match queued {
            Ok(plan) => plan,
            Err(error) => {
                self.release_ticket_offer_reservation(ticket_offer.as_ref());
                return Err(error);
            }
        };
        let fallback_projection =
            committed_projection_from_plan(&record, &plan, &projection_attachments);
        let projection = self.outbound_projection(&msg_id).unwrap_or(fallback_projection);
        let mut committed = SendCommitOutcome::from_plan(&msg_id, projection, &plan);
        let operation = matches!(
            plan.actual_method,
            DeliveryMethod::Direct | DeliveryMethod::Opportunistic | DeliveryMethod::Propagated
        )
        .then(|| Arc::new(OutboundOperation::new()));
        if let Some(operation) = &operation {
            self.operations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(msg_id.clone(), Arc::clone(operation));
        }
        drop(_lifecycle);
        match self.emit_attachment_transfers(&msg_id) {
            Ok(()) => {
                if let Ok(message) = self.outbound_projection(&msg_id) {
                    committed.message = message;
                }
            }
            Err(error) => {
                crate::daemon_diagnostic!(
                    "[messaging] attachment transfer observation failed after commit: {error}"
                );
            }
        }
        crate::daemon_diagnostic!(
            "[messaging-flow] stage=outbound_persisted id={} source={} destination={} bytes={}",
            msg_id,
            record.source,
            peer_hash,
            payload.len()
        );

        #[cfg(test)]
        let injected_post_commit_failure = self.post_commit_failure.lock().unwrap().take();
        #[cfg(test)]
        if injected_post_commit_failure == Some(true) {
            let store = Arc::clone(&self.store);
            let _ = std::panic::catch_unwind(|| {
                let _guard = store.lock().unwrap();
                panic!("injected post-commit store poison");
            });
        }

        let recovery = committed.clone();
        let post_commit: Result<SendCommitOutcome, std::io::Error> = async {
            #[cfg(test)]
            if injected_post_commit_failure.is_some() {
                return Err(std::io::Error::other("injected post-commit failure"));
            }
        match plan.actual_method {
            DeliveryMethod::Propagated => {
                self.begin_attempt(&msg_id, &dest_hash).await?;
                let operation = operation
                    .as_ref()
                    .ok_or_else(|| std::io::Error::other("propagated delivery ownership missing"))?;
                let cancellation = operation.cancellation.clone();
                transport.request_path(&dest_hash).await;
                let resolution_deadline = std::time::Instant::now()
                    + plan
                        .deadline
                        .saturating_duration_since(std::time::Instant::now())
                        .min(Duration::from_secs(12));
                let recipient = loop {
                    if cancellation.is_cancelled() {
                        let error = TransportError::Cancelled;
                        operation.complete_dispatch(&Err(error.clone()));
                        let _ =
                            self.apply_lifecycle_evidence(&msg_id, LifecycleEvidence::Cancelled)?;
                        self.remove_operation(&msg_id);
                        return Ok(committed.failed(error.to_string()));
                    }
                    if let Some(identity) = transport.resolve_identity(&dest_hash).await {
                        break Some(identity);
                    }
                    if std::time::Instant::now() >= resolution_deadline {
                        break None;
                    }
                    tokio::select! {
                        () = cancellation.cancelled() => {}
                        () = tokio::time::sleep(Duration::from_millis(25)) => {}
                    }
                };
                let Some(recipient) = recipient else {
                    let error = TransportError::SendFailed(
                        "propagated LXMF recipient identity unavailable".into(),
                    );
                    operation.complete_dispatch(&Err(error.clone()));
                    let _ = self.apply_lifecycle_evidence(
                        &msg_id,
                        LifecycleEvidence::Failed(error.to_string()),
                    )?;
                    self.remove_operation(&msg_id);
                    return Ok(committed.failed(error.to_string()));
                };
                let coordinator = self
                    .standard_propagation
                    .get()
                    .ok_or_else(|| std::io::Error::other("propagation coordinator missing"))?
                    .clone();
                let materialize_message_id = msg_id.clone();
                let materialize_cancellation = cancellation.clone();
                let materialize_deadline = plan.deadline;
                let materialized = match tokio::task::spawn_blocking(move || {
                    coordinator.materialize_outbound(
                        &materialize_message_id,
                        &recipient,
                        now,
                        materialize_deadline,
                        &materialize_cancellation,
                    )
                })
                .await
                {
                    Ok(result) => result.map_err(|error| std::io::Error::other(error.to_string())),
                    Err(error) => {
                        Err(std::io::Error::other(format!("propagation stamp worker: {error}")))
                    }
                };
                if let Err(error) = materialized {
                    operation.complete_dispatch(&Err(TransportError::SendFailed(error.to_string())));
                    let _ = self.apply_lifecycle_evidence(
                        &msg_id,
                        LifecycleEvidence::Failed(error.to_string()),
                    )?;
                    self.remove_operation(&msg_id);
                    return Ok(committed.failed(error.to_string()));
                }
                let gate_operation = Arc::clone(operation);
                let dispatch_gate: DispatchGate =
                    Arc::new(move |representation| gate_operation.begin_dispatch(representation));
                let result = self
                    .standard_propagation
                    .get()
                    .ok_or_else(|| std::io::Error::other("propagation coordinator missing"))?
                    .upload(&msg_id, plan.deadline, cancellation, Some(dispatch_gate))
                    .await;
                let acceptance = match result {
                    Ok(acceptance) => acceptance,
                    Err(error) => {
                        operation.complete_dispatch(&Err(error.clone()));
                        let _ = self.apply_lifecycle_evidence(
                            &msg_id,
                            LifecycleEvidence::Failed(error.to_string()),
                        )?;
                        return Ok(committed.failed(error.to_string()));
                    }
                };
                let evidence = match acceptance {
                    crate::standard_propagation::StandardPropagationUploadAcceptance::ResourceProof(
                        hash,
                    ) => (hex::encode(hash), WireRepresentation::Resource),
                    crate::standard_propagation::StandardPropagationUploadAcceptance::PacketProof(
                        hash,
                    ) => (hex::encode(hash), WireRepresentation::Packet),
                    crate::standard_propagation::StandardPropagationUploadAcceptance::AlreadyAccepted
                    | crate::standard_propagation::StandardPropagationUploadAcceptance::AlreadyPresent => {
                        (String::new(), WireRepresentation::Packet)
                    }
                };
                operation.complete_dispatch(&Ok(evidence));
                self.lock_store()?
                    .standard_propagation_mark_upload_accepted(
                        &msg_id,
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
                            .unwrap_or(0),
                    )
                    .map_err(std::io::Error::other)?;
                self.router.finish(&msg_id, OutboundState::Sent, "sent: propagated")?;
                self.remove_operation(&msg_id);
                self.emit_status(&msg_id, "sent: propagated", OutboundState::Sent, None);
                return Ok(committed);
            }
            DeliveryMethod::Paper => {
                self.begin_attempt(&msg_id, &dest_hash).await?;
                transport.request_path(&dest_hash).await;
                let recipient = transport.resolve_identity(&dest_hash).await;
                let Some(recipient) = recipient else {
                    let error = "paper LXMF recipient identity unavailable";
                    let _ = self.apply_lifecycle_evidence(
                        &msg_id,
                        LifecycleEvidence::Failed(error.into()),
                    )?;
                    return Ok(committed.failed(error));
                };
                let resolved_destination = rns_core::destination::SingleOutputDestination::new(
                    recipient,
                    DestinationName::new("lxmf", "delivery"),
                )
                .desc
                .address_hash;
                if resolved_destination != dest_hash {
                    let error = "paper LXMF recipient identity does not match requested destination";
                    let _ = self.apply_lifecycle_evidence(
                        &msg_id,
                        LifecycleEvidence::Failed(error.into()),
                    )?;
                    return Ok(committed.failed(error));
                }
                let wire = match lxmf::WireMessage::unpack(&payload) {
                    Ok(wire) => wire,
                    Err(error) => {
                        let error = format!("paper LXMF canonical wire: {error}");
                        let _ = self.apply_lifecycle_evidence(
                            &msg_id,
                            LifecycleEvidence::Failed(error.clone()),
                        )?;
                        return Ok(committed.failed(error));
                    }
                };
                let paper_uri = match wire.pack_paper_uri_with_rng(&recipient, rand_core::OsRng) {
                    Ok(uri)
                        if lxmf::WireMessage::decode_lxm_uri(&uri)
                            .is_ok_and(|paper| paper.len() <= lxmf::PAPER_MDU) =>
                    {
                        uri
                    }
                    Ok(_) => {
                        let error = "paper delivery content exceeds paper MDU";
                        let _ = self.apply_lifecycle_evidence(
                            &msg_id,
                            LifecycleEvidence::Failed(error.into()),
                        )?;
                        return Ok(committed.failed(error));
                    }
                    Err(error) => {
                        let error = format!("paper LXMF export: {error}");
                        let _ = self.apply_lifecycle_evidence(
                            &msg_id,
                            LifecycleEvidence::Failed(error.clone()),
                        )?;
                        return Ok(committed.failed(error));
                    }
                };
                self.router.finish(&msg_id, OutboundState::Sent, "sent: paper export")?;
                self.emit_status(&msg_id, "sent: paper export", OutboundState::Sent, None);
                return Ok(SendCommitOutcome {
                    disposition: SendCommitDisposition::PaperExported,
                    paper_uri: Some(paper_uri),
                    ..committed
                });
            }
            DeliveryMethod::Direct | DeliveryMethod::Opportunistic => {}
        }

        self.begin_attempt(&msg_id, &dest_hash).await?;
        if !transport.is_connected() {
            let operation = operation
                .as_ref()
                .ok_or_else(|| std::io::Error::other("delivery ownership missing"))?;
            operation.complete_dispatch(&Err(TransportError::Unavailable));
            let _ = self.apply_lifecycle_evidence(
                &msg_id,
                LifecycleEvidence::Failed("transport not connected".into()),
            )?;
            return Ok(committed.failed("transport not connected"));
        }
        let operation = operation.ok_or_else(|| std::io::Error::other("delivery ownership missing"))?;

        // Run the coordinator-selected delivery attempt.
        let mut delivery_result = match plan.actual_method {
            DeliveryMethod::Direct => {
                self.deliver_selected(
                    transport.as_ref(),
                    dest_hash,
                    &payload,
                    plan.deadline,
                    plan.representation,
                    Arc::clone(&operation),
                )
                .await
            }
            DeliveryMethod::Opportunistic => {
                let remaining = self.router.remaining(plan.deadline)?;
                if let Err(error) = operation.begin_dispatch(LinkRepresentation::Packet) {
                    let detail = error.to_string();
                    let result = Err(error);
                    operation.complete_dispatch(&result);
                    return Ok(committed.failed(detail));
                }
                match tokio::time::timeout(
                    remaining,
                    transport.send_raw(dest_hash, opportunistic_payload),
                )
                .await
                {
                    Err(_) => Err(TransportError::SendFailed("router deadline expired".into())),
                    Ok(Ok(outcome))
                        if rns_core::transport::delivery::send_outcome_is_sent(outcome) =>
                    {
                        Ok((String::new(), WireRepresentation::Packet))
                    }
                    Ok(Ok(outcome)) => Err(TransportError::SendFailed(
                        rns_core::transport::delivery::send_outcome_label(outcome).into(),
                    )),
                    Ok(Err(error)) => Err(error),
                }
            }
            DeliveryMethod::Propagated | DeliveryMethod::Paper => unreachable!(),
        };
        if plan.actual_method == DeliveryMethod::Direct
            && delivery_result.is_err()
            && !matches!(delivery_result, Err(TransportError::Cancelled))
            && opportunistic_payload.len() <= rns_core::packet::LXMF_MAX_PAYLOAD
        {
            let direct_error = delivery_result.as_ref().unwrap_err().to_string();
            if let Err(error) = operation.begin_fallback_dispatch() {
                delivery_result = Err(error);
            } else {
                let fallback = self.router.fallback_to_opportunistic(
                    &msg_id,
                    format!("direct delivery failed: {direct_error}"),
                )?;
                let remaining = self.router.remaining(fallback.deadline)?;
                delivery_result = match tokio::time::timeout(
                    remaining,
                    transport.send_raw(dest_hash, opportunistic_payload),
                )
                .await
                {
                    Err(_) => Err(TransportError::SendFailed("router deadline expired".into())),
                    Ok(Ok(outcome))
                        if rns_core::transport::delivery::send_outcome_is_sent(outcome) =>
                    {
                        Ok((String::new(), WireRepresentation::Packet))
                    }
                    Ok(Ok(outcome)) => Err(TransportError::SendFailed(
                        rns_core::transport::delivery::send_outcome_label(outcome).into(),
                    )),
                    Ok(Err(error)) => Err(error),
                };
                plan = fallback;
                committed.actual_method = plan.actual_method.as_str().into();
                committed.fallback_reason = plan.fallback_reason.clone();
            }
        }
        operation.complete_dispatch(&delivery_result);

        match &delivery_result {
            Ok((packet_hash, representation)) => {
                let _lifecycle = self.lock_lifecycle();
                crate::daemon_diagnostic!(
                    "[messaging-flow] stage=link_delivery_completed id={} destination={} packet={}",
                    msg_id,
                    peer_hash,
                    packet_hash
                );
                debug_assert_eq!(*representation, plan.representation);
                // Publish send state before exposing the mapping so an
                // immediate completion cannot be overwritten by this update.
                let method = match plan.actual_method {
                    DeliveryMethod::Direct => "direct",
                    DeliveryMethod::Opportunistic => "opportunistic",
                    DeliveryMethod::Propagated | DeliveryMethod::Paper => unreachable!(),
                };
                let status = format!("sent: {method}");
                let _ = self.router.finish(&msg_id, OutboundState::Sent, &status)?;
                self.emit_attachment_transfers(&msg_id)?;
                if !packet_hash.is_empty() {
                    self.track_receipt(packet_hash, &msg_id);
                }
            }
            Err(e) => {
                if matches!(e, TransportError::Cancelled) {
                    return Ok(committed.failed(e.to_string()));
                }
                let _ = self
                    .apply_lifecycle_evidence(&msg_id, LifecycleEvidence::Failed(e.to_string()))?;
            }
        }

        Ok(match delivery_result {
            Ok(_) => committed,
            Err(error) => committed.failed(error.to_string()),
        })
        }
        .await;

        let mut outcome = match post_commit {
            Ok(outcome) => outcome,
            Err(error) => {
                let detail = error.to_string();
                if let Err(terminal_error) = self
                    .apply_lifecycle_evidence(&msg_id, LifecycleEvidence::Failed(detail.clone()))
                {
                    crate::daemon_diagnostic!(
                        "[messaging] post-commit failure terminalization failed id={msg_id}: {terminal_error}"
                    );
                    if let Err(route_error) =
                        self.router.finish(&msg_id, OutboundState::Failed, &detail)
                    {
                        crate::daemon_diagnostic!(
                            "[messaging] post-commit route finalization failed id={msg_id}: {route_error}"
                        );
                    }
                }
                self.remove_operation(&msg_id);
                recovery.failed(detail)
            }
        };
        match self.outbound_projection(&msg_id) {
            Ok(message) if message.id == outcome.message_id => outcome.message = message,
            Ok(message) => {
                let detail = format!(
                    "persisted send projection ID mismatch: expected {}, observed {}",
                    outcome.message_id, message.id
                );
                outcome.disposition = SendCommitDisposition::Failed;
                outcome.paper_uri = None;
                outcome.terminal_error = Some(match outcome.terminal_error {
                    Some(error) => format!("{error}; {detail}"),
                    None => detail,
                });
            }
            Err(error) => {
                let detail = format!("persisted send projection freshness unavailable: {error}");
                outcome.disposition = SendCommitDisposition::Failed;
                outcome.paper_uri = None;
                outcome.terminal_error = Some(match outcome.terminal_error {
                    Some(error) => format!("{error}; {detail}"),
                    None => detail,
                });
            }
        }
        Ok(outcome)
    }

    /// Low-level delivery: request path → resolve identity → send via link.
    pub async fn deliver(
        transport: &dyn MeshTransport,
        dest_hash: AddressHash,
        payload: &[u8],
    ) -> Result<String, TransportError> {
        let expected = if payload.len() <= rns_core::transport::resource::LINK_PACKET_MDU {
            WireRepresentation::Packet
        } else {
            WireRepresentation::Resource
        };
        Self::deliver_until(
            transport,
            dest_hash,
            payload,
            std::time::Instant::now() + Duration::from_secs(32),
            expected,
            None,
            None,
        )
        .await
        .map(|(hash, _)| hash)
    }

    async fn deliver_selected(
        &self,
        transport: &dyn MeshTransport,
        dest_hash: AddressHash,
        payload: &[u8],
        deadline: std::time::Instant,
        expected: WireRepresentation,
        operation: Arc<OutboundOperation>,
    ) -> Result<(String, WireRepresentation), TransportError> {
        Self::deliver_until(
            transport,
            dest_hash,
            payload,
            deadline,
            expected,
            Some(&self.router),
            Some(operation),
        )
        .await
    }

    async fn deliver_until(
        transport: &dyn MeshTransport,
        dest_hash: AddressHash,
        payload: &[u8],
        deadline: std::time::Instant,
        expected: WireRepresentation,
        router: Option<&RouterCoordinator>,
        operation: Option<Arc<OutboundOperation>>,
    ) -> Result<(String, WireRepresentation), TransportError> {
        let cancellation = operation
            .as_ref()
            .map_or_else(CancellationToken::new, |operation| operation.cancellation.clone());
        let remaining = |deadline: std::time::Instant| {
            router.map_or_else(
                || {
                    let duration = deadline.saturating_duration_since(std::time::Instant::now());
                    if duration.is_zero() {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "outbound LXMF router deadline expired",
                        ))
                    } else {
                        Ok(duration)
                    }
                },
                |coordinator| coordinator.remaining(deadline),
            )
        };
        // Step 1: Request path
        crate::daemon_diagnostic!(
            "[messaging-flow] stage=path_request_started destination={}",
            hex::encode(dest_hash.as_slice())
        );
        tokio::select! {
            biased;
            () = cancellation.cancelled() => return Err(TransportError::Cancelled),
            result = tokio::time::timeout(
                remaining(deadline).map_err(|error| TransportError::SendFailed(error.to_string()))?,
                transport.request_path(&dest_hash),
            ) => result.map_err(|_| TransportError::SendFailed("router deadline expired".into()))?,
        }
        crate::daemon_diagnostic!(
            "[messaging-flow] stage=path_request_completed destination={}",
            hex::encode(dest_hash.as_slice())
        );

        // Step 2: Poll for peer identity (12s timeout)
        let mut identity = None;
        let resolution_deadline = router.map_or_else(
            || deadline.min(std::time::Instant::now() + Duration::from_secs(12)),
            |coordinator| coordinator.cap_deadline(deadline, Duration::from_secs(12)),
        );
        while remaining(resolution_deadline).is_ok() {
            let resolve_remaining = remaining(resolution_deadline)
                .map_err(|error| TransportError::SendFailed(error.to_string()))?;
            let resolved = tokio::select! {
                biased;
                () = cancellation.cancelled() => return Err(TransportError::Cancelled),
                result = tokio::time::timeout(
                    resolve_remaining,
                    transport.resolve_identity(&dest_hash),
                ) => result.map_err(|_| {
                    TransportError::SendFailed("identity resolution deadline expired".into())
                })?,
            };
            if let Some(found) = resolved {
                identity = Some(found);
                break;
            }
            let sleep_for = Duration::from_millis(250).min(
                remaining(resolution_deadline)
                    .map_err(|error| TransportError::SendFailed(error.to_string()))?,
            );
            tokio::select! {
                biased;
                () = cancellation.cancelled() => return Err(TransportError::Cancelled),
                () = tokio::time::sleep(sleep_for) => {}
            }
        }

        let identity = identity.ok_or_else(|| {
            crate::daemon_diagnostic!(
                "[messaging-flow] stage=identity_resolution_failed destination={}",
                hex::encode(dest_hash.as_slice())
            );
            TransportError::SendFailed("peer not announced — identity not resolved".into())
        })?;
        crate::daemon_diagnostic!(
            "[messaging-flow] stage=identity_resolved destination={}",
            hex::encode(dest_hash.as_slice())
        );

        // Step 3: Build destination descriptor
        let dest_desc = DestinationDesc {
            identity,
            address_hash: dest_hash,
            name: DestinationName::new("lxmf", "delivery"),
        };

        // Step 4: Send via link
        crate::daemon_diagnostic!(
            "[messaging-flow] stage=link_send_started destination={} bytes={}",
            hex::encode(dest_hash.as_slice()),
            payload.len()
        );
        let send_remaining =
            remaining(deadline).map_err(|error| TransportError::SendFailed(error.to_string()))?;
        let representation = match expected {
            WireRepresentation::Packet => LinkRepresentation::Packet,
            WireRepresentation::Resource => LinkRepresentation::Resource,
            WireRepresentation::Paper => {
                return Err(TransportError::SendFailed(
                    "paper representation cannot use a link".into(),
                ));
            }
        };
        let owned_send = operation.is_some();
        let send = if let Some(operation) = operation {
            let gate_operation = Arc::clone(&operation);
            let dispatch_gate: DispatchGate =
                Arc::new(move |actual| gate_operation.begin_dispatch(actual));
            transport.send_via_link_selected_cancellable(
                dest_desc,
                payload,
                send_remaining,
                representation,
                cancellation,
                dispatch_gate,
            )
        } else {
            transport.send_via_link_selected(dest_desc, payload, send_remaining, representation)
        };
        let send_result = if owned_send {
            Ok(send.await)
        } else {
            tokio::time::timeout(send_remaining, send).await
        };
        let result = match send_result {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => {
                crate::daemon_diagnostic!(
                    "[messaging-flow] stage=link_send_failed destination={} error={}",
                    hex::encode(dest_hash.as_slice()),
                    error
                );
                return Err(error);
            }
            Err(_) => return Err(TransportError::SendFailed("router deadline expired".into())),
        };

        let delivered = match result {
            rns_core::transport::delivery::LinkSendResult::Packet(packet) => {
                (hex::encode(packet.hash().to_bytes()), WireRepresentation::Packet)
            }
            rns_core::transport::delivery::LinkSendResult::Resource(hash) => {
                (hex::encode(hash.to_bytes()), WireRepresentation::Resource)
            }
        };
        Ok(delivered)
    }

    // --- Inbound ---

    /// Decodes and persists an inbound LXMF wire payload while preserving a
    /// structured distinction between accepted, duplicate, malformed, and
    /// storage-failed outcomes.
    pub fn accept_inbound(
        &self,
        destination: [u8; 16],
        data: &[u8],
        payload_mode: InboundPayloadMode,
    ) -> InboundAcceptOutcome {
        self.accept_inbound_with_identity(destination, data, payload_mode, None)
    }

    pub fn accept_inbound_with_identity(
        &self,
        destination: [u8; 16],
        data: &[u8],
        payload_mode: InboundPayloadMode,
        sender_identity: Option<&rns_core::identity::Identity>,
    ) -> InboundAcceptOutcome {
        self.accept_inbound_with_identity_and_transfer(
            destination,
            data,
            payload_mode,
            sender_identity,
            None,
            None,
        )
    }

    pub fn accept_inbound_resource_with_identity(
        &self,
        destination: [u8; 16],
        data: &[u8],
        payload_mode: InboundPayloadMode,
        sender_identity: Option<&rns_core::identity::Identity>,
        transfer: crate::storage::messages::InboundAttachmentTransferEvidence,
    ) -> InboundAcceptOutcome {
        let resource_hash = transfer.resource_hash;
        let pending = {
            let mut pending =
                self.pending_resources.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let value = pending.values.remove(&resource_hash);
            pending.order.retain(|candidate| *candidate != resource_hash);
            value
        };
        let transferred =
            pending.map_or(transfer.transferred, |value| value.received.max(transfer.transferred));
        let total = pending.map_or(transfer.total, |value| value.total.max(transfer.total));
        self.accept_inbound_with_identity_and_transfer(
            destination,
            data,
            payload_mode,
            sender_identity,
            Some(crate::storage::messages::InboundAttachmentTransferEvidence {
                resource_hash,
                transferred: transferred.min(total),
                total,
                checksum_verified: transfer.checksum_verified,
            }),
            None,
        )
    }

    pub fn accept_propagated_inbound(
        &self,
        destination: [u8; 16],
        full_wire: &[u8],
        transient_id: [u8; 32],
        attempt_id: [u8; 16],
        peer: [u8; 16],
    ) -> InboundAcceptOutcome {
        let outcome = self.accept_inbound_with_identity_and_transfer(
            destination,
            full_wire,
            InboundPayloadMode::FullWire,
            None,
            None,
            Some((transient_id, attempt_id, peer)),
        );
        if let InboundAcceptOutcome::Accepted(record) = &outcome
            && let (Some(events), Ok(Some(canonical))) =
                (self.events.get(), self.canonical_inbound(&record.id))
        {
            events.emit_message_new(record, Some(&canonical));
        }
        outcome
    }

    fn accept_inbound_with_identity_and_transfer(
        &self,
        destination: [u8; 16],
        data: &[u8],
        payload_mode: InboundPayloadMode,
        sender_identity: Option<&rns_core::identity::Identity>,
        transfer: Option<crate::storage::messages::InboundAttachmentTransferEvidence>,
        propagation: Option<([u8; 32], [u8; 16], [u8; 16])>,
    ) -> InboundAcceptOutcome {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
            .unwrap_or(0);
        let source = source_from_wire(data, payload_mode);
        let decoded = {
            let store = match self.lock_store() {
                Ok(store) => store,
                Err(error) => {
                    return InboundAcceptOutcome::StorageError { message_id: String::new(), error };
                }
            };
            if let Err(error) = store.expire_lxmf_tickets(now) {
                return InboundAcceptOutcome::StorageError {
                    message_id: String::new(),
                    error: std::io::Error::other(error),
                };
            }
            let policy = match store.lxmf_stamp_policy() {
                Ok(policy) => policy,
                Err(error) => {
                    return InboundAcceptOutcome::StorageError {
                        message_id: String::new(),
                        error: std::io::Error::other(error),
                    };
                }
            };
            let issued_tickets = match source.map_or(Ok(Vec::new()), |source| {
                store.issued_lxmf_tickets(&hex::encode(source), now)
            }) {
                Ok(tickets) => tickets,
                Err(error) => {
                    return InboundAcceptOutcome::StorageError {
                        message_id: String::new(),
                        error: std::io::Error::other(error),
                    };
                }
            };
            decode_canonical_inbound_payload(
                destination,
                data,
                payload_mode,
                sender_identity,
                Some(policy.target_cost.saturating_sub(policy.flexibility)),
                &issued_tickets,
            )
        };
        let decoded = match decoded {
            Ok(decoded) => decoded,
            Err(error) => {
                let mut diagnostics = InboundDecodeDiagnostics::default();
                diagnostics.attempts.push(crate::inbound_delivery::DecodeAttempt {
                    candidate: match payload_mode {
                        InboundPayloadMode::FullWire => "full_wire",
                        InboundPayloadMode::DestinationStripped => "destination_stripped",
                    },
                    len: data.len(),
                    error: error.to_string(),
                });
                return InboundAcceptOutcome::Rejected { diagnostics };
            }
        };
        if let Some((expires_at, _)) = decoded.received_ticket.as_ref()
            && (*expires_at <= now
                || *expires_at
                    > now.saturating_add(TICKET_TTL_SECS + lxmf::stamps::TICKET_GRACE_SECS))
        {
            let mut diagnostics = InboundDecodeDiagnostics::default();
            diagnostics.attempts.push(crate::inbound_delivery::DecodeAttempt {
                candidate: "ticket",
                len: data.len(),
                error: "LXMF ticket expiry outside accepted window".into(),
            });
            return InboundAcceptOutcome::Rejected { diagnostics };
        }
        let record = decoded.projection;
        let canonical = decoded.canonical;
        let received_ticket = decoded.received_ticket;
        let attachment_result = canonical
            .fields_msgpack
            .as_deref()
            .map(|bytes| {
                rmp_serde::from_slice::<rmpv::Value>(bytes)
                    .map_err(|error| lxmf::LxmfError::Decode(error.to_string()))
                    .and_then(|fields| lxmf::attachments::parse_attachment_field(Some(&fields)))
            })
            .unwrap_or_else(|| Ok(Vec::new()));
        let (attachments, attachment_issue) = match attachment_result {
            Ok(entries) => (
                entries
                    .into_iter()
                    .map(|entry| crate::storage::messages::AttachmentBlobInput {
                        wire_name: entry.filename,
                        data: entry.data,
                        content_type: None,
                        source: match entry.source {
                            lxmf::attachments::AttachmentFieldSource::CanonicalBinary => {
                                "canonical_binary"
                            }
                            lxmf::attachments::AttachmentFieldSource::RustIntegerArray => {
                                "rust_integer_array"
                            }
                        }
                        .into(),
                    })
                    .collect::<Vec<_>>(),
                None,
            ),
            Err(error) => {
                (Vec::new(), Some(error.to_string().chars().take(1024).collect::<String>()))
            }
        };
        let received_ticket = received_ticket.map(|(expires_at, ticket)| {
            crate::storage::messages::LxmfTicketRecord {
                peer: record.source.clone(),
                ticket,
                expires_at,
                direction: "received".into(),
            }
        });
        let store = match self.lock_store() {
            Ok(store) => store,
            Err(error) => {
                return InboundAcceptOutcome::StorageError { message_id: record.id, error };
            }
        };
        let inserted = store.insert_canonical_with_attachments_ticket_transfer_and_propagation(
            &record,
            &canonical,
            received_ticket.as_ref(),
            &attachments,
            attachment_issue.as_deref(),
            transfer,
            propagation.map(|(transient, attempt, peer)| (transient, attempt, peer, now)),
        );
        drop(store);
        match inserted {
            Ok(true) => {
                if let Err(error) = self.emit_attachment_transfers(&record.id) {
                    crate::daemon_diagnostic!(
                        "[messaging] inbound attachment observation failed after commit: {error}"
                    );
                }
                InboundAcceptOutcome::Accepted(record)
            }
            Ok(false) => match self.canonical_inbound(&record.id) {
                Ok(Some(stored)) if canonical_immutable_matches(&stored, &canonical) => {
                    crate::daemon_diagnostic!(
                        "[messaging] duplicate inbound dropped: {}",
                        record.id
                    );
                    InboundAcceptOutcome::Duplicate { message_id: record.id }
                }
                Ok(_) => InboundAcceptOutcome::StorageError {
                    message_id: record.id,
                    error: std::io::Error::other(
                        "duplicate lacks a matching durable canonical record",
                    ),
                },
                Err(error) => InboundAcceptOutcome::StorageError { message_id: record.id, error },
            },
            Err(error) => InboundAcceptOutcome::StorageError {
                message_id: record.id,
                error: std::io::Error::other(error),
            },
        }
    }

    pub fn revalidate_unknown_identity(
        &self,
        source: [u8; 16],
        identity: &rns_core::identity::Identity,
    ) -> Result<usize, std::io::Error> {
        let mut changed = 0usize;
        loop {
            let store = self.lock_store()?;
            let records =
                store.unknown_identity_messages(&source, 256).map_err(std::io::Error::other)?;
            if records.is_empty() {
                break;
            }
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
                .unwrap_or(0);
            for record in records {
                let decoded = decode_canonical_inbound_payload(
                    record.destination,
                    &record.wire,
                    InboundPayloadMode::FullWire,
                    Some(identity),
                    None,
                    &[],
                )
                .map_err(|error| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string())
                })?;
                if !canonical_immutable_matches(&record, &decoded.canonical) {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!(
                            "canonical LXMF record {} differs from stored wire",
                            record.message_id
                        ),
                    ));
                }
                let state = decoded.canonical.authentication_state.as_str();
                let ticket = decoded.received_ticket.and_then(|(expires_at, ticket)| {
                    (expires_at > now).then(|| crate::storage::messages::LxmfTicketRecord {
                        peer: hex::encode(source),
                        ticket,
                        expires_at,
                        direction: "received".into(),
                    })
                });
                let updated = store
                    .update_unknown_auth_with_verified_ticket(
                        &record.message_id,
                        state,
                        ticket.as_ref(),
                    )
                    .map_err(std::io::Error::other)?;
                if updated {
                    changed += 1;
                    if let (Some(events), Some(projection)) = (
                        self.events.get(),
                        store.get_message(&record.message_id).map_err(std::io::Error::other)?,
                    ) {
                        let mut authoritative = record.clone();
                        authoritative.authentication_state = state.into();
                        events.emit_message_authentication_changed(&projection, &authoritative);
                    }
                }
            }
        }
        Ok(changed)
    }

    pub fn canonical_inbound(
        &self,
        message_id: &str,
    ) -> Result<Option<crate::storage::messages::CanonicalInboundRecord>, std::io::Error> {
        self.lock_store()?.canonical_inbound(message_id).map_err(std::io::Error::other)
    }

    pub fn delivery_evidence(
        &self,
        message_id: &str,
    ) -> Result<Vec<crate::storage::messages::MessageDeliveryEvidenceRecord>, std::io::Error> {
        self.lock_store()?.message_delivery_evidence(message_id).map_err(std::io::Error::other)
    }

    pub fn terminal_detail(&self, message_id: &str) -> Result<Option<String>, std::io::Error> {
        self.lock_store()?.outbound_terminal_detail(message_id).map_err(std::io::Error::other)
    }

    pub fn inbound_is_dispatchable(&self, message_id: &str) -> Result<bool, std::io::Error> {
        let Some(canonical) = self.canonical_inbound(message_id)? else {
            return Ok(false);
        };
        Ok(matches!(canonical.authentication_state.as_str(), "verified" | "not_applicable")
            && canonical.stamp_state != "invalid")
    }

    /// Accept an already-decoded inbound message. Returns `true` only when this
    /// call inserted the first copy of the LXMF message.
    pub fn accept_inbound_record(&self, record: &MessageRecord) -> Result<bool, std::io::Error> {
        self.lock_store()?.insert_message_if_absent(record).map_err(std::io::Error::other)
    }

    // --- Querying ---

    /// Get a message by ID.
    pub fn get_message(&self, message_id: &str) -> Result<Option<MessageRecord>, std::io::Error> {
        self.lock_store()?.get_message(message_id).map_err(std::io::Error::other)
    }

    pub fn list_attachments(
        &self,
        message_id: &str,
    ) -> Result<Vec<MessageAttachmentRecord>, std::io::Error> {
        self.lock_store()?.list_message_attachments(message_id).map_err(std::io::Error::other)
    }

    pub fn attachment_blob_usage(&self) -> Result<(u64, u64), std::io::Error> {
        self.lock_store()?.attachment_blob_usage().map_err(std::io::Error::other)
    }

    pub fn query_attachment_chunk(
        &self,
        message_id: &str,
        ordinal: u8,
        offset: usize,
        max_bytes: usize,
    ) -> Result<Option<crate::storage::messages::AttachmentChunkRecord>, std::io::Error> {
        self.lock_store()?
            .query_attachment_chunk(message_id, ordinal, offset, max_bytes)
            .map_err(std::io::Error::other)
    }

    pub fn outbound_lifecycle(
        &self,
        message_id: &str,
    ) -> Result<Option<(OutboundRouteRecord, Vec<OutboundAttemptRecord>)>, std::io::Error> {
        let Some(route) = self.router.route(message_id)? else {
            return Ok(None);
        };
        let attempts =
            self.lock_store()?.outbound_attempts(message_id).map_err(std::io::Error::other)?;
        Ok(Some((route, attempts)))
    }

    pub fn retry_eligibility(
        &self,
        message_id: &str,
    ) -> Result<Option<(bool, Option<MessageRetryIneligibilityReason>)>, std::io::Error> {
        let Some(message) = self.get_message(message_id)? else {
            return Ok(None);
        };
        if message.direction != "out" {
            return Ok(Some((false, Some(MessageRetryIneligibilityReason::Inbound))));
        }
        let Some(route) = self.router.route(message_id)? else {
            return Ok(Some((false, Some(MessageRetryIneligibilityReason::MissingOutboundRoute))));
        };
        let attempts =
            self.lock_store()?.outbound_attempts(message_id).map_err(std::io::Error::other)?;
        let recovered_interruption = route.state == "queued"
            && attempts.last().is_some_and(|attempt| attempt.state == "interrupted");
        if !matches!(route.state.as_str(), "failed" | "expired") && !recovered_interruption {
            return Ok(Some((false, Some(MessageRetryIneligibilityReason::LifecycleState))));
        }
        if self
            .lock_store()?
            .canonical_outbound_wire(message_id)
            .map_err(std::io::Error::other)?
            .is_none()
        {
            return Ok(Some((
                false,
                Some(MessageRetryIneligibilityReason::CanonicalWireUnavailable),
            )));
        }
        if route.attempt_count as usize >= super::router::MAX_ATTEMPTS_PER_MESSAGE {
            return Ok(Some((false, Some(MessageRetryIneligibilityReason::AttemptLimitReached))));
        }
        Ok(Some((true, None)))
    }

    pub async fn retry_chat(&self, message: &MessageRecord) -> Result<String, std::io::Error> {
        match self.retry_message_outcome(&message.id).await? {
            RetryMessageOutcome::Created(id) | RetryMessageOutcome::Existing(id) => Ok(id),
            RetryMessageOutcome::NotFound => {
                Err(std::io::Error::new(std::io::ErrorKind::NotFound, "message not found"))
            }
            RetryMessageOutcome::TerminalConflict(state) => {
                Err(std::io::Error::other(format!("message is not retryable: {state}")))
            }
        }
    }

    async fn dispatch_persisted_retry(
        &self,
        message: &MessageRecord,
        payload: Vec<u8>,
        mut plan: crate::services::router::DeliveryPlan,
    ) -> Result<SendCommitOutcome, std::io::Error> {
        let transport = self
            .transport
            .get()
            .cloned()
            .ok_or_else(|| std::io::Error::other("transport not available"))?;
        let peer_hash = canonical_peer_hash(&message.destination)?;
        let dest_bytes: [u8; 16] = hex::decode(&peer_hash)
            .map_err(std::io::Error::other)?
            .try_into()
            .map_err(|_| std::io::Error::other("canonical peer hash must be 16 bytes"))?;
        let dest_hash = AddressHash::new(dest_bytes);
        let opportunistic_payload =
            rns_core::transport::delivery::strip_destination_prefix(&payload, &dest_bytes);
        let projection = self.outbound_projection(&message.id)?;
        let mut committed = SendCommitOutcome::from_plan(&message.id, projection, &plan);
        let operation = Arc::new(OutboundOperation::new());
        self.operations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(message.id.clone(), Arc::clone(&operation));
        if let Err(error) = self.emit_attachment_transfers(&message.id) {
            crate::daemon_diagnostic!(
                "[messaging] retry attachment transfer observation failed: {error}"
            );
        }

        if plan.actual_method == DeliveryMethod::Propagated {
            let coordinator = self
                .standard_propagation
                .get()
                .cloned()
                .ok_or_else(|| std::io::Error::other("propagation coordinator missing"))?;
            let cancellation = operation.cancellation.clone();
            let job = self
                .lock_store()?
                .standard_propagation_client_job(&message.id)
                .map_err(std::io::Error::other)?
                .ok_or_else(|| std::io::Error::other("propagation job disappeared"))?;
            if job.state == "preparing" {
                transport.request_path(&dest_hash).await;
                let resolution_deadline =
                    plan.deadline.min(std::time::Instant::now() + Duration::from_secs(12));
                let recipient = loop {
                    if cancellation.is_cancelled() {
                        return Err(std::io::Error::other(TransportError::Cancelled));
                    }
                    if let Some(identity) = transport.resolve_identity(&dest_hash).await {
                        break identity;
                    }
                    if std::time::Instant::now() >= resolution_deadline {
                        let detail = "propagated LXMF recipient identity unavailable";
                        let _ = self.apply_lifecycle_evidence(
                            &message.id,
                            LifecycleEvidence::Failed(detail.into()),
                        )?;
                        self.remove_operation(&message.id);
                        return Ok(committed.failed(detail));
                    }
                    tokio::select! {
                        () = cancellation.cancelled() => {}
                        () = tokio::time::sleep(Duration::from_millis(25)) => {}
                    }
                };
                let materialize_id = message.id.clone();
                let materialize_cancellation = cancellation.clone();
                let deadline = plan.deadline;
                tokio::task::spawn_blocking(move || {
                    coordinator.materialize_outbound(
                        &materialize_id,
                        &recipient,
                        current_unix_time_secs(),
                        deadline,
                        &materialize_cancellation,
                    )
                })
                .await
                .map_err(|error| {
                    std::io::Error::other(format!("propagation retry worker: {error}"))
                })?
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            }
            let now = current_unix_time_secs();
            self.lock_store()?
                .standard_propagation_resume_outbound_attempt(
                    &message.id,
                    now,
                    now.saturating_add(
                        i64::try_from(
                            plan.deadline
                                .saturating_duration_since(std::time::Instant::now())
                                .as_secs(),
                        )
                        .unwrap_or(i64::MAX),
                    ),
                )
                .map_err(std::io::Error::other)?;
            let gate_operation = Arc::clone(&operation);
            let dispatch_gate: DispatchGate =
                Arc::new(move |representation| gate_operation.begin_dispatch(representation));
            match self
                .standard_propagation
                .get()
                .ok_or_else(|| std::io::Error::other("propagation coordinator missing"))?
                .upload(&message.id, plan.deadline, cancellation, Some(dispatch_gate))
                .await
            {
                Ok(_) => {
                    self.lock_store()?
                        .standard_propagation_mark_upload_accepted(
                            &message.id,
                            current_unix_time_secs(),
                        )
                        .map_err(std::io::Error::other)?;
                    self.router.finish(&message.id, OutboundState::Sent, "sent: propagated")?;
                }
                Err(error) => {
                    let _ = self.apply_lifecycle_evidence(
                        &message.id,
                        LifecycleEvidence::Failed(error.to_string()),
                    )?;
                    committed = committed.failed(error.to_string());
                }
            }
            self.remove_operation(&message.id);
            if let Ok(projection) = self.outbound_projection(&message.id) {
                committed.message = projection;
            }
            return Ok(committed);
        }

        if !transport.is_connected() {
            operation.complete_dispatch(&Err(TransportError::Unavailable));
            let detail = "transport not connected";
            let _ = self
                .apply_lifecycle_evidence(&message.id, LifecycleEvidence::Failed(detail.into()))?;
            self.remove_operation(&message.id);
            committed = committed.failed(detail);
        } else {
            let mut delivery_result = match plan.actual_method {
                DeliveryMethod::Direct => {
                    self.deliver_selected(
                        transport.as_ref(),
                        dest_hash,
                        &payload,
                        plan.deadline,
                        plan.representation,
                        Arc::clone(&operation),
                    )
                    .await
                }
                DeliveryMethod::Opportunistic => {
                    let remaining = self.router.remaining(plan.deadline)?;
                    operation
                        .begin_dispatch(LinkRepresentation::Packet)
                        .map_err(std::io::Error::other)?;
                    match tokio::time::timeout(
                        remaining,
                        transport.send_raw(dest_hash, opportunistic_payload),
                    )
                    .await
                    {
                        Err(_) => Err(TransportError::SendFailed("router deadline expired".into())),
                        Ok(Ok(outcome))
                            if rns_core::transport::delivery::send_outcome_is_sent(outcome) =>
                        {
                            Ok((String::new(), WireRepresentation::Packet))
                        }
                        Ok(Ok(outcome)) => Err(TransportError::SendFailed(
                            rns_core::transport::delivery::send_outcome_label(outcome).into(),
                        )),
                        Ok(Err(error)) => Err(error),
                    }
                }
                DeliveryMethod::Propagated | DeliveryMethod::Paper => {
                    Err(TransportError::SendFailed("persisted retry method is unsupported".into()))
                }
            };
            if plan.actual_method == DeliveryMethod::Direct
                && delivery_result.is_err()
                && !matches!(delivery_result, Err(TransportError::Cancelled))
                && opportunistic_payload.len() <= rns_core::packet::LXMF_MAX_PAYLOAD
            {
                let direct_error = delivery_result.as_ref().unwrap_err().to_string();
                if let Err(error) = operation.begin_fallback_dispatch() {
                    delivery_result = Err(error);
                } else {
                    let fallback = self.router.fallback_to_opportunistic(
                        &message.id,
                        format!("direct delivery failed: {direct_error}"),
                    )?;
                    let remaining = self.router.remaining(fallback.deadline)?;
                    delivery_result = match tokio::time::timeout(
                        remaining,
                        transport.send_raw(dest_hash, opportunistic_payload),
                    )
                    .await
                    {
                        Err(_) => Err(TransportError::SendFailed("router deadline expired".into())),
                        Ok(Ok(outcome))
                            if rns_core::transport::delivery::send_outcome_is_sent(outcome) =>
                        {
                            Ok((String::new(), WireRepresentation::Packet))
                        }
                        Ok(Ok(outcome)) => Err(TransportError::SendFailed(
                            rns_core::transport::delivery::send_outcome_label(outcome).into(),
                        )),
                        Ok(Err(error)) => Err(error),
                    };
                    plan = fallback;
                    committed.actual_method = plan.actual_method.as_str().into();
                    committed.fallback_reason = plan.fallback_reason.clone();
                }
            }
            operation.complete_dispatch(&delivery_result);
            match &delivery_result {
                Ok((evidence_hash, representation)) => {
                    debug_assert_eq!(*representation, plan.representation);
                    let method = plan.actual_method.as_str();
                    let status = format!("sent: {method}");
                    self.router.finish(&message.id, OutboundState::Sent, &status)?;
                    self.emit_attachment_transfers(&message.id)?;
                    if !evidence_hash.is_empty() {
                        self.track_receipt(evidence_hash, &message.id);
                    }
                }
                Err(error) => {
                    let _ = self.apply_lifecycle_evidence(
                        &message.id,
                        LifecycleEvidence::Failed(error.to_string()),
                    )?;
                    committed = committed.failed(error.to_string());
                }
            }
            self.remove_operation(&message.id);
        }
        if let Ok(projection) = self.outbound_projection(&message.id) {
            committed.message = projection;
        }
        Ok(committed)
    }

    pub async fn retry_message_outcome(
        &self,
        message_id: &str,
    ) -> Result<RetryMessageOutcome, std::io::Error> {
        let _retry_guard = self.retry_lock.lock().await;
        let Some((eligible, reason)) = self.retry_eligibility(message_id)? else {
            return Ok(RetryMessageOutcome::NotFound);
        };
        if !eligible {
            let state = match reason {
                Some(MessageRetryIneligibilityReason::Inbound) => "inbound".into(),
                Some(MessageRetryIneligibilityReason::MissingOutboundRoute) => {
                    "nonretryable".into()
                }
                Some(MessageRetryIneligibilityReason::CanonicalWireUnavailable) => {
                    "canonical_wire_unavailable".into()
                }
                Some(MessageRetryIneligibilityReason::AttemptLimitReached) => {
                    return Err(std::io::Error::other("outbound LXMF attempt limit reached"));
                }
                Some(MessageRetryIneligibilityReason::LifecycleState) | None => self
                    .router
                    .route(message_id)?
                    .map(|route| route.state)
                    .unwrap_or_else(|| "nonretryable".into()),
                Some(_) => "nonretryable".into(),
            };
            return Ok(RetryMessageOutcome::TerminalConflict(state));
        }
        let message = self
            .get_message(message_id)?
            .ok_or_else(|| std::io::Error::other("retryable message disappeared"))?;
        let route = self
            .router
            .route(message_id)?
            .ok_or_else(|| std::io::Error::other("retryable outbound route disappeared"))?;
        let payload = self
            .lock_store()?
            .canonical_outbound_wire(message_id)
            .map_err(std::io::Error::other)?;
        let Some(payload) = payload else {
            return Ok(RetryMessageOutcome::TerminalConflict("canonical_wire_unavailable".into()));
        };
        if lxmf::inbound_decode::outbound_message_id_hex(&payload).as_deref() != Some(message_id) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "persisted canonical outbound wire does not match message ID",
            ));
        }
        let destination_bytes: [u8; 16] = hex::decode(&message.destination)
            .map_err(std::io::Error::other)?
            .try_into()
            .map_err(|_| std::io::Error::other("persisted destination has the wrong length"))?;
        let destination = AddressHash::new(destination_bytes);
        let attempt_number = route.attempt_count.saturating_add(1);
        let route_observation = self.observed_route(message_id, attempt_number, &destination).await;
        let plan =
            match self.router.begin_retry_with_route(message_id, route_observation.as_ref())? {
                RetryStartResult::Started(plan) => plan,
                RetryStartResult::Existing(route) => {
                    return Ok(if route.state == "sending" {
                        RetryMessageOutcome::Existing(message_id.into())
                    } else {
                        RetryMessageOutcome::TerminalConflict(route.state)
                    });
                }
                RetryStartResult::MissingCanonicalWire => {
                    return Ok(RetryMessageOutcome::TerminalConflict(
                        "canonical_wire_unavailable".into(),
                    ));
                }
            };
        let outcome = self.dispatch_persisted_retry(&message, payload, plan).await?;
        Ok(RetryMessageOutcome::Created(outcome.message_id))
    }

    pub async fn cancel_outbound(&self, message_id: &str) -> Result<bool, std::io::Error> {
        Ok(matches!(
            self.cancel_outbound_outcome(message_id).await?,
            CancelMessageOutcome::Applied(_)
        ))
    }

    pub async fn cancel_outbound_outcome(
        &self,
        message_id: &str,
    ) -> Result<CancelMessageOutcome, std::io::Error> {
        let mut resource_hash = None;
        let operation = {
            let _lifecycle = self.lock_lifecycle();
            let Some(message) = self.get_message(message_id)? else {
                return Ok(CancelMessageOutcome::NotFound);
            };
            if message.direction != "out" {
                return Ok(CancelMessageOutcome::TerminalConflict("inbound".into()));
            }
            let Some(route) = self.router.route(message_id)? else {
                return Ok(CancelMessageOutcome::TerminalConflict("noncancellable".into()));
            };
            if route.state == "cancelled" {
                return Ok(CancelMessageOutcome::AlreadyCancelled);
            }
            if route.state == "sent" && route.actual_method == "propagated" {
                return Ok(CancelMessageOutcome::TerminalConflict("sent".into()));
            }
            if matches!(route.state.as_str(), "delivered" | "failed" | "expired" | "rejected") {
                return Ok(CancelMessageOutcome::TerminalConflict(route.state));
            }

            let operation = self.operation(message_id);
            if let Some(operation) = &operation {
                let phase = operation.lock_state().phase;
                match phase {
                    OutboundOperationPhase::Preparing => {
                        operation.cancel_before_dispatch();
                    }
                    OutboundOperationPhase::Dispatching(LinkRepresentation::Packet)
                    | OutboundOperationPhase::Accepted(LinkRepresentation::Packet, _) => {
                        return Ok(CancelMessageOutcome::TerminalConflict(
                            "packet_dispatched".into(),
                        ));
                    }
                    OutboundOperationPhase::Dispatching(LinkRepresentation::Resource) => {
                        operation.request_resource_cancel();
                    }
                    OutboundOperationPhase::Accepted(LinkRepresentation::Resource, hash) => {
                        resource_hash = hash;
                    }
                    OutboundOperationPhase::Failed => {
                        return Ok(CancelMessageOutcome::TerminalConflict(
                            "dispatch_failed".into(),
                        ));
                    }
                    OutboundOperationPhase::Cancelled => {}
                }
            } else if route.representation == "resource" {
                resource_hash =
                    self.outbound_resource_evidence(message_id)?.and_then(|(evidence_id, _)| {
                        hex::decode(evidence_id)
                            .ok()
                            .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
                            .map(rns_core::hash::Hash::new)
                    });
                if route.state == "sent" && resource_hash.is_none() {
                    return Ok(CancelMessageOutcome::TerminalConflict(
                        "resource_evidence_unavailable".into(),
                    ));
                }
            } else if route.state == "sent" || route.state == "sending" {
                return Ok(CancelMessageOutcome::TerminalConflict("packet_dispatched".into()));
            }
            operation
        };

        if let Some(operation) = operation {
            let Ok(phase) =
                tokio::time::timeout(Duration::from_secs(2), operation.wait_for_cancel_handoff())
                    .await
            else {
                return Ok(CancelMessageOutcome::TerminalConflict(
                    "cancellation_handoff_timeout".into(),
                ));
            };
            match phase {
                OutboundOperationPhase::Cancelled => {}
                OutboundOperationPhase::Accepted(LinkRepresentation::Resource, hash) => {
                    resource_hash = hash;
                }
                OutboundOperationPhase::Dispatching(LinkRepresentation::Packet)
                | OutboundOperationPhase::Accepted(LinkRepresentation::Packet, _) => {
                    return Ok(CancelMessageOutcome::TerminalConflict("packet_dispatched".into()));
                }
                OutboundOperationPhase::Failed => {
                    return Ok(CancelMessageOutcome::TerminalConflict("dispatch_failed".into()));
                }
                OutboundOperationPhase::Preparing
                | OutboundOperationPhase::Dispatching(LinkRepresentation::Resource) => {
                    return Ok(CancelMessageOutcome::TerminalConflict(
                        "cancellation_handoff_incomplete".into(),
                    ));
                }
            }
        }

        if let Some(hash) = resource_hash {
            let Some(transport) = self.transport.get() else {
                return Ok(CancelMessageOutcome::TerminalConflict("transport_unavailable".into()));
            };
            match transport.cancel_resource(hash).await {
                Ok(true) => {}
                Ok(false) => {
                    return Ok(CancelMessageOutcome::TerminalConflict(
                        "resource_cleanup_rejected".into(),
                    ));
                }
                Err(error) => {
                    return Ok(CancelMessageOutcome::TerminalConflict(format!(
                        "resource_cleanup_failed: {error}"
                    )));
                }
            }
        }

        let _lifecycle = self.lock_lifecycle();
        let changed = self.router.apply_evidence(message_id, LifecycleEvidence::Cancelled)?;
        if !changed {
            let state = self
                .router
                .route(message_id)?
                .map_or_else(|| "deleted".into(), |route| route.state);
            return Ok(if state == "cancelled" {
                CancelMessageOutcome::AlreadyCancelled
            } else {
                CancelMessageOutcome::TerminalConflict(state)
            });
        }
        self.remove_message_correlations(message_id);
        if self.get_message(message_id)?.is_some() {
            self.emit_status(message_id, "cancelled", OutboundState::Cancelled, Some("cancelled"));
        }
        Ok(CancelMessageOutcome::Applied("cancelled".into()))
    }

    fn outbound_resource_evidence(
        &self,
        message_id: &str,
    ) -> Result<Option<(String, String)>, std::io::Error> {
        self.lock_store()?
            .outbound_evidence_for_message(message_id, "resource")
            .map_err(std::io::Error::other)
    }

    pub async fn reconcile_router_deadlines(&self) -> Result<Vec<String>, std::io::Error> {
        if let Some(transport) = self.transport.get() {
            for (_, evidence_id) in self.router.due_resource_evidence()? {
                let Ok(bytes) = hex::decode(evidence_id) else {
                    continue;
                };
                let Ok(hash) = <[u8; 32]>::try_from(bytes) else {
                    continue;
                };
                if let Err(error) = transport.cancel_resource(rns_core::hash::Hash::new(hash)).await
                {
                    crate::daemon_diagnostic!(
                        "[router] failed to cancel expiring resource: {error}"
                    );
                }
            }
        }
        let expired = self.router.reconcile_deadlines()?;
        for message_id in &expired {
            let _lifecycle = self.lock_lifecycle();
            self.remove_message_correlations(message_id);
            if self.get_message(message_id)?.is_some() {
                self.emit_status(
                    message_id,
                    "expired",
                    OutboundState::Expired,
                    Some("delivery deadline expired"),
                );
            }
        }
        Ok(expired)
    }

    /// List messages with pagination.
    pub fn list_messages(
        &self,
        limit: usize,
        before_ts: Option<i64>,
    ) -> Result<Vec<MessageRecord>, std::io::Error> {
        self.lock_store()?.list_messages(limit, before_ts).map_err(std::io::Error::other)
    }

    /// Count message buckets (inbound, outbound).
    pub fn count_messages(&self) -> Result<(u64, u64), std::io::Error> {
        self.lock_store()?.count_message_buckets().map_err(std::io::Error::other)
    }

    // --- Conversations & contacts (new store methods) ---

    /// List messages for a specific peer with pagination.
    pub fn list_messages_for_peer(
        &self,
        peer_hash: &str,
        limit: usize,
        before_ts: Option<i64>,
    ) -> Result<Vec<MessageRecord>, std::io::Error> {
        let peer_hash = canonical_peer_hash(peer_hash)?;
        self.lock_store()?
            .list_messages_for_peer(&peer_hash, limit, before_ts)
            .map_err(std::io::Error::other)
    }

    pub fn message_projection_snapshot_for_peer(
        &self,
        peer_hash: &str,
        limit: usize,
        before_ts: Option<i64>,
    ) -> Result<Vec<crate::storage::messages::MessageProjectionSnapshot>, std::io::Error> {
        let peer_hash = canonical_peer_hash(peer_hash)?;
        self.lock_store()?
            .message_projection_snapshot_for_peer(&peer_hash, limit, before_ts)
            .map_err(std::io::Error::other)
    }

    pub fn message_projection_page_for_peer(
        &self,
        peer_hash: &str,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<crate::storage::messages::MessageProjectionPage, crate::storage::messages::PageError>
    {
        let peer_hash = canonical_peer_hash(peer_hash).map_err(|error| {
            crate::storage::messages::PageError::InvalidCursor(error.to_string())
        })?;
        self.lock_store()
            .map_err(|error| crate::storage::messages::PageError::Internal(error.to_string()))?
            .message_projection_page_for_peer(&peer_hash, limit, cursor)
    }

    /// Mark unread inbound messages from a peer as read.
    pub fn mark_read(&self, peer_hash: &str) -> Result<u64, std::io::Error> {
        let peer_hash = canonical_peer_hash(peer_hash)?;
        self.lock_store()?.mark_read(&peer_hash).map_err(std::io::Error::other)
    }

    pub fn mark_read_outcome(
        &self,
        peer_hash: &str,
    ) -> Result<crate::storage::messages::ConversationMutationOutcome, std::io::Error> {
        let peer_hash = canonical_peer_hash(peer_hash)?;
        self.lock_store()?.mark_read_outcome(&peer_hash).map_err(std::io::Error::other)
    }

    /// Delete all messages in a conversation with a peer.
    pub fn delete_conversation(&self, peer_hash: &str) -> Result<u64, std::io::Error> {
        Ok(self.delete_conversation_outcome(peer_hash)?.affected_count)
    }

    pub fn delete_conversation_outcome(
        &self,
        peer_hash: &str,
    ) -> Result<crate::storage::messages::ConversationMutationOutcome, std::io::Error> {
        let peer_hash = canonical_peer_hash(peer_hash)?;
        let _lifecycle = self.lock_lifecycle();
        let (outcome, message_ids) = self.router.delete_conversation_outcome(&peer_hash)?;
        if outcome.disposition == crate::storage::messages::MutationDisposition::Applied {
            for message_id in message_ids {
                self.remove_message_correlations(&message_id);
            }
        }
        Ok(outcome)
    }

    /// Delete a single message by ID.
    pub fn delete_message(&self, message_id: &str) -> Result<bool, std::io::Error> {
        Ok(self.delete_message_outcome(message_id)?.disposition
            == crate::storage::messages::MutationDisposition::Applied)
    }

    pub fn delete_message_outcome(
        &self,
        message_id: &str,
    ) -> Result<crate::storage::messages::MessageMutationOutcome, std::io::Error> {
        let _lifecycle = self.lock_lifecycle();
        let outcome = self.router.delete_message_outcome(message_id)?;
        if outcome.disposition == crate::storage::messages::MutationDisposition::Applied {
            self.remove_message_correlations(message_id);
        }
        Ok(outcome)
    }

    /// Search messages by content substring.
    pub fn search_messages(
        &self,
        query: &str,
        peer_hash: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MessageRecord>, std::io::Error> {
        let peer_hash = peer_hash
            .map(|peer| {
                let canonical = canonical_peer_hash(peer)?;
                if canonical != peer {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "search peer hash must be canonical lowercase hexadecimal",
                    ));
                }
                Ok(canonical)
            })
            .transpose()?;
        self.lock_store()?
            .search_messages(query, peer_hash.as_deref(), limit)
            .map_err(std::io::Error::other)
    }

    pub fn search_message_projection_snapshot(
        &self,
        query: &str,
        peer_hash: Option<&str>,
        limit: usize,
    ) -> Result<Vec<crate::storage::messages::MessageProjectionSnapshot>, std::io::Error> {
        let peer_hash = peer_hash.map(canonical_peer_hash).transpose()?;
        self.lock_store()?
            .search_message_projection_snapshot(query, peer_hash.as_deref(), limit)
            .map_err(std::io::Error::other)
    }

    pub fn search_message_projection_outcome(
        &self,
        query: &str,
        peer_hash: Option<&str>,
        limit: usize,
    ) -> Result<crate::storage::messages::MessageSearchSnapshot, std::io::Error> {
        if query.is_empty() || query.len() > crate::storage::messages::MAX_SEARCH_QUERY_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "search query must be 1..=1024 UTF-8 bytes",
            ));
        }
        if !(1..=crate::storage::messages::MAX_MESSAGE_QUERY_LIMIT).contains(&limit) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "search limit must be between 1 and 256",
            ));
        }
        let peer_hash = peer_hash.map(canonical_peer_hash).transpose()?;
        self.lock_store()?
            .search_message_projection_outcome(query, peer_hash.as_deref(), limit)
            .map_err(std::io::Error::other)
    }

    /// List conversation summaries.
    pub fn start_conversation(
        &self,
        peer_hash: &str,
    ) -> Result<crate::storage::messages::ConversationMutationOutcome, std::io::Error> {
        let peer_hash = canonical_peer_hash(peer_hash)?;
        self.lock_store()?.start_conversation(&peer_hash).map_err(std::io::Error::other)
    }

    /// List conversation summaries.
    pub fn list_conversations(
        &self,
        unread_only: bool,
    ) -> Result<Vec<crate::storage::messages::ConversationSummary>, std::io::Error> {
        self.lock_store()?.list_conversations(unread_only).map_err(std::io::Error::other)
    }

    pub fn conversation_page(
        &self,
        unread_only: bool,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<crate::storage::messages::ConversationPage, crate::storage::messages::PageError>
    {
        self.lock_store()
            .map_err(|error| crate::storage::messages::PageError::Internal(error.to_string()))?
            .conversation_page(unread_only, limit, cursor)
    }

    /// Set a contact (upsert).
    pub fn set_contact(
        &self,
        peer_hash: &str,
        alias: Option<&str>,
        notes: Option<&str>,
    ) -> Result<crate::storage::messages::ContactRecord, std::io::Error> {
        let peer_hash = canonical_peer_hash(peer_hash)?;
        self.lock_store()?.set_contact(&peer_hash, alias, notes).map_err(std::io::Error::other)
    }

    pub fn set_contact_outcome(
        &self,
        peer_hash: &str,
        alias: Option<&str>,
        notes: Option<&str>,
    ) -> Result<crate::storage::messages::ContactMutationOutcome, std::io::Error> {
        let peer_hash = canonical_peer_hash(peer_hash)?;
        self.lock_store()?.set_contact_outcome(&peer_hash, alias, notes).map_err(
            |error| match error {
                rusqlite::Error::InvalidParameterName(message) => {
                    std::io::Error::new(std::io::ErrorKind::InvalidInput, message)
                }
                error => std::io::Error::other(error),
            },
        )
    }

    /// Remove a contact.
    pub fn remove_contact(&self, peer_hash: &str) -> Result<bool, std::io::Error> {
        let peer_hash = canonical_peer_hash(peer_hash)?;
        self.lock_store()?.remove_contact(&peer_hash).map_err(std::io::Error::other)
    }

    pub fn remove_contact_outcome(
        &self,
        peer_hash: &str,
    ) -> Result<crate::storage::messages::ContactMutationOutcome, std::io::Error> {
        let peer_hash = canonical_peer_hash(peer_hash)?;
        self.lock_store()?.remove_contact_outcome(&peer_hash).map_err(std::io::Error::other)
    }

    /// List all contacts.
    pub fn contact(
        &self,
        peer_hash: &str,
    ) -> Result<Option<crate::storage::messages::ContactRecord>, std::io::Error> {
        let peer_hash = canonical_peer_hash(peer_hash)?;
        self.lock_store()?.contact(&peer_hash).map_err(std::io::Error::other)
    }

    /// List all contacts.
    pub fn list_contacts(
        &self,
    ) -> Result<Vec<crate::storage::messages::ContactRecord>, std::io::Error> {
        self.lock_store()?.list_contacts().map_err(std::io::Error::other)
    }

    pub fn set_conversation_pinned(
        &self,
        peer_hash: &str,
        pinned: bool,
    ) -> Result<bool, std::io::Error> {
        let peer_hash = canonical_peer_hash(peer_hash)?;
        self.lock_store()?
            .set_conversation_pinned(&peer_hash, pinned)
            .map_err(std::io::Error::other)
    }

    pub fn set_conversation_muted(
        &self,
        peer_hash: &str,
        muted: bool,
    ) -> Result<bool, std::io::Error> {
        let peer_hash = canonical_peer_hash(peer_hash)?;
        self.lock_store()?.set_conversation_muted(&peer_hash, muted).map_err(std::io::Error::other)
    }

    pub fn set_conversation_flag_outcome(
        &self,
        peer_hash: &str,
        flag: &str,
        value: bool,
    ) -> Result<crate::storage::messages::ConversationMutationOutcome, std::io::Error> {
        let peer_hash = canonical_peer_hash(peer_hash)?;
        self.lock_store()?
            .set_conversation_flag_outcome(&peer_hash, flag, value)
            .map_err(std::io::Error::other)
    }

    pub fn set_draft(
        &self,
        peer_hash: &str,
        content: &str,
    ) -> Result<crate::storage::messages::ConversationDraft, std::io::Error> {
        let peer_hash = canonical_peer_hash(peer_hash)?;
        self.lock_store()?.set_draft(&peer_hash, content).map_err(std::io::Error::other)
    }

    pub fn draft(
        &self,
        peer_hash: &str,
    ) -> Result<Option<crate::storage::messages::ConversationDraft>, std::io::Error> {
        let peer_hash = canonical_peer_hash(peer_hash)?;
        self.lock_store()?.draft(&peer_hash).map_err(std::io::Error::other)
    }

    pub fn clear_draft(&self, peer_hash: &str) -> Result<bool, std::io::Error> {
        let peer_hash = canonical_peer_hash(peer_hash)?;
        self.lock_store()?.clear_draft(&peer_hash).map_err(std::io::Error::other)
    }

    pub fn clear_draft_if_revision(
        &self,
        peer_hash: &str,
        revision: u64,
    ) -> Result<bool, std::io::Error> {
        let peer_hash = canonical_peer_hash(peer_hash)?;
        self.lock_store()?
            .clear_draft_if_revision(&peer_hash, revision)
            .map_err(std::io::Error::other)
    }

    // --- Receipt tracking ---

    /// Track a receipt mapping (packet_hash → message_id).
    /// Called after successful send to correlate delivery receipts.
    pub fn track_receipt(&self, packet_hash: &str, message_id: &str) {
        let route = self.router.route(message_id).ok().flatten();
        if route.is_none() && self.get_message(message_id).ok().flatten().is_none() {
            return;
        }
        let kind = route.as_ref().map_or("packet", |route| {
            if route.representation == "resource" { "resource" } else { "packet" }
        });
        if route.is_some() {
            match self.router.track_evidence(packet_hash, message_id, kind) {
                Ok(true) => {}
                Ok(false) => return,
                Err(error) => {
                    crate::daemon_diagnostic!(
                        "[messaging] failed to persist receipt correlation: {error}"
                    );
                    return;
                }
            }
        }
        let mut receipts = self.receipts.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !receipts.mappings.contains_key(packet_hash)
            && receipts.mappings.len() >= MAX_RECEIPT_CORRELATIONS
            && let Some(oldest) = receipts.mappings.keys().next().cloned()
        {
            receipts.mappings.remove(&oldest);
        }
        receipts
            .mappings
            .insert(packet_hash.to_string(), (message_id.to_string(), kind.to_string()));
    }

    /// Resolve a packet hash to its originating message ID.
    pub fn resolve_receipt(&self, packet_hash: &str) -> Option<String> {
        self.receipts
            .lock()
            .unwrap()
            .mappings
            .get(packet_hash)
            .map(|value| value.0.clone())
            .or_else(|| {
                self.router.resolve_evidence(packet_hash).ok().flatten().map(|value| value.0)
            })
    }

    /// Apply an authenticated RNS delivery receipt for an exact tracked LXMF packet.
    pub fn handle_packet_delivery_receipt(
        &self,
        packet_hash: &str,
    ) -> Result<bool, std::io::Error> {
        self.handle_correlated_evidence(
            packet_hash,
            "packet",
            LifecycleEvidence::PacketDeliveryReceipt,
        )
    }

    /// Apply verified completion for an exact tracked outbound LXMF resource.
    pub fn handle_resource_complete(&self, resource_hash: &str) -> Result<bool, std::io::Error> {
        self.handle_correlated_evidence(
            resource_hash,
            "resource",
            LifecycleEvidence::ResourceDeliveryComplete,
        )
    }

    pub fn handle_resource_progress(
        &self,
        resource_hash: &[u8; 32],
        transferred: u64,
        total: u64,
    ) -> Result<bool, std::io::Error> {
        let evidence_hash = hex::encode(resource_hash);
        let evidence_changed = self
            .lock_store()?
            .update_delivery_evidence_progress(&evidence_hash, transferred, total)
            .map_err(std::io::Error::other)?;
        let attachment_changed = self
            .lock_store()?
            .update_attachment_transfer_progress(resource_hash, transferred, total)
            .map_err(std::io::Error::other)?;
        if evidence_changed || attachment_changed {
            if let Some((message_id, kind)) = self.router.resolve_evidence(&evidence_hash)?
                && kind == "resource"
            {
                if let Some(events) = self.events.get() {
                    events.emit_message_inspection_changed(&message_id);
                }
                if attachment_changed {
                    self.emit_attachment_transfers(&message_id)?;
                }
            }
        } else {
            let mut pending =
                self.pending_resources.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if !pending.values.contains_key(resource_hash) {
                while pending.order.len() >= MAX_PENDING_RESOURCE_OBSERVATIONS {
                    if let Some(oldest) = pending.order.pop_front() {
                        pending.values.remove(&oldest);
                    }
                }
                pending.order.push_back(*resource_hash);
            }
            pending.values.insert(
                *resource_hash,
                PendingResourceObservation { received: transferred.min(total), total },
            );
        }
        Ok(evidence_changed || attachment_changed)
    }

    pub fn forget_pending_resource(&self, resource_hash: &[u8; 32]) {
        let mut pending =
            self.pending_resources.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        pending.values.remove(resource_hash);
        pending.order.retain(|candidate| candidate != resource_hash);
    }

    pub fn handle_resource_failure(
        &self,
        resource_hash: &str,
        reason: &str,
    ) -> Result<bool, std::io::Error> {
        self.handle_correlated_evidence(
            resource_hash,
            "resource",
            LifecycleEvidence::Failed(format!("resource-{reason}")),
        )
    }

    pub fn handle_resource_cancelled(&self, resource_hash: &str) -> Result<bool, std::io::Error> {
        self.handle_correlated_evidence(resource_hash, "resource", LifecycleEvidence::Cancelled)
    }

    fn handle_correlated_evidence(
        &self,
        evidence_id: &str,
        expected_kind: &str,
        evidence: LifecycleEvidence,
    ) -> Result<bool, std::io::Error> {
        let correlation = self
            .receipts
            .lock()
            .unwrap()
            .mappings
            .get(evidence_id)
            .cloned()
            .or_else(|| self.router.resolve_evidence(evidence_id).ok().flatten());
        let Some((message_id, kind)) = correlation else {
            return Ok(false);
        };
        if kind != expected_kind {
            return Ok(false);
        }
        if self.router.route(&message_id)?.is_none() {
            return self.apply_lifecycle_evidence(&message_id, evidence);
        }
        let _lifecycle = self.lock_lifecycle();
        let delivered = matches!(
            &evidence,
            LifecycleEvidence::PacketDeliveryReceipt | LifecycleEvidence::ResourceDeliveryComplete
        );
        let status = match &evidence {
            LifecycleEvidence::PacketDeliveryReceipt => "delivered: packet-receipt".to_string(),
            LifecycleEvidence::ResourceDeliveryComplete => {
                "delivered: resource-complete".to_string()
            }
            LifecycleEvidence::Cancelled => "cancelled".to_string(),
            LifecycleEvidence::Expired => "expired".to_string(),
            LifecycleEvidence::Failed(reason) => format!("failed: {reason}"),
        };
        let (lifecycle_state, detail) = match &evidence {
            LifecycleEvidence::PacketDeliveryReceipt => {
                (OutboundState::Delivered, Some("authenticated packet receipt".to_string()))
            }
            LifecycleEvidence::ResourceDeliveryComplete => {
                (OutboundState::Delivered, Some("verified resource completion".to_string()))
            }
            LifecycleEvidence::Cancelled => {
                (OutboundState::Cancelled, Some("cancelled".to_string()))
            }
            LifecycleEvidence::Expired => {
                (OutboundState::Expired, Some("delivery deadline expired".to_string()))
            }
            LifecycleEvidence::Failed(reason) => (OutboundState::Failed, Some(reason.clone())),
        };
        let Some(exact_message_id) =
            self.router.apply_correlated_evidence(evidence_id, expected_kind, evidence)?
        else {
            return Ok(false);
        };
        debug_assert_eq!(exact_message_id, message_id);
        if delivered {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
                .unwrap_or(0);
            self.lock_store()?
                .mark_ticket_offer_delivered(&message_id, now)
                .map_err(std::io::Error::other)?;
        }
        self.remove_message_correlations(&message_id);
        if self.get_message(&message_id)?.is_some() {
            self.emit_status(&message_id, &status, lifecycle_state, detail.as_deref());
        }
        self.emit_attachment_transfers(&message_id)?;
        Ok(true)
    }

    fn apply_lifecycle_evidence(
        &self,
        message_id: &str,
        evidence: LifecycleEvidence,
    ) -> Result<bool, std::io::Error> {
        let _lifecycle = self.lock_lifecycle();
        let delivered = matches!(
            &evidence,
            LifecycleEvidence::PacketDeliveryReceipt | LifecycleEvidence::ResourceDeliveryComplete
        );
        let status = match &evidence {
            LifecycleEvidence::PacketDeliveryReceipt => "delivered: packet-receipt".to_string(),
            LifecycleEvidence::ResourceDeliveryComplete => {
                "delivered: resource-complete".to_string()
            }
            LifecycleEvidence::Cancelled => "cancelled".to_string(),
            LifecycleEvidence::Expired => "expired".to_string(),
            LifecycleEvidence::Failed(reason) => format!("failed: {reason}"),
        };
        let (lifecycle_state, detail) = match &evidence {
            LifecycleEvidence::PacketDeliveryReceipt => {
                (OutboundState::Delivered, Some("authenticated packet receipt".to_string()))
            }
            LifecycleEvidence::ResourceDeliveryComplete => {
                (OutboundState::Delivered, Some("verified resource completion".to_string()))
            }
            LifecycleEvidence::Cancelled => {
                (OutboundState::Cancelled, Some("cancelled".to_string()))
            }
            LifecycleEvidence::Expired => {
                (OutboundState::Expired, Some("delivery deadline expired".to_string()))
            }
            LifecycleEvidence::Failed(reason) => (OutboundState::Failed, Some(reason.clone())),
        };
        if self.router.route(message_id)?.is_none() {
            let changed = self
                .lock_store()?
                .update_receipt_status(message_id, &status)
                .map_err(std::io::Error::other)?;
            if changed {
                self.remove_message_correlations(message_id);
                if self.get_message(message_id)?.is_some() {
                    self.emit_status(message_id, &status, lifecycle_state, detail.as_deref());
                }
                self.emit_attachment_transfers(message_id)?;
            }
            return Ok(changed);
        }
        let changed = self.router.apply_evidence(message_id, evidence)?;
        if changed {
            if delivered {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
                    .unwrap_or(0);
                self.lock_store()?
                    .mark_ticket_offer_delivered(message_id, now)
                    .map_err(std::io::Error::other)?;
            }
            self.remove_message_correlations(message_id);
            if self.get_message(message_id)?.is_some() {
                self.emit_status(message_id, &status, lifecycle_state, detail.as_deref());
            }
            self.emit_attachment_transfers(message_id)?;
        }
        Ok(changed)
    }

    fn emit_status(
        &self,
        message_id: &str,
        status: &str,
        state: OutboundState,
        terminal_detail: Option<&str>,
    ) {
        if let Some(events) = self.events.get() {
            let lifecycle_state = match state {
                OutboundState::Queued => styrene_ipc::types::MessageLifecycleState::Queued,
                OutboundState::Sending => styrene_ipc::types::MessageLifecycleState::Sending,
                OutboundState::Sent => styrene_ipc::types::MessageLifecycleState::Sent,
                OutboundState::Delivered => styrene_ipc::types::MessageLifecycleState::Delivered,
                OutboundState::Failed => styrene_ipc::types::MessageLifecycleState::Failed,
                OutboundState::Cancelled => styrene_ipc::types::MessageLifecycleState::Cancelled,
                OutboundState::Expired => styrene_ipc::types::MessageLifecycleState::Expired,
            };
            let kind = match state {
                OutboundState::Delivered => styrene_ipc::types::MessageEventKind::Delivered,
                OutboundState::Failed | OutboundState::Expired => {
                    styrene_ipc::types::MessageEventKind::Failed
                }
                _ => styrene_ipc::types::MessageEventKind::StatusChanged,
            };
            events.emit_message_status(message_id, status, lifecycle_state, terminal_detail, kind);
        }
    }

    fn emit_attachment_transfers(&self, message_id: &str) -> Result<(), std::io::Error> {
        let Some(events) = self.events.get() else { return Ok(()) };
        for record in self.list_attachments(message_id)? {
            let (Some(transfer_id), Some(representation), Some(direction), Some(state)) = (
                record.transfer_id,
                record.representation,
                record.direction,
                record.transfer_state,
            ) else {
                continue;
            };
            let mut transfer = styrene_ipc::types::AttachmentTransferInfo::default();
            transfer.message_id = record.message_id;
            transfer.transfer_id = transfer_id;
            transfer.resource_hash = record.resource_hash.map(hex::encode);
            transfer.representation = representation;
            transfer.direction = direction;
            transfer.state = state;
            transfer.transferred = record.transferred;
            transfer.total = record.total;
            transfer.checksum_verified = record.checksum_verified;
            transfer.cancellable = transfer.representation == "resource"
                && transfer.direction == "outbound"
                && matches!(transfer.state.as_str(), "queued" | "transferring");
            transfer.error = record.transfer_error;
            events.emit_attachment_transfer(transfer);
        }
        Ok(())
    }

    /// Remove a receipt mapping (e.g., on send failure).
    pub fn remove_receipt(&self, packet_hash: &str) {
        self.receipts.lock().unwrap().mappings.remove(packet_hash);
    }

    // --- Store management ---

    /// Clear all messages (for testing or admin operations).
    pub fn clear_messages(&self) -> Result<(), std::io::Error> {
        self.lock_store()?.clear_messages().map_err(std::io::Error::other)
    }

    /// Prune outbound messages by count, using the given eviction priority.
    pub fn prune_outbound(
        &self,
        count: usize,
        eviction_priority: &str,
    ) -> Result<Vec<String>, std::io::Error> {
        self.store
            .lock()
            .unwrap()
            .prune_outbound_messages(count, eviction_priority)
            .map_err(std::io::Error::other)
    }
}

impl Default for MessagingService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tcp_interface(
        hash: AddressHash,
        generation: u64,
    ) -> rns_core::transport::iface::InterfaceSnapshot {
        rns_core::transport::iface::InterfaceSnapshot {
            hash,
            kind: rns_core::transport::iface::InterfaceKind::TcpClient,
            mode: rns_core::transport::iface::InterfaceMode::PointToPoint,
            state: rns_core::transport::iface::InterfaceState::Connected,
            local_endpoint: None,
            remote_endpoint: None,
            parent: None,
            tx_bytes: 0,
            rx_bytes: 0,
            violations: Default::default(),
            filters: Default::default(),
            connected_peers: 1,
            generation,
        }
    }

    fn path_snapshot(
        destination: AddressHash,
        interface: AddressHash,
        next_hop: AddressHash,
        age: Duration,
        lifetime: Duration,
    ) -> rns_core::transport::core_transport::path_table::PathSnapshot {
        let observed_at = std::time::SystemTime::now().checked_sub(age).unwrap();
        rns_core::transport::core_transport::path_table::PathSnapshot {
            destination,
            hops: 2,
            received_from: next_hop,
            iface: interface,
            age,
            observed_at,
            lifetime,
            expires_at: observed_at + lifetime,
        }
    }

    fn route_test_service(
        store: Arc<Mutex<MessagesStore>>,
    ) -> (MessagingService, Arc<crate::transport::mock_transport::MockTransport>) {
        let service = MessagingService::with_store(store);
        let transport = Arc::new(crate::transport::mock_transport::MockTransport::new_default());
        service.set_signer(
            transport.clone(),
            Arc::new(rns_core::identity::PrivateIdentity::new_from_name("route-observation-local")),
        );
        (service, transport)
    }

    fn make_test_record(id: &str, source: &str, dest: &str) -> MessageRecord {
        MessageRecord {
            id: id.into(),
            source: source.into(),
            destination: dest.into(),
            title: "Test".into(),
            content: "Hello".into(),
            timestamp: 1000,
            direction: "out".into(),
            fields: None,
            receipt_status: None,
            read: false,
        }
    }

    #[tokio::test]
    async fn retry_projection_and_command_share_canonical_wire_requirement() {
        let service = MessagingService::new();
        let message =
            make_test_record("missing-canonical-wire", &"11".repeat(16), &"22".repeat(16));
        service.router.queue(&message, Some("opportunistic"), 1, 1, None).unwrap();
        service.router.finish(&message.id, OutboundState::Failed, "failed").unwrap();

        assert_eq!(
            service.retry_eligibility(&message.id).unwrap(),
            Some((false, Some(MessageRetryIneligibilityReason::CanonicalWireUnavailable)))
        );
        assert_eq!(
            service.retry_message_outcome(&message.id).await.unwrap(),
            RetryMessageOutcome::TerminalConflict("canonical_wire_unavailable".into())
        );
    }

    #[tokio::test]
    async fn attempt_capture_is_immutable_across_tcp_reconnect_and_restart() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("messages.db");
        let store = Arc::new(Mutex::new(MessagesStore::open(&path).unwrap()));
        let (service, transport) = route_test_service(store.clone());
        let destination = AddressHash::new([0x41; 16]);
        let first_interface = AddressHash::new([0x51; 16]);
        transport.set_interface_snapshots(vec![tcp_interface(first_interface, 3)]);
        transport.set_path_snapshot(path_snapshot(
            destination,
            first_interface,
            AddressHash::new([0x61; 16]),
            Duration::from_secs(1),
            Duration::from_secs(60),
        ));

        let message_id = service
            .send_chat_with_method(
                &hex::encode(destination.as_slice()),
                "observed route",
                None,
                Some("opportunistic"),
            )
            .await
            .unwrap();
        let projection = service.outbound_projection(&message_id).unwrap();
        let captured = &projection.attempts[0];
        assert_eq!(
            captured.route.outcome,
            styrene_ipc::types::MessageAttemptRouteOutcome::Observed
        );
        assert_eq!(captured.route.connection_generation, Some(3));
        assert_eq!(captured.route.interface.as_ref().unwrap().kind, "tcp_client");
        assert_eq!(captured.bearer, None);

        let replacement_interface = AddressHash::new([0x52; 16]);
        transport.inject_lifecycle(
            crate::transport::mesh_transport::TransportLifecycleEvent::Reconnected,
        );
        transport.set_interface_snapshots(vec![tcp_interface(replacement_interface, 9)]);
        transport.set_path_snapshot(path_snapshot(
            destination,
            replacement_interface,
            AddressHash::new([0x62; 16]),
            Duration::from_secs(1),
            Duration::from_secs(60),
        ));
        let unchanged = service.outbound_projection(&message_id).unwrap();
        assert_eq!(unchanged.attempts[0], *captured);
        drop(service);
        drop(store);

        let reopened = Arc::new(Mutex::new(MessagesStore::open(&path).unwrap()));
        let restored =
            MessagingService::with_store(reopened).outbound_projection(&message_id).unwrap();
        assert_eq!(restored.attempts[0], *captured);
    }

    #[tokio::test]
    async fn attempt_capture_persists_unknown_without_attributable_interface_snapshot() {
        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let (service, transport) = route_test_service(store);
        let destination = AddressHash::new([0x42; 16]);
        transport.set_path_snapshot(path_snapshot(
            destination,
            AddressHash::new([0x55; 16]),
            AddressHash::new([0x65; 16]),
            Duration::from_secs(1),
            Duration::from_secs(60),
        ));
        transport.set_interface_snapshots(vec![tcp_interface(AddressHash::new([0x56; 16]), 4)]);

        let message_id = service
            .send_chat_with_method(
                &hex::encode(destination.as_slice()),
                "unknown route",
                None,
                Some("opportunistic"),
            )
            .await
            .unwrap();
        let attempt = service.outbound_projection(&message_id).unwrap().attempts.remove(0);
        assert_eq!(attempt.route.outcome, styrene_ipc::types::MessageAttemptRouteOutcome::Unknown);
        assert_eq!(attempt.route.interface, None);
        assert_eq!(attempt.route.connection_generation, None);
        assert_eq!(attempt.bearer, None);
    }

    #[tokio::test]
    async fn retry_captures_new_route_without_rewriting_stale_first_attempt() {
        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let (service, transport) = route_test_service(store);
        let destination = AddressHash::new([0x43; 16]);
        let first_interface = AddressHash::new([0x53; 16]);
        transport.set_interface_snapshots(vec![tcp_interface(first_interface, 1)]);
        transport.set_path_snapshot(path_snapshot(
            destination,
            first_interface,
            AddressHash::new([0x63; 16]),
            Duration::from_secs(120),
            Duration::from_secs(30),
        ));
        transport.queue_send_raw(Err(TransportError::Unavailable));

        let message_id = service
            .send_chat_with_method(
                &hex::encode(destination.as_slice()),
                "retry route",
                None,
                Some("opportunistic"),
            )
            .await
            .unwrap();
        let first = service.outbound_projection(&message_id).unwrap().attempts.remove(0);
        assert!(first.route.stale);
        assert_eq!(first.route.connection_generation, Some(1));

        let second_interface = AddressHash::new([0x54; 16]);
        transport.set_interface_snapshots(vec![tcp_interface(second_interface, 2)]);
        transport.set_path_snapshot(path_snapshot(
            destination,
            second_interface,
            AddressHash::new([0x64; 16]),
            Duration::from_secs(1),
            Duration::from_secs(60),
        ));
        assert!(matches!(
            service.retry_message_outcome(&message_id).await.unwrap(),
            RetryMessageOutcome::Created(_)
        ));

        let attempts = service.outbound_projection(&message_id).unwrap().attempts;
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0], first);
        assert_eq!(attempts[1].route.connection_generation, Some(2));
        assert!(!attempts[1].route.stale);
        assert_ne!(
            attempts[0].route.interface.as_ref().unwrap().id,
            attempts[1].route.interface.as_ref().unwrap().id
        );
        assert!(attempts.iter().all(|attempt| attempt.bearer.is_none()));
    }

    #[test]
    fn insert_and_retrieve_message() {
        let svc = MessagingService::new();
        let record = make_test_record("msg1", "src", "dst");
        svc.accept_inbound_record(&record).unwrap();

        let retrieved = svc.get_message("msg1").unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().content, "Hello");
    }

    #[test]
    fn duplicate_inbound_record_is_not_replaced() {
        let svc = MessagingService::new();
        let mut original = make_test_record("duplicate-id", "src", "dst");
        original.content = "first".into();
        assert!(svc.accept_inbound_record(&original).unwrap());

        let mut replay = original.clone();
        replay.content = "mutated replay".into();
        assert!(!svc.accept_inbound_record(&replay).unwrap());

        let stored = svc.get_message("duplicate-id").unwrap().unwrap();
        assert_eq!(stored.content, "first");
    }

    #[tokio::test]
    async fn cancel_winner_blocks_late_evidence_installation_and_terminal_delete_ghosts() {
        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let message = make_test_record("cancel-delete", &"11".repeat(16), &"22".repeat(16));
        let route = OutboundRouteRecord {
            message_id: message.id.clone(),
            requested_method: "direct".into(),
            actual_method: "direct".into(),
            representation: "packet".into(),
            fallback_reason: None,
            correlation_id: message.id.clone(),
            retry_of: None,
            deadline_unix_ms: i64::MAX,
            state: "queued".into(),
            attempt_count: 0,
        };
        store.lock().unwrap().insert_outbound_message(&message, &route).unwrap();
        let service = MessagingService::with_store(store);

        assert!(matches!(
            service.cancel_outbound_outcome(&message.id).await.unwrap(),
            CancelMessageOutcome::Applied(_)
        ));
        assert_eq!(
            service.cancel_outbound_outcome(&message.id).await.unwrap(),
            CancelMessageOutcome::AlreadyCancelled
        );
        service.track_receipt("late-receipt", &message.id);
        assert_eq!(service.resolve_receipt("late-receipt"), None);
        assert_eq!(
            service.delete_message_outcome(&message.id).unwrap().disposition,
            crate::storage::messages::MutationDisposition::Applied
        );
        assert!(!service.handle_packet_delivery_receipt("late-receipt").unwrap());
        assert!(service.get_message(&message.id).unwrap().is_none());
    }

    #[tokio::test]
    async fn predispatch_cancel_closes_the_dispatch_gate() {
        let operation = OutboundOperation::new();
        assert!(operation.cancel_before_dispatch());
        assert!(matches!(
            operation.begin_dispatch(LinkRepresentation::Packet),
            Err(TransportError::Cancelled)
        ));
        assert_eq!(operation.wait_for_cancel_handoff().await, OutboundOperationPhase::Cancelled);
    }

    #[test]
    fn accepted_resource_cancellation_prevents_packet_fallback() {
        let operation = OutboundOperation::new();
        operation.begin_dispatch(LinkRepresentation::Resource).unwrap();
        operation.request_resource_cancel();

        assert!(matches!(operation.begin_fallback_dispatch(), Err(TransportError::Cancelled)));
        operation.complete_dispatch(&Ok((String::new(), WireRepresentation::Packet)));
        assert_eq!(operation.lock_state().phase, OutboundOperationPhase::Cancelled);
    }

    #[tokio::test]
    async fn resource_cleanup_rejection_does_not_persist_cancelled() {
        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let message = make_test_record("resource-cleanup", &"11".repeat(16), &"22".repeat(16));
        let route = OutboundRouteRecord {
            message_id: message.id.clone(),
            requested_method: "direct".into(),
            actual_method: "direct".into(),
            representation: "resource".into(),
            fallback_reason: None,
            correlation_id: message.id.clone(),
            retry_of: None,
            deadline_unix_ms: i64::MAX,
            state: "sent".into(),
            attempt_count: 1,
        };
        store.lock().unwrap().insert_outbound_message(&message, &route).unwrap();
        let service = MessagingService::with_store(store);
        service
            .transport
            .set(Arc::new(crate::transport::null_transport::NullTransport::new()))
            .ok()
            .expect("transport is unset");
        let operation = Arc::new(OutboundOperation::new());
        operation.lock_state().phase = OutboundOperationPhase::Accepted(
            LinkRepresentation::Resource,
            Some(rns_core::hash::Hash::new([0x33; 32])),
        );
        service.operations.lock().unwrap().insert(message.id.clone(), operation);

        assert_eq!(
            service.cancel_outbound_outcome(&message.id).await.unwrap(),
            CancelMessageOutcome::TerminalConflict("resource_cleanup_rejected".into())
        );
        assert_eq!(service.router.route(&message.id).unwrap().unwrap().state, "sent");
    }

    #[test]
    fn receipt_correlations_are_bounded_and_removed_at_terminal_state() {
        let service = MessagingService::new();
        let message = make_test_record("bounded-correlations", "me", "peer");
        service.accept_inbound_record(&message).unwrap();
        for index in 0..=MAX_RECEIPT_CORRELATIONS {
            service.track_receipt(&format!("receipt-{index}"), &message.id);
        }
        assert_eq!(service.receipts.lock().unwrap().mappings.len(), MAX_RECEIPT_CORRELATIONS);

        assert!(
            service
                .apply_lifecycle_evidence(
                    &message.id,
                    LifecycleEvidence::Failed("terminal test".into()),
                )
                .unwrap()
        );
        assert!(service.receipts.lock().unwrap().mappings.is_empty());
    }

    #[test]
    fn duplicate_wire_delivery_has_structured_outcome() {
        let svc = MessagingService::new();
        let source = [0x11; 16];
        let destination = [0x22; 16];
        let signer = rns_core::identity::PrivateIdentity::new_from_name("inbound-outcome-test");
        let wire = lxmf_bridge::build_wire_message(source, destination, "", "hello", None, &signer)
            .unwrap();

        let accepted_id = match svc.accept_inbound(destination, &wire, InboundPayloadMode::FullWire)
        {
            InboundAcceptOutcome::Accepted(record) => record.id,
            outcome => panic!("expected accepted outcome, got {outcome:?}"),
        };
        assert!(matches!(
            svc.accept_inbound(destination, &wire, InboundPayloadMode::FullWire),
            InboundAcceptOutcome::Duplicate { message_id } if message_id == accepted_id
        ));
    }

    #[test]
    fn duplicate_wire_without_matching_canonical_record_is_storage_failure() {
        let source = [0x31; 16];
        let destination = [0x32; 16];
        let signer =
            rns_core::identity::PrivateIdentity::new_from_name("unverified-duplicate-test");
        let wire = lxmf_bridge::build_wire_message(source, destination, "", "hello", None, &signer)
            .unwrap();
        let decoder = MessagingService::new();
        let record = match decoder.accept_inbound(destination, &wire, InboundPayloadMode::FullWire)
        {
            InboundAcceptOutcome::Accepted(record) => record,
            outcome => panic!("expected accepted outcome, got {outcome:?}"),
        };
        let service = MessagingService::new();
        assert!(service.accept_inbound_record(&record).unwrap());

        assert!(matches!(
            service.accept_inbound(destination, &wire, InboundPayloadMode::FullWire),
            InboundAcceptOutcome::StorageError { message_id, .. } if message_id == record.id
        ));
    }

    #[test]
    fn malformed_wire_delivery_has_structured_outcome() {
        let svc = MessagingService::new();
        assert!(matches!(
            svc.accept_inbound([0x22; 16], b"not-lxmf", InboundPayloadMode::FullWire),
            InboundAcceptOutcome::Rejected { diagnostics } if !diagnostics.attempts.is_empty()
        ));
    }

    #[test]
    fn inbound_field_five_persists_binary_blobs_and_redacts_projection() {
        let svc = MessagingService::new();
        let source = [0x11; 16];
        let destination = [0x22; 16];
        let signer = rns_core::identity::PrivateIdentity::new_from_name("inbound-attachments");
        let wire = lxmf_bridge::build_wire_message(
            source,
            destination,
            "",
            "attachments",
            Some(serde_json::json!({
                "attachments": [
                    {"name": "duplicate.bin", "data": []},
                    {"name": "duplicate.bin", "data": [0, 1, 255]}
                ]
            })),
            &signer,
        )
        .unwrap();
        let resource_hash = [0x44; 32];
        assert!(
            !svc.handle_resource_progress(&resource_hash, wire.len() as u64 / 2, wire.len() as u64)
                .unwrap()
        );
        let id = match svc.accept_inbound_resource_with_identity(
            destination,
            &wire,
            InboundPayloadMode::FullWire,
            None,
            crate::storage::messages::InboundAttachmentTransferEvidence {
                resource_hash,
                transferred: wire.len() as u64,
                total: wire.len() as u64,
                checksum_verified: true,
            },
        ) {
            InboundAcceptOutcome::Accepted(record) => {
                let fields = record.fields.expect("redacted fields").to_string();
                assert!(fields.contains("stored_attachment"));
                assert!(!fields.contains("255"));
                record.id
            }
            outcome => panic!("expected accepted attachment, got {outcome:?}"),
        };
        let attachments = svc.list_attachments(&id).unwrap();
        assert_eq!(attachments.len(), 2);
        assert_eq!(attachments[0].wire_name, attachments[1].wire_name);
        assert_ne!(attachments[0].digest, attachments[1].digest);
        assert_eq!(attachments[0].resource_hash.as_deref(), Some(resource_hash.as_slice()));
        assert_eq!(attachments[0].transferred, wire.len() as u64);
        assert_eq!(attachments[0].total, wire.len() as u64);
        assert!(attachments[0].checksum_verified);
        assert_eq!(attachments[0].byte_len, 0);
        let chunk = svc.query_attachment_chunk(&id, 1, 0, 256).unwrap().unwrap();
        assert_eq!(chunk.data, [0, 1, 255]);
        assert!(chunk.done);
        assert_eq!(svc.canonical_inbound(&id).unwrap().unwrap().wire, wire);
    }

    #[test]
    fn malformed_attachment_preserves_canonical_message_but_exposes_no_blob() {
        let svc = MessagingService::new();
        let signer = rns_core::identity::PrivateIdentity::new_from_name("malformed-attachment");
        let source = [0x31; 16];
        let destination = [0x32; 16];
        let payload = lxmf::Payload::new(
            1_700_000_000.0,
            Some(b"body".to_vec()),
            Some(Vec::new()),
            Some(rmpv::Value::Map(vec![(
                rmpv::Value::from(5),
                rmpv::Value::Array(vec![rmpv::Value::Array(vec![rmpv::Value::from(
                    "missing-data",
                )])]),
            )])),
            None,
        );
        let mut message = lxmf::WireMessage::new(destination, source, payload);
        message.sign(&signer).unwrap();
        let wire = message.pack().unwrap();
        let id = match svc.accept_inbound(destination, &wire, InboundPayloadMode::FullWire) {
            InboundAcceptOutcome::Accepted(record) => record.id,
            outcome => panic!("malformed attachment must preserve message, got {outcome:?}"),
        };
        assert_eq!(svc.canonical_inbound(&id).unwrap().unwrap().wire, wire);
        let attachments = svc.list_attachments(&id).unwrap();
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].integrity, "invalid");
        assert!(svc.query_attachment_chunk(&id, 0, 0, 256).unwrap().is_none());
    }

    #[test]
    fn canonical_inbound_authentication_revalidates_without_changing_content() {
        let svc = MessagingService::new();
        let signer = rns_core::identity::PrivateIdentity::new_from_name("deferred-auth-sender");
        let source: [u8; 16] =
            signer.as_identity().address_hash.as_slice().try_into().expect("identity hash length");
        let destination = [0x42; 16];
        let payload = lxmf::Payload::new(
            1_700_000_000.25,
            Some(vec![0xff, 0x00]),
            Some(vec![0xfe]),
            Some(rmpv::Value::Map(vec![(
                rmpv::Value::from(9),
                rmpv::Value::Binary(vec![0x80, 0x81]),
            )])),
            None,
        );
        let mut wire = lxmf::WireMessage::new(destination, source, payload);
        wire.sign(&signer).expect("sign");
        let wire = wire.pack().expect("pack");

        let id = match svc.accept_inbound(destination, &wire, InboundPayloadMode::FullWire) {
            InboundAcceptOutcome::Accepted(record) => record.id,
            outcome => panic!("expected acceptance, got {outcome:?}"),
        };
        let before = svc.canonical_inbound(&id).unwrap().unwrap();
        assert_eq!(before.authentication_state, "unknown_identity");
        assert_eq!(before.timestamp, 1_700_000_000.25);
        assert_eq!(before.content, vec![0xff, 0x00]);

        assert_eq!(svc.revalidate_unknown_identity(source, signer.as_identity()).unwrap(), 1);
        let after = svc.canonical_inbound(&id).unwrap().unwrap();
        assert_eq!(after.authentication_state, "verified");
        assert_eq!(after.content, before.content);
        assert_eq!(after.title, before.title);
        assert_eq!(after.fields_msgpack, before.fields_msgpack);
        assert_eq!(after.wire, before.wire);
    }

    #[test]
    fn list_messages_with_pagination() {
        let svc = MessagingService::new();
        for i in 0..5 {
            let mut record = make_test_record(&format!("msg{i}"), "src", "dst");
            record.timestamp = 1000 + i;
            svc.accept_inbound_record(&record).unwrap();
        }

        let messages = svc.list_messages(3, None).unwrap();
        assert_eq!(messages.len(), 3);
    }

    #[test]
    fn count_message_buckets() {
        let svc = MessagingService::new();
        // count_message_buckets returns (queued_count, in_flight_count)
        // based on receipt_status. No receipt_status = queued.
        let record = make_test_record("msg1", "src", "dst");
        svc.accept_inbound_record(&record).unwrap();

        let (queued, in_flight) = svc.count_messages().unwrap();
        assert_eq!(queued, 1); // no receipt_status = queued
        assert_eq!(in_flight, 0);
    }

    #[test]
    fn receipt_tracking_roundtrip() {
        let svc = MessagingService::new();
        svc.accept_inbound_record(&make_test_record("msg_123", "me", "peer")).unwrap();
        svc.track_receipt("pkt_abc", "msg_123");

        assert_eq!(svc.resolve_receipt("pkt_abc"), Some("msg_123".into()));
        assert_eq!(svc.resolve_receipt("unknown"), None);
    }

    #[test]
    fn untracked_packet_receipt_is_rejected_and_not_buffered() {
        let svc = MessagingService::new();
        let record = make_test_record("msg1", "me", "peer");
        svc.accept_inbound_record(&record).unwrap();

        assert!(!svc.handle_packet_delivery_receipt("pkt_hash").unwrap());
        svc.track_receipt("pkt_hash", "msg1");
        assert_eq!(svc.get_message("msg1").unwrap().unwrap().receipt_status, None);

        assert!(svc.handle_packet_delivery_receipt("pkt_hash").unwrap());
        let msg = svc.get_message("msg1").unwrap().unwrap();
        assert_eq!(msg.receipt_status, Some("delivered: packet-receipt".into()));
    }

    #[test]
    fn completion_evidence_is_exactly_correlated_and_representation_checked() {
        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        for (id, representation) in
            [("message-a", "packet"), ("message-b", "packet"), ("message-resource", "resource")]
        {
            let message = make_test_record(id, "local", "remote");
            store
                .lock()
                .unwrap()
                .insert_outbound_message(
                    &message,
                    &OutboundRouteRecord {
                        message_id: id.into(),
                        requested_method: "direct".into(),
                        actual_method: "direct".into(),
                        representation: representation.into(),
                        fallback_reason: None,
                        correlation_id: id.into(),
                        retry_of: None,
                        deadline_unix_ms: i64::MAX,
                        state: "sent".into(),
                        attempt_count: 1,
                    },
                )
                .unwrap();
            store
                .lock()
                .unwrap()
                .begin_outbound_attempt(&OutboundAttemptRecord {
                    message_id: id.into(),
                    attempt_number: 1,
                    started_unix_ms: 1,
                    deadline_unix_ms: i64::MAX,
                    state: "sent".into(),
                    route_observation: None,
                })
                .unwrap();
        }
        let service = MessagingService::with_store(store);
        let packet_a = "aa".repeat(32);
        let packet_b = "bb".repeat(32);
        let resource = "cc".repeat(32);
        service.track_receipt(&packet_a, "message-a");
        service.track_receipt(&packet_b, "message-b");
        service.track_receipt(&resource, "message-resource");

        assert!(!service.handle_packet_delivery_receipt("forged").unwrap());
        assert!(service.handle_packet_delivery_receipt(&packet_a).unwrap());
        assert_eq!(service.outbound_lifecycle("message-a").unwrap().unwrap().0.state, "delivered");
        assert_eq!(service.outbound_lifecycle("message-b").unwrap().unwrap().0.state, "queued");
        assert!(!service.handle_resource_complete(&packet_b).unwrap());
        assert_eq!(service.outbound_lifecycle("message-b").unwrap().unwrap().0.state, "queued");
        assert!(!service.handle_packet_delivery_receipt(&resource).unwrap());
        assert!(service.handle_resource_complete(&resource).unwrap());
        assert_eq!(
            service.outbound_lifecycle("message-resource").unwrap().unwrap().0.state,
            "delivered"
        );
        assert!(!service.handle_packet_delivery_receipt(&packet_a).unwrap());
    }

    #[test]
    fn route_missing_lifecycle_evidence_is_concurrent_and_poison_safe() {
        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let service = Arc::new(MessagingService::with_store(store.clone()));
        service
            .accept_inbound_record(&make_test_record("route-missing", "local", "remote"))
            .unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let apply = || {
            let service = service.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                service.apply_lifecycle_evidence("route-missing", LifecycleEvidence::Cancelled)
            })
        };
        let first = apply();
        let second = apply();
        barrier.wait();
        assert!(first.join().unwrap().unwrap());
        assert!(second.join().unwrap().unwrap());
        assert_eq!(
            service.get_message("route-missing").unwrap().unwrap().receipt_status.as_deref(),
            Some("cancelled")
        );

        let poisoned_store = store.clone();
        assert!(
            std::thread::spawn(move || {
                let _guard = poisoned_store.lock().expect("messaging test store lock");
                panic!("poison messaging store");
            })
            .join()
            .is_err()
        );
        let error = service
            .apply_lifecycle_evidence("route-missing", LifecycleEvidence::Cancelled)
            .expect_err("poisoned store must return an operational error");
        assert!(error.to_string().contains("store lock poisoned"));
    }

    #[test]
    fn concurrent_service_instances_return_the_persisted_retry_winner() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("messages.sqlite");
        let original = make_test_record("original", "local", "11");
        {
            let store = MessagesStore::open(&path).unwrap();
            store
                .insert_outbound_message(
                    &original,
                    &OutboundRouteRecord {
                        message_id: original.id.clone(),
                        requested_method: "direct".into(),
                        actual_method: "direct".into(),
                        representation: "packet".into(),
                        fallback_reason: None,
                        correlation_id: original.id.clone(),
                        retry_of: None,
                        deadline_unix_ms: i64::MAX,
                        state: "failed".into(),
                        attempt_count: 1,
                    },
                )
                .unwrap();
        }
        let first = Arc::new(MessagingService::with_store(Arc::new(Mutex::new(
            MessagesStore::open(&path).unwrap(),
        ))));
        let second = Arc::new(MessagingService::with_store(Arc::new(Mutex::new(
            MessagesStore::open(&path).unwrap(),
        ))));
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let run = |service: Arc<MessagingService>, id: &'static str| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let retry = make_test_record(id, "local", "11");
                barrier.wait();
                service
                    .router
                    .queue_retry(&retry, Some("direct"), 1, 1, "original", "original")
                    .unwrap()
            })
        };
        let first = run(first, "retry-a");
        let second = run(second, "retry-b");
        barrier.wait();
        let outcomes = [first.join().unwrap(), second.join().unwrap()];

        let queued = outcomes
            .iter()
            .find_map(|outcome| match outcome {
                RetryQueueResult::Queued(_) => Some(()),
                RetryQueueResult::Existing(_) => None,
            })
            .is_some();
        let existing = outcomes.iter().find_map(|outcome| match outcome {
            RetryQueueResult::Existing(route) => Some(route.message_id.as_str()),
            RetryQueueResult::Queued(_) => None,
        });
        assert!(queued);
        let winner =
            MessagesStore::open(&path).unwrap().outbound_retry_for("original").unwrap().unwrap();
        assert_eq!(existing, Some(winner.message_id.as_str()));
    }

    #[test]
    fn remove_receipt_clears_mapping() {
        let svc = MessagingService::new();
        svc.track_receipt("pkt", "msg");
        svc.remove_receipt("pkt");
        assert!(svc.resolve_receipt("pkt").is_none());
    }

    #[test]
    fn peer_mutations_validate_and_canonicalize_identity_hashes() {
        let service = MessagingService::new();
        let uppercase = "AB".repeat(16);
        let canonical = uppercase.to_ascii_lowercase();

        let contact = service.set_contact(&uppercase, Some("Peer"), None).unwrap();
        assert_eq!(contact.peer_hash, canonical);
        service.set_conversation_pinned(&uppercase, true).unwrap();
        service.set_conversation_muted(&uppercase, true).unwrap();
        assert_eq!(service.set_draft(&uppercase, "draft").unwrap().peer_hash, canonical);

        for invalid in ["peer".to_string(), "ab".to_string(), "gg".repeat(16), "ab".repeat(17)] {
            assert_eq!(
                service.set_contact(&invalid, None, None).unwrap_err().kind(),
                std::io::ErrorKind::InvalidInput
            );
            assert_eq!(
                service.set_draft(&invalid, "draft").unwrap_err().kind(),
                std::io::ErrorKind::InvalidInput
            );
        }
    }

    #[test]
    fn clear_messages_empties_store() {
        let svc = MessagingService::new();
        let record = make_test_record("msg1", "src", "dst");
        svc.accept_inbound_record(&record).unwrap();
        svc.clear_messages().unwrap();

        assert!(svc.get_message("msg1").unwrap().is_none());
    }

    #[tokio::test]
    async fn send_chat_without_transport_returns_error() {
        let svc = MessagingService::new();
        let result = svc.send_chat("abcdef0123456789", "hello", None).await;
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("transport not available"),
            "should fail when no transport"
        );
    }

    #[tokio::test]
    async fn outbound_lxmf_uses_delivery_destination_as_source() {
        use crate::transport::mock_transport::{MockCall, MockTransport};

        let svc = MessagingService::new();
        let transport = Arc::new(MockTransport::new(
            AddressHash::new([0x11; 16]),
            AddressHash::new([0x22; 16]),
        ));
        let signer = Arc::new(rns_core::identity::PrivateIdentity::new_from_name(
            "outbound-delivery-source",
        ));
        svc.set_signer(transport.clone(), signer);

        let message_id = svc
            .send_chat_with_method(&"33".repeat(16), "body", None, Some("opportunistic"))
            .await
            .expect("send opportunistic message");
        let stripped = transport
            .calls()
            .into_iter()
            .find_map(|call| match call {
                MockCall::SendRaw { data, .. } => Some(data),
                _ => None,
            })
            .expect("opportunistic wire send");
        let mut wire = vec![0x33; 16];
        wire.extend_from_slice(&stripped);
        let decoded = lxmf::inbound_decode::decode_inbound_message(
            [0x33; 16],
            &wire,
            InboundPayloadMode::FullWire,
        )
        .expect("decode outbound wire");

        assert_eq!(decoded.source, [0x22; 16]);
        assert_eq!(svc.get_message(&message_id).unwrap().unwrap().source, "22".repeat(16));
    }

    #[tokio::test]
    async fn direct_failure_falls_back_with_same_persisted_message_and_structured_fields() {
        use crate::transport::mock_transport::{MockCall, MockTransport};

        let service = MessagingService::new();
        let transport = Arc::new(MockTransport::new_default());
        let remote = rns_core::identity::PrivateIdentity::new_from_name("fallback-remote");
        transport.queue_resolve(Some(*remote.as_identity()));
        transport.queue_send_link(Err(TransportError::SendFailed("link rejected".into())));
        let signer = Arc::new(rns_core::identity::PrivateIdentity::new_from_name("fallback-local"));
        service.set_signer(transport.clone(), signer);

        let message_id = service
            .send_chat_with_fields(
                &"33".repeat(16),
                "echo body",
                Some("[auto-reply]"),
                serde_json::json!({"styrene_echo": {"response": true, "request_id": "request"}}),
            )
            .await
            .unwrap();
        let route = service.outbound_lifecycle(&message_id).unwrap().unwrap().0;
        assert_eq!(route.requested_method, "direct");
        assert_eq!(route.actual_method, "opportunistic");
        assert_eq!(route.correlation_id, message_id);
        assert!(
            route.fallback_reason.as_deref().is_some_and(|reason| reason.contains("link rejected"))
        );
        assert_eq!(route.attempt_count, 1);
        assert!(transport.calls().iter().any(|call| matches!(call, MockCall::SendRaw { .. })));
        assert_eq!(
            service.get_message(&message_id).unwrap().unwrap().fields,
            Some(serde_json::json!({"styrene_echo": {"response": true, "request_id": "request"}}))
        );
    }

    #[tokio::test]
    async fn outbound_request_places_attachment_binary_in_signed_wire_and_store() {
        use crate::transport::mock_transport::{MockCall, MockTransport};

        let svc = MessagingService::new();
        let transport = Arc::new(MockTransport::new_default());
        let signer =
            Arc::new(rns_core::identity::PrivateIdentity::new_from_name("outbound-attachment"));
        svc.set_signer(transport.clone(), signer);
        let message_id = svc
            .send_chat_with_attachments(
                &"22".repeat(16),
                "body",
                None,
                Some("opportunistic"),
                &[AttachmentBlobInput {
                    wire_name: "vector.bin".into(),
                    data: vec![0, 1, 255],
                    content_type: Some("application/octet-stream".into()),
                    source: "local".into(),
                }],
            )
            .await
            .expect("send attachment");
        let stripped = transport
            .calls()
            .into_iter()
            .find_map(|call| match call {
                MockCall::SendRaw { data, .. } => Some(data),
                _ => None,
            })
            .expect("opportunistic wire send");
        let mut wire = hex::decode("22".repeat(16)).unwrap();
        wire.extend_from_slice(&stripped);
        let decoded = lxmf::inbound_decode::decode_inbound_message(
            [0x22; 16],
            &wire,
            InboundPayloadMode::FullWire,
        )
        .unwrap();
        let attachments = lxmf::attachments::parse_attachment_field(decoded.fields.as_ref())
            .expect("canonical field 5");
        assert_eq!(attachments[0].data, [0, 1, 255]);
        assert_eq!(
            attachments[0].source,
            lxmf::attachments::AttachmentFieldSource::CanonicalBinary
        );
        assert_eq!(svc.list_attachments(&message_id).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn retry_preserves_attachment_name_order_digest_bytes_and_blob_deduplication() {
        use crate::transport::mock_transport::{MockCall, MockTransport};

        let svc = MessagingService::new();
        let transport = Arc::new(MockTransport::new_default());
        let signer =
            Arc::new(rns_core::identity::PrivateIdentity::new_from_name("retry-attachment"));
        svc.set_signer(transport.clone(), signer);
        let original = svc
            .send_chat_with_attachments(
                &"33".repeat(16),
                "retry body",
                None,
                Some("opportunistic"),
                &[
                    AttachmentBlobInput {
                        wire_name: "same.bin".into(),
                        data: vec![1, 2, 3],
                        content_type: None,
                        source: "local".into(),
                    },
                    AttachmentBlobInput {
                        wire_name: "same.bin".into(),
                        data: vec![4, 5],
                        content_type: None,
                        source: "local".into(),
                    },
                ],
            )
            .await
            .unwrap();
        assert!(
            svc.apply_lifecycle_evidence(&original, LifecycleEvidence::Failed("retry-test".into()))
                .unwrap()
        );
        let retry = match svc.retry_message_outcome(&original).await.unwrap() {
            RetryMessageOutcome::Created(id) => id,
            outcome => panic!("expected retry creation, got {outcome:?}"),
        };
        assert_eq!(retry, original);
        let original_relations = svc.list_attachments(&original).unwrap();
        let retry_relations = svc.list_attachments(&retry).unwrap();
        assert_eq!(original_relations.len(), 2);
        assert_eq!(retry_relations.len(), 2);
        for (left, right) in original_relations.iter().zip(&retry_relations) {
            assert_eq!(left.ordinal, right.ordinal);
            assert_eq!(left.wire_name, right.wire_name);
            assert_eq!(left.digest, right.digest);
            let left_bytes =
                svc.query_attachment_chunk(&original, left.ordinal, 0, 256).unwrap().unwrap().data;
            let right_bytes =
                svc.query_attachment_chunk(&retry, right.ordinal, 0, 256).unwrap().unwrap().data;
            assert_eq!(left_bytes, right_bytes);
        }
        assert_eq!(svc.attachment_blob_usage().unwrap().0, 2);

        let wires = transport
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                MockCall::SendRaw { data, .. } => Some(data),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(wires.len(), 2);
        assert_eq!(wires[0], wires[1]);
        let parsed = wires
            .into_iter()
            .map(|stripped| {
                let mut wire = hex::decode("33".repeat(16)).unwrap();
                wire.extend_from_slice(&stripped);
                let decoded = lxmf::inbound_decode::decode_inbound_message(
                    [0x33; 16],
                    &wire,
                    InboundPayloadMode::FullWire,
                )
                .unwrap();
                lxmf::attachments::parse_attachment_field(decoded.fields.as_ref()).unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(parsed[0], parsed[1]);
        let (_, attempts) = svc.outbound_lifecycle(&original).unwrap().unwrap();
        assert_eq!(
            attempts.iter().map(|attempt| attempt.attempt_number).collect::<Vec<_>>(),
            [1, 2]
        );
    }

    fn succeeded_request(value: rmpv::Value) -> styrene_ipc::types::RequestObservationInfo {
        let mut encoded = Vec::new();
        rmpv::encode::write_value(&mut encoded, &value).unwrap();
        let mut receipt = styrene_ipc::types::RequestObservationInfo::default();
        receipt.request_id = "55".repeat(16);
        receipt.state = styrene_ipc::types::RequestState::Succeeded;
        receipt.response = Some(encoded);
        receipt
    }

    fn persist_preparing_propagated_job(
        path: &std::path::Path,
        name: &str,
    ) -> (
        String,
        Arc<rns_core::identity::PrivateIdentity>,
        rns_core::identity::PrivateIdentity,
        rns_core::identity::PrivateIdentity,
    ) {
        use crate::services::router::RouterCoordinator;
        use crate::storage::standard_propagation::StandardPropagationPeer;
        use crate::transport::mock_transport::MockTransport;

        let store = Arc::new(Mutex::new(MessagesStore::open(path).unwrap()));
        let local =
            Arc::new(rns_core::identity::PrivateIdentity::new_from_name(&format!("{name}-local")));
        let recipient =
            rns_core::identity::PrivateIdentity::new_from_name(&format!("{name}-recipient"));
        let peer = rns_core::identity::PrivateIdentity::new_from_name(&format!("{name}-peer"));
        let recipient_destination = rns_core::destination::SingleOutputDestination::new(
            *recipient.as_identity(),
            DestinationName::new("lxmf", "delivery"),
        )
        .desc
        .address_hash;
        let propagation_destination = rns_core::destination::SingleOutputDestination::new(
            *peer.as_identity(),
            DestinationName::new("lxmf", "propagation"),
        )
        .desc
        .address_hash;
        let mut peer_hash = [0u8; 16];
        peer_hash.copy_from_slice(peer.address_hash().as_slice());
        store
            .lock()
            .unwrap()
            .standard_propagation_upsert_peer(&StandardPropagationPeer {
                identity_hash: peer_hash,
                propagation_destination: Some(
                    propagation_destination.as_slice().try_into().unwrap(),
                ),
                configured: true,
                enabled: true,
                transfer_limit_kb: Some(256),
                sync_limit_kb: Some(4000),
                stamp_cost: Some(0),
                stamp_flexibility: Some(0),
                peering_cost: Some(0),
                observed_at: 1,
            })
            .unwrap();
        store
            .lock()
            .unwrap()
            .standard_propagation_set_selection(Some(peer_hash), "manual", 1)
            .unwrap();
        let mut source = [0u8; 16];
        source.copy_from_slice(local.address_hash().as_slice());
        let mut destination = [0u8; 16];
        destination.copy_from_slice(recipient_destination.as_slice());
        let canonical_wire =
            crate::lxmf_bridge::build_wire_message(source, destination, "", name, None, &local)
                .unwrap();
        let message_id = lxmf::inbound_decode::outbound_message_id_hex(&canonical_wire).unwrap();
        let coordinator = crate::standard_propagation::StandardPropagationCoordinator::new(
            Arc::new(MockTransport::new_default()),
            store.clone(),
            local.clone(),
        );
        let preparing = coordinator.prepare_outbound(&message_id, &canonical_wire, 1).unwrap();
        let router = RouterCoordinator::new(store);
        router
            .queue_propagated_with_ticket_offer_and_attachments(
                &MessageRecord {
                    id: message_id.clone(),
                    source: hex::encode(source),
                    destination: hex::encode(destination),
                    title: String::new(),
                    content: name.into(),
                    timestamp: 1,
                    direction: "out".into(),
                    fields: None,
                    receipt_status: Some("queued".into()),
                    read: true,
                },
                Some("propagated"),
                canonical_wire.len(),
                canonical_wire.len().saturating_sub(16),
                None,
                None,
                &[],
                &preparing,
                &canonical_wire,
            )
            .unwrap();
        (message_id, local, recipient, peer)
    }

    #[tokio::test]
    async fn propagated_send_spools_exact_ciphertext_and_requires_exact_transfer_proof() {
        use crate::storage::standard_propagation::StandardPropagationPeer;
        use crate::transport::mock_transport::{MockCall, MockTransport};
        use rns_core::transport::delivery::LinkSendResult;

        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let service = MessagingService::with_store(store.clone());
        let transport = Arc::new(MockTransport::new_default());
        let local = Arc::new(rns_core::identity::PrivateIdentity::new_from_name("prop-local"));
        let recipient = rns_core::identity::PrivateIdentity::new_from_name("prop-recipient");
        let recipient_destination = rns_core::destination::SingleOutputDestination::new(
            *recipient.as_identity(),
            DestinationName::new("lxmf", "delivery"),
        )
        .desc
        .address_hash;
        let peer = rns_core::identity::PrivateIdentity::new_from_name("prop-peer");
        let propagation_destination = rns_core::destination::SingleOutputDestination::new(
            *peer.as_identity(),
            DestinationName::new("lxmf", "propagation"),
        )
        .desc
        .address_hash;
        let mut peer_hash = [0u8; 16];
        peer_hash.copy_from_slice(peer.address_hash().as_slice());
        store
            .lock()
            .unwrap()
            .standard_propagation_upsert_peer(&StandardPropagationPeer {
                identity_hash: peer_hash,
                propagation_destination: Some(
                    propagation_destination.as_slice().try_into().unwrap(),
                ),
                configured: true,
                enabled: true,
                transfer_limit_kb: Some(256),
                sync_limit_kb: Some(4000),
                stamp_cost: Some(0),
                stamp_flexibility: Some(0),
                peering_cost: Some(0),
                observed_at: 1,
            })
            .unwrap();
        store
            .lock()
            .unwrap()
            .standard_propagation_set_selection(Some(peer_hash), "manual", 1)
            .unwrap();
        transport.queue_resolve(Some(*recipient.as_identity()));
        transport.queue_resolve(Some(*peer.as_identity()));
        let link_id = AddressHash::new([0x77; 16]);
        transport.queue_open_link(Ok(link_id));
        transport.queue_request(Ok(succeeded_request(rmpv::Value::Boolean(true))));
        let resource_hash = rns_core::hash::Hash::new([9; 32]);
        transport.queue_send_link(Ok(LinkSendResult::Resource(resource_hash)));
        transport.queue_close(Ok(()));
        service.set_signer(transport.clone(), local);
        let persisted_store = store.clone();
        let persistence_transport = transport.clone();
        let persistence_check = tokio::spawn(async move {
            persistence_transport
                .wait_for_calls(1, |call| matches!(call, MockCall::RequestPath { .. }))
                .await;
            let store = persisted_store.lock().unwrap();
            let message_id = store.list_messages(1, None).unwrap()[0].id.clone();
            assert!(store.outbound_route(&message_id).unwrap().is_some());
            assert!(store.standard_propagation_client_job(&message_id).unwrap().is_some());
        });
        let resource_transport = transport.clone();
        tokio::spawn(async move {
            resource_transport
                .wait_for_calls(1, |call| matches!(call, MockCall::SendOnLink { .. }))
                .await;
            resource_transport.inject_resource(rns_core::transport::resource::ResourceEvent {
                hash: resource_hash,
                link_id,
                kind: rns_core::transport::resource::ResourceEventKind::OutboundComplete,
            });
        });

        let message_id = service
            .send_chat_with_method(
                &hex::encode(recipient_destination.as_slice()),
                "propagated",
                None,
                Some("propagated"),
            )
            .await
            .unwrap();
        persistence_check.await.unwrap();
        let job =
            store.lock().unwrap().standard_propagation_client_job(&message_id).unwrap().unwrap();
        assert_eq!(job.state, "accepted");
        assert_eq!(
            lxmf::propagation::transient_id(job.lxmf_data.as_deref().unwrap()),
            job.transient_id.unwrap()
        );
        assert_eq!(
            store.lock().unwrap().get_message(&message_id).unwrap().unwrap().receipt_status,
            Some("sent: propagated".into())
        );
        assert!(transport.calls().iter().any(|call| {
            matches!(call, MockCall::SendOnLink { link_id: actual, .. } if *actual == link_id)
        }));
        assert_eq!(
            transport
                .calls()
                .iter()
                .filter(|call| matches!(call, MockCall::StartRequest { .. }))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn paper_export_returns_uri_only_in_committed_outcome_and_marks_sent() {
        use crate::transport::mock_transport::MockTransport;

        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let service = MessagingService::with_store(store.clone());
        let transport = Arc::new(MockTransport::new_default());
        let local = Arc::new(rns_core::identity::PrivateIdentity::new_from_name("paper-local"));
        let recipient = rns_core::identity::PrivateIdentity::new_from_name("paper-recipient");
        let destination = rns_core::destination::SingleOutputDestination::new(
            *recipient.as_identity(),
            DestinationName::new("lxmf", "delivery"),
        )
        .desc
        .address_hash;
        transport.queue_resolve(Some(*recipient.as_identity()));
        service.set_signer(transport, local);

        let outcome = service
            .send_chat_outcome_with_attachments(
                &hex::encode(destination.as_slice()),
                "paper content",
                Some("paper title"),
                Some("paper"),
                &[],
            )
            .await
            .unwrap();

        assert_eq!(outcome.disposition, SendCommitDisposition::PaperExported);
        assert_eq!(outcome.message.id, outcome.message_id);
        assert!(outcome.paper_uri.as_deref().is_some_and(|uri| uri.starts_with("lxm://")));
        assert!(!format!("{outcome:?}").contains("lxm://"));
        let persisted = store.lock().unwrap().get_message(&outcome.message_id).unwrap().unwrap();
        assert_eq!(persisted.receipt_status.as_deref(), Some("sent: paper export"));
        assert!(!persisted.content.contains("lxm://"));
    }

    #[tokio::test]
    async fn paper_identity_failure_returns_persisted_failed_outcome() {
        use crate::transport::mock_transport::MockTransport;

        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let service = MessagingService::with_store(store.clone());
        let transport = Arc::new(MockTransport::new_default());
        let local = Arc::new(rns_core::identity::PrivateIdentity::new_from_name("paper-fail"));
        service.set_signer(transport, local);

        let outcome = service
            .send_chat_outcome_with_attachments(
                &"44".repeat(16),
                "paper content",
                None,
                Some("paper"),
                &[],
            )
            .await
            .unwrap();

        assert_eq!(outcome.disposition, SendCommitDisposition::Failed);
        assert!(!outcome.message_id.is_empty());
        assert_eq!(outcome.message.id, outcome.message_id);
        assert!(outcome.paper_uri.is_none());
        assert!(store.lock().unwrap().get_message(&outcome.message_id).unwrap().is_some());
    }

    #[tokio::test]
    async fn paper_recipient_mismatch_commits_failure_without_uri_disclosure() {
        use crate::transport::mock_transport::MockTransport;

        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let service = MessagingService::with_store(store.clone());
        let transport = Arc::new(MockTransport::new_default());
        let local = Arc::new(rns_core::identity::PrivateIdentity::new_from_name("paper-local"));
        let wrong_recipient =
            rns_core::identity::PrivateIdentity::new_from_name("paper-wrong-recipient");
        transport.queue_resolve(Some(*wrong_recipient.as_identity()));
        service.set_signer(transport, local);

        let outcome = service
            .send_chat_outcome_with_attachments(
                &"45".repeat(16),
                "paper mismatch sentinel",
                None,
                Some("paper"),
                &[],
            )
            .await
            .unwrap();

        assert_eq!(outcome.disposition, SendCommitDisposition::Failed);
        assert!(!outcome.message_id.is_empty());
        assert_eq!(outcome.message.id, outcome.message_id);
        assert!(outcome.paper_uri.is_none());
        assert!(
            outcome
                .terminal_error
                .as_deref()
                .is_some_and(|error| error.contains("does not match requested destination"))
        );
        assert!(!format!("{outcome:?}").contains("lxm://"));
        assert!(store.lock().unwrap().get_message(&outcome.message_id).unwrap().is_some());
    }

    #[tokio::test]
    async fn injected_post_commit_failure_is_total_and_terminalized_best_effort() {
        use crate::transport::mock_transport::MockTransport;

        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let service = MessagingService::with_store(store.clone());
        service.set_signer(
            Arc::new(MockTransport::new_default()),
            Arc::new(rns_core::identity::PrivateIdentity::new_from_name("post-commit")),
        );
        service.inject_post_commit_failure(false);

        let outcome = service
            .send_chat_outcome_with_attachments(
                &"46".repeat(16),
                "post commit",
                None,
                Some("direct"),
                &[],
            )
            .await
            .unwrap();

        assert_eq!(outcome.disposition, SendCommitDisposition::Failed);
        assert!(!outcome.message_id.is_empty());
        assert_eq!(outcome.message.id, outcome.message_id);
        assert_eq!(outcome.terminal_error.as_deref(), Some("injected post-commit failure"));
        assert!(
            store
                .lock()
                .unwrap()
                .get_message(&outcome.message_id)
                .unwrap()
                .unwrap()
                .receipt_status
                .as_deref()
                .is_some_and(|status| status.starts_with("failed"))
        );
    }

    #[tokio::test]
    async fn poisoned_projection_after_commit_still_returns_failed_id() {
        use crate::transport::mock_transport::MockTransport;

        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let service = MessagingService::with_store(store);
        service.set_signer(
            Arc::new(MockTransport::new_default()),
            Arc::new(rns_core::identity::PrivateIdentity::new_from_name("post-commit-poison")),
        );
        service.inject_post_commit_failure(true);

        let outcome = service
            .send_chat_outcome_with_attachments(
                &"47".repeat(16),
                "post commit poison",
                None,
                Some("direct"),
                &[],
            )
            .await
            .unwrap();

        assert_eq!(outcome.disposition, SendCommitDisposition::Failed);
        assert!(!outcome.message_id.is_empty());
        assert_eq!(outcome.message.id, outcome.message_id);
        assert!(
            outcome
                .terminal_error
                .as_deref()
                .is_some_and(|error| error.contains("injected post-commit failure")
                    && error.contains("projection freshness"))
        );
    }

    #[tokio::test]
    async fn paper_mdu_failure_is_persisted_without_uri() {
        use crate::transport::mock_transport::MockTransport;

        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let service = MessagingService::with_store(store.clone());
        let transport = Arc::new(MockTransport::new_default());
        let local = Arc::new(rns_core::identity::PrivateIdentity::new_from_name("paper-mdu-local"));
        let recipient = rns_core::identity::PrivateIdentity::new_from_name("paper-mdu-peer");
        let destination = rns_core::destination::SingleOutputDestination::new(
            *recipient.as_identity(),
            DestinationName::new("lxmf", "delivery"),
        )
        .desc
        .address_hash;
        transport.queue_resolve(Some(*recipient.as_identity()));
        service.set_signer(transport, local);

        let outcome = service
            .send_chat_outcome_with_attachments(
                &hex::encode(destination.as_slice()),
                &"x".repeat(lxmf::PAPER_MDU),
                None,
                Some("paper"),
                &[],
            )
            .await
            .unwrap();

        assert_eq!(outcome.disposition, SendCommitDisposition::Failed);
        assert!(outcome.paper_uri.is_none());
        assert!(outcome.terminal_error.as_deref().is_some_and(|error| error.contains("paper MDU")));
        assert!(store.lock().unwrap().get_message(&outcome.message_id).unwrap().is_some());
    }

    #[tokio::test]
    async fn propagated_preparing_insert_failure_rolls_back_base_queue_before_network() {
        use crate::storage::standard_propagation::StandardPropagationPeer;
        use crate::transport::mock_transport::MockTransport;

        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let service = MessagingService::with_store(store.clone());
        let transport = Arc::new(MockTransport::new_default());
        let local = Arc::new(rns_core::identity::PrivateIdentity::new_from_name("rollback-local"));
        let recipient = rns_core::identity::PrivateIdentity::new_from_name("rollback-recipient");
        let recipient_destination = rns_core::destination::SingleOutputDestination::new(
            *recipient.as_identity(),
            DestinationName::new("lxmf", "delivery"),
        )
        .desc
        .address_hash;
        let peer = rns_core::identity::PrivateIdentity::new_from_name("rollback-peer");
        let propagation_destination = rns_core::destination::SingleOutputDestination::new(
            *peer.as_identity(),
            DestinationName::new("lxmf", "propagation"),
        )
        .desc
        .address_hash;
        let mut peer_hash = [0u8; 16];
        peer_hash.copy_from_slice(peer.address_hash().as_slice());
        {
            let mut store = store.lock().unwrap();
            store
                .standard_propagation_upsert_peer(&StandardPropagationPeer {
                    identity_hash: peer_hash,
                    propagation_destination: Some(
                        propagation_destination.as_slice().try_into().unwrap(),
                    ),
                    configured: true,
                    enabled: true,
                    transfer_limit_kb: Some(256),
                    sync_limit_kb: Some(4000),
                    stamp_cost: Some(0),
                    stamp_flexibility: Some(0),
                    peering_cost: Some(0),
                    observed_at: 1,
                })
                .unwrap();
            store.standard_propagation_set_selection(Some(peer_hash), "manual", 1).unwrap();
            store.standard_propagation_fail_job_insert_for_test().unwrap();
        }
        service.set_signer(transport.clone(), local);

        assert!(
            service
                .send_chat_with_method(
                    &hex::encode(recipient_destination.as_slice()),
                    "rollback",
                    None,
                    Some("propagated"),
                )
                .await
                .is_err()
        );
        assert!(store.lock().unwrap().list_messages(16, None).unwrap().is_empty());
        assert!(transport.calls().is_empty());
    }

    #[tokio::test]
    async fn reopened_preparing_job_materializes_and_uploads() {
        use crate::transport::mock_transport::{MockCall, MockTransport};
        use rns_core::transport::delivery::LinkSendResult;

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("preparing-recovery.db");
        let (message_id, local, recipient, peer) =
            persist_preparing_propagated_job(&path, "preparing-recovery");
        let store = Arc::new(Mutex::new(MessagesStore::open(&path).unwrap()));
        assert_eq!(
            store
                .lock()
                .unwrap()
                .standard_propagation_client_job(&message_id)
                .unwrap()
                .unwrap()
                .state,
            "preparing"
        );
        let service = MessagingService::with_store(store.clone());
        let transport = Arc::new(MockTransport::new_default());
        transport.queue_resolve(Some(*recipient.as_identity()));
        transport.queue_resolve(Some(*peer.as_identity()));
        let link_id = AddressHash::new([0x61; 16]);
        let resource_hash = rns_core::hash::Hash::new([0x62; 32]);
        transport.queue_open_link(Ok(link_id));
        transport.queue_request(Ok(succeeded_request(rmpv::Value::Boolean(true))));
        transport.queue_send_link(Ok(LinkSendResult::Resource(resource_hash)));
        transport.queue_close(Ok(()));
        service.set_signer(transport.clone(), local);
        let proof_transport = transport.clone();
        tokio::spawn(async move {
            proof_transport
                .wait_for_calls(1, |call| matches!(call, MockCall::SendOnLink { .. }))
                .await;
            proof_transport.inject_resource(rns_core::transport::resource::ResourceEvent {
                hash: resource_hash,
                link_id,
                kind: rns_core::transport::resource::ResourceEventKind::OutboundComplete,
            });
        });

        assert_eq!(
            service
                .resume_standard_propagation_outbound_once(CancellationToken::new())
                .await
                .unwrap(),
            1
        );
        let job =
            store.lock().unwrap().standard_propagation_client_job(&message_id).unwrap().unwrap();
        assert_eq!(job.state, "accepted");
        assert!(job.canonical_wire.is_none());
        assert!(job.lxmf_data.is_some());
        assert_eq!(service.router.route(&message_id).unwrap().unwrap().state, "sent");
    }

    #[tokio::test]
    async fn reopened_spooled_job_reuses_exact_ciphertext_and_stamp() {
        use crate::transport::mock_transport::{MockCall, MockTransport};
        use rns_core::transport::delivery::LinkSendResult;

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("spooled-recovery.db");
        let (message_id, local, recipient, peer) =
            persist_preparing_propagated_job(&path, "spooled-recovery");
        {
            let store = Arc::new(Mutex::new(MessagesStore::open(&path).unwrap()));
            let coordinator = crate::standard_propagation::StandardPropagationCoordinator::new(
                Arc::new(MockTransport::new_default()),
                store,
                local.clone(),
            );
            coordinator
                .materialize_outbound(
                    &message_id,
                    recipient.as_identity(),
                    2,
                    std::time::Instant::now() + Duration::from_secs(5),
                    &CancellationToken::new(),
                )
                .unwrap();
        }
        let store = Arc::new(Mutex::new(MessagesStore::open(&path).unwrap()));
        store
            .lock()
            .unwrap()
            .standard_propagation_reconcile_startup(
                3,
                crate::storage::standard_propagation::StandardPropagationPolicy {
                    queue_max_count: 4096,
                    queue_max_bytes: 16 * 1024 * 1024,
                    expiry_secs: 30 * 24 * 60 * 60,
                },
            )
            .unwrap();
        let before =
            store.lock().unwrap().standard_propagation_client_job(&message_id).unwrap().unwrap();
        let transport = Arc::new(MockTransport::new_default());
        transport.queue_resolve(Some(*peer.as_identity()));
        let link_id = AddressHash::new([0x71; 16]);
        let resource_hash = rns_core::hash::Hash::new([0x72; 32]);
        transport.queue_open_link(Ok(link_id));
        transport.queue_request(Ok(succeeded_request(rmpv::Value::Boolean(true))));
        transport.queue_send_link(Ok(LinkSendResult::Resource(resource_hash)));
        transport.queue_close(Ok(()));
        let service = MessagingService::with_store(store.clone());
        service.set_signer(transport.clone(), local);
        let proof_transport = transport.clone();
        tokio::spawn(async move {
            proof_transport
                .wait_for_calls(1, |call| matches!(call, MockCall::SendOnLink { .. }))
                .await;
            proof_transport.inject_resource(rns_core::transport::resource::ResourceEvent {
                hash: resource_hash,
                link_id,
                kind: rns_core::transport::resource::ResourceEventKind::OutboundComplete,
            });
        });
        service.resume_standard_propagation_outbound_once(CancellationToken::new()).await.unwrap();
        let after =
            store.lock().unwrap().standard_propagation_client_job(&message_id).unwrap().unwrap();
        assert_eq!(after.transient_id, before.transient_id);
        assert_eq!(after.lxmf_data, before.lxmf_data);
        assert_eq!(after.stamp, before.stamp);
        assert_eq!(after.correlation_id, before.correlation_id);
        assert_ne!(after.attempt_id, before.attempt_id);
    }

    #[tokio::test]
    async fn cancelled_preparing_job_is_not_resumed_after_reopen() {
        use crate::services::router::RouterCoordinator;
        use crate::transport::mock_transport::MockTransport;

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("cancelled-preparing.db");
        let (message_id, local, _, _) =
            persist_preparing_propagated_job(&path, "cancelled-preparing");
        {
            let store = Arc::new(Mutex::new(MessagesStore::open(&path).unwrap()));
            let router = RouterCoordinator::new(store);
            assert!(router.apply_evidence(&message_id, LifecycleEvidence::Cancelled).unwrap());
        }
        let store = Arc::new(Mutex::new(MessagesStore::open(&path).unwrap()));
        let service = MessagingService::with_store(store);
        let transport = Arc::new(MockTransport::new_default());
        service.set_signer(transport.clone(), local);
        assert_eq!(
            service
                .resume_standard_propagation_outbound_once(CancellationToken::new())
                .await
                .unwrap(),
            0
        );
        assert!(transport.calls().is_empty());
    }

    #[tokio::test]
    async fn propagated_send_without_selection_is_explicitly_failed_and_never_networked() {
        use crate::transport::mock_transport::{MockCall, MockTransport};

        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let service = MessagingService::with_store(store.clone());
        let transport = Arc::new(MockTransport::new_default());
        service.set_signer(
            transport.clone(),
            Arc::new(rns_core::identity::PrivateIdentity::new_from_name("no-selection")),
        );
        let error = service
            .send_chat_with_method(&"33".repeat(16), "no peer", None, Some("propagated"))
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("no selected compatible standard LXMF propagation peer")
        );
        assert!(store.lock().unwrap().list_messages(1, None).unwrap().is_empty());
        assert!(!transport.calls().iter().any(|call| {
            matches!(call, MockCall::OpenLink { .. } | MockCall::SendOnLink { .. })
        }));
    }
}
