use lxmf::propagation::{decode_transfer_envelope, propagated_destination};
use lxmf::propagation_announce::{AnnounceError, StandardPropagationAnnounce};
use lxmf::stamps::{validate_peering_key, validate_propagation_stamp};
use rand_core::{OsRng, RngCore};
use rns_core::destination::{
    DestinationDesc, DestinationName, IngressContext, IngressHandler, IngressRegistrationError,
    RequestAccess, RequestRegistrationError, SingleInputDestination, SingleOutputDestination,
};
use rns_core::hash::AddressHash;
use rns_core::identity::{Identity, PrivateIdentity};
use rns_core::packet::Packet;
use rns_core::transport::core_transport::{
    DestinationRegistrationError, SendPacketOutcome, Transport,
};
use rns_core::transport::resource::MAX_UNSOLICITED_RESOURCE_SIZE;
use rns_core::transport::resource::ResourceEventKind;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::services::EventService;
use crate::storage::messages::MessagesStore;
use crate::storage::standard_propagation::{
    StandardPropagationAttemptStatus, StandardPropagationClientJob, StandardPropagationGetRequest,
    StandardPropagationIngestOutcome, StandardPropagationIngestRequest, StandardPropagationItem,
    StandardPropagationOfferRequest, StandardPropagationPolicy, StandardPropagationProtocolStatus,
    StandardPropagationSelection, StandardPropagationStats,
};
use crate::transport::mesh_transport::{
    DispatchGate, LinkOpenResult, LinkRepresentation, MeshTransport, TransportError,
    TransportLifecycleEvent,
};

pub const DEFAULT_PROPAGATION_NODE_NAME: &str = "Styrene propagation node";
pub const OFFER_PATH: &str = "/offer";
pub const GET_PATH: &str = "/get";
pub const ERROR_NO_IDENTITY: u64 = 0xf0;
pub const ERROR_NO_ACCESS: u64 = 0xf1;
pub const ERROR_INVALID_KEY: u64 = 0xf3;
pub const ERROR_INVALID_DATA: u64 = 0xf4;
pub const ERROR_INVALID_STAMP: u64 = 0xf5;
pub const ERROR_THROTTLED: u64 = 0xf6;

const MAX_OFFER_BYTES: usize = 64 * 1024;
const MAX_OFFER_IDS: usize = 1024;
const MAX_TRANSFER_MESSAGES: usize = 1024;
const MAX_GET_REQUEST_BYTES: usize = 64 * 1024;
const MAX_GET_IDS: usize = 1024;
const MAX_GET_RESPONSE_BYTES: usize = MAX_UNSOLICITED_RESOURCE_SIZE;
const DECIMAL_KB: usize = 1000;
const DEFAULT_EXPIRY_SECS: i64 = 30 * 24 * 60 * 60;
const DEFAULT_THROTTLE_SECS: i64 = 180;

#[derive(Clone, Copy)]
struct PropagationPolicy {
    target_cost: u32,
    flexibility: u32,
    peering_cost: u32,
    transfer_limit_kb: usize,
    sync_limit_kb: usize,
    queue_max_count: usize,
    queue_max_bytes: usize,
    expiry_secs: i64,
    throttle_secs: i64,
    max_offer_links: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StandardPropagationRuntimePolicy {
    pub target_cost: u32,
    pub flexibility: u32,
    pub peering_cost: u32,
    pub transfer_limit_kb: usize,
    pub sync_limit_kb: usize,
    pub queue_max_count: usize,
    pub queue_max_bytes: usize,
    pub expiry_secs: i64,
    pub throttle_secs: i64,
    pub max_offer_links: usize,
}

#[derive(Clone)]
pub struct StandardPropagationRuntimeObservation {
    active: Arc<AtomicBool>,
    registered: bool,
    policy: StandardPropagationRuntimePolicy,
}

impl StandardPropagationRuntimeObservation {
    pub fn registered(policy: StandardPropagationRuntimePolicy) -> Self {
        Self { active: Arc::new(AtomicBool::new(false)), registered: true, policy }
    }

    pub fn client() -> Self {
        let policy = PropagationPolicy::default();
        Self {
            active: Arc::new(AtomicBool::new(true)),
            registered: false,
            policy: StandardPropagationRuntimePolicy {
                target_cost: policy.target_cost,
                flexibility: policy.flexibility,
                peering_cost: policy.peering_cost,
                transfer_limit_kb: policy.transfer_limit_kb,
                sync_limit_kb: policy.sync_limit_kb,
                queue_max_count: policy.queue_max_count,
                queue_max_bytes: policy.queue_max_bytes,
                expiry_secs: policy.expiry_secs,
                throttle_secs: policy.throttle_secs,
                max_offer_links: policy.max_offer_links,
            },
        }
    }

    pub fn active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    pub fn is_registered(&self) -> bool {
        self.registered
    }

    pub fn policy(&self) -> StandardPropagationRuntimePolicy {
        self.policy
    }

    fn set_active(&self, active: bool) {
        self.active.store(active, Ordering::Release);
    }
}

impl Default for PropagationPolicy {
    fn default() -> Self {
        Self {
            target_cost: 16,
            flexibility: 3,
            peering_cost: 18,
            transfer_limit_kb: 256,
            sync_limit_kb: 4000,
            queue_max_count: 4096,
            queue_max_bytes: 16 * 1024 * 1024,
            expiry_secs: DEFAULT_EXPIRY_SECS,
            throttle_secs: DEFAULT_THROTTLE_SECS,
            max_offer_links: 3,
        }
    }
}

impl PropagationPolicy {
    fn sync_limit_bytes(self) -> usize {
        self.sync_limit_kb.saturating_mul(DECIMAL_KB)
    }

    fn storage(self) -> StandardPropagationPolicy {
        StandardPropagationPolicy {
            queue_max_count: self.queue_max_count,
            queue_max_bytes: self.queue_max_bytes,
            expiry_secs: self.expiry_secs,
        }
    }
}

trait PropagationClock: Send + Sync {
    fn now(&self) -> i64;
}

struct SystemPropagationClock;

impl PropagationClock for SystemPropagationClock {
    fn now(&self) -> i64 {
        unix_time().unwrap_or(0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StandardPropagationState {
    HandlersReady,
    Active,
}

#[cfg(test)]
type QueueSnapshot = Vec<([u8; 32], Vec<u8>, [u8; 32])>;

#[derive(Clone)]
struct PendingOffer {
    remote_identity: [u8; 16],
    attempt_id: [u8; 16],
    deadline: i64,
    ids: BTreeSet<[u8; 32]>,
}

#[derive(Default)]
struct PropagationState {
    pending: BTreeMap<AddressHash, PendingOffer>,
    validated_links: BTreeMap<AddressHash, [u8; 16]>,
    throttled: BTreeMap<[u8; 16], i64>,
    link_throttled: BTreeMap<AddressHash, i64>,
}

impl PropagationState {
    fn pending_count(&self) -> usize {
        self.pending.values().map(|pending| pending.ids.len()).sum()
    }

    fn expire(&mut self, now: i64, policy: PropagationPolicy) {
        let _ = policy;
        self.pending.retain(|_, pending| now < pending.deadline);
        self.throttled.retain(|_, until| now < *until);
        self.link_throttled.retain(|_, until| now < *until);
    }
}

struct IngressWorker {
    lifecycle: JoinHandle<()>,
}

impl IngressWorker {
    fn abort(&self) {
        self.lifecycle.abort();
    }

    async fn shutdown(&mut self) {
        self.abort();
        let _ = (&mut self.lifecycle).await;
    }
}

/// Client-side coordinator for the standard MessagePack LXMF propagation exchange.
pub struct StandardPropagationCoordinator {
    transport: Arc<dyn MeshTransport>,
    store: Arc<StdMutex<MessagesStore>>,
    identity: Arc<PrivateIdentity>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StandardPropagationUploadAcceptance {
    AlreadyAccepted,
    AlreadyPresent,
    PacketProof([u8; 32]),
    ResourceProof([u8; 32]),
}

impl StandardPropagationCoordinator {
    pub fn new(
        transport: Arc<dyn MeshTransport>,
        store: Arc<StdMutex<MessagesStore>>,
        identity: Arc<PrivateIdentity>,
    ) -> Self {
        Self { transport, store, identity }
    }

    fn selected_peer(
        &self,
    ) -> Result<crate::storage::standard_propagation::StandardPropagationPeer, TransportError> {
        self.store
            .lock()
            .map_err(|_| TransportError::SendFailed("standard propagation store poisoned".into()))?
            .standard_propagation_selected_peer()
            .map_err(|error| TransportError::SendFailed(format!("propagation selection: {error}")))?
            .ok_or_else(|| {
                TransportError::SendFailed(
                    "no selected compatible standard LXMF propagation peer".into(),
                )
            })
    }

    pub fn prepare_outbound(
        &self,
        message_id: &str,
        canonical_wire: &[u8],
        now: i64,
    ) -> Result<StandardPropagationClientJob, TransportError> {
        let wire = lxmf::WireMessage::unpack(canonical_wire).map_err(|error| {
            TransportError::SendFailed(format!("canonical outbound LXMF wire: {error}"))
        })?;
        if lxmf::inbound_decode::outbound_message_id_hex(canonical_wire).as_deref()
            != Some(message_id)
        {
            return Err(TransportError::SendFailed(
                "canonical outbound LXMF message ID mismatch".into(),
            ));
        }
        let peer = self.selected_peer()?;
        let propagation_destination = peer.propagation_destination.ok_or_else(|| {
            TransportError::SendFailed("selected propagation peer has no destination".into())
        })?;
        let attempt_digest = Sha256::digest(
            [
                b"styrene-standard-propagation-attempt-v1".as_slice(),
                message_id.as_bytes(),
                peer.identity_hash.as_slice(),
                propagation_destination.as_slice(),
            ]
            .concat(),
        );
        let mut attempt_id = [0u8; 16];
        attempt_id.copy_from_slice(&attempt_digest[..16]);
        Ok(StandardPropagationClientJob {
            message_id: message_id.to_string(),
            transient_id: None,
            destination: wire.destination,
            canonical_wire: Some(canonical_wire.to_vec()),
            lxmf_data: None,
            stamp: None,
            peer: peer.identity_hash,
            propagation_destination,
            stamp_cost: peer.stamp_cost.unwrap_or(0),
            peering_cost: peer.peering_cost.unwrap_or(0),
            correlation_id: attempt_id,
            attempt_id,
            state: "preparing".into(),
            created_at: now.max(0),
            updated_at: now.max(0),
        })
    }

    pub fn materialize_outbound(
        &self,
        message_id: &str,
        recipient: &Identity,
        now: i64,
        deadline: std::time::Instant,
        cancellation: &CancellationToken,
    ) -> Result<StandardPropagationClientJob, TransportError> {
        let preparing = self
            .store
            .lock()
            .map_err(|_| TransportError::SendFailed("standard propagation store poisoned".into()))?
            .standard_propagation_client_job(message_id)
            .map_err(|error| {
                TransportError::SendFailed(format!("propagation preparation: {error}"))
            })?
            .ok_or_else(|| {
                TransportError::SendFailed("propagation preparation is missing".into())
            })?;
        if preparing.state != "preparing" {
            return if matches!(preparing.state.as_str(), "spooled" | "uploading" | "accepted") {
                Ok(preparing)
            } else {
                Err(TransportError::SendFailed(
                    "propagation preparation is not materializable".into(),
                ))
            };
        }
        let canonical_wire = preparing.canonical_wire.as_deref().ok_or_else(|| {
            TransportError::SendFailed("propagation preparation has no canonical wire".into())
        })?;
        let wire = lxmf::WireMessage::unpack(canonical_wire).map_err(|error| {
            TransportError::SendFailed(format!("persisted canonical outbound LXMF wire: {error}"))
        })?;
        if wire.destination != preparing.destination
            || lxmf::inbound_decode::outbound_message_id_hex(canonical_wire).as_deref()
                != Some(preparing.message_id.as_str())
        {
            return Err(TransportError::SendFailed(
                "persisted propagation preparation failed canonical validation".into(),
            ));
        }
        let recipient_destination =
            SingleOutputDestination::new(*recipient, DestinationName::new("lxmf", "delivery"))
                .desc
                .address_hash;
        if recipient_destination.as_slice() != preparing.destination {
            return Err(TransportError::SendFailed(
                "resolved recipient identity does not match persisted destination".into(),
            ));
        }
        let (envelope, transient_id) = wire
            .pack_propagation_with_options_and_rng(
                recipient,
                preparing.created_at as f64,
                None,
                OsRng,
            )
            .map_err(|error| TransportError::SendFailed(format!("propagation pack: {error}")))?;
        let payloads = decode_transfer_envelope(&envelope, 4 * 1024 * 1024, 1, 4 * 1024 * 1024)
            .map_err(|error| {
                TransportError::SendFailed(format!("propagation envelope: {error:?}"))
            })?;
        let lxmf_data = payloads
            .into_iter()
            .next()
            .ok_or_else(|| TransportError::SendFailed("empty propagation envelope".into()))?;
        let stamp = lxmf::stamps::generate_material_stamp_with_control(
            &transient_id,
            preparing.stamp_cost,
            lxmf::stamps::PROPAGATION_NODE_WORKBLOCK_ROUNDS,
            usize::MAX,
            || cancellation.is_cancelled() || std::time::Instant::now() >= deadline,
        )
        .map_err(|error| TransportError::SendFailed(format!("propagation stamp: {error:?}")))?;
        let materialized = StandardPropagationClientJob {
            message_id: preparing.message_id,
            transient_id: Some(transient_id),
            destination: preparing.destination,
            canonical_wire: None,
            lxmf_data: Some(lxmf_data),
            stamp: Some(stamp),
            peer: preparing.peer,
            propagation_destination: preparing.propagation_destination,
            stamp_cost: preparing.stamp_cost,
            peering_cost: preparing.peering_cost,
            correlation_id: preparing.correlation_id,
            attempt_id: preparing.attempt_id,
            state: "spooled".into(),
            created_at: preparing.created_at,
            updated_at: now.max(0),
        };
        self.store
            .lock()
            .map_err(|_| TransportError::SendFailed("standard propagation store poisoned".into()))?
            .standard_propagation_spool_outbound(&materialized)
            .map_err(|error| {
                TransportError::SendFailed(format!("propagation materialization: {error}"))
            })?;
        Ok(materialized)
    }

    async fn request(
        &self,
        link_id: AddressHash,
        path: &str,
        data: Vec<u8>,
        correlation: String,
        deadline: std::time::Instant,
        cancellation: &CancellationToken,
    ) -> Result<Vec<u8>, TransportError> {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(TransportError::TimedOut);
        }
        let mut request = styrene_ipc::types::StartRequestInfo::default();
        request.link_id = hex::encode(link_id.as_slice());
        request.path = path.into();
        request.data = data;
        request.timeout_ms = remaining.as_millis().min(u64::MAX as u128) as u64;
        request.max_response_size = MAX_GET_RESPONSE_BYTES as u64;
        request.correlation_id = Some(correlation);
        let mut receipt = tokio::select! {
            biased;
            () = cancellation.cancelled() => return Err(TransportError::Cancelled),
            result = self.transport.start_request(request) => result?,
        };
        loop {
            match receipt.state {
                styrene_ipc::types::RequestState::Succeeded => {
                    return receipt.response.ok_or_else(|| {
                        TransportError::SendFailed(
                            "propagation request succeeded without response".into(),
                        )
                    });
                }
                state if state.is_terminal() => {
                    return Err(TransportError::SendFailed(format!(
                        "propagation request terminated: {state:?}"
                    )));
                }
                _ => {}
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                let _ = self.transport.cancel_request(&receipt.request_id).await;
                return Err(TransportError::TimedOut);
            }
            tokio::select! {
                biased;
                () = cancellation.cancelled() => {
                    let _ = self.transport.cancel_request(&receipt.request_id).await;
                    return Err(TransportError::Cancelled);
                }
                () = tokio::time::sleep(remaining.min(std::time::Duration::from_millis(25))) => {}
            }
            receipt =
                self.transport.request_receipt(&receipt.request_id).await?.ok_or_else(|| {
                    TransportError::SendFailed("propagation request disappeared".into())
                })?;
        }
    }

    fn offer_request(
        &self,
        job: &StandardPropagationClientJob,
        deadline: std::time::Instant,
        cancellation: &CancellationToken,
    ) -> Result<Vec<u8>, TransportError> {
        let mut local = [0u8; 16];
        local.copy_from_slice(self.identity.address_hash().as_slice());
        let mut material = [0u8; 32];
        material[..16].copy_from_slice(&job.peer);
        material[16..].copy_from_slice(&local);
        let key = lxmf::stamps::generate_material_stamp_with_control(
            &material,
            job.peering_cost,
            lxmf::stamps::PEERING_WORKBLOCK_ROUNDS,
            usize::MAX,
            || cancellation.is_cancelled() || std::time::Instant::now() >= deadline,
        )
        .map_err(|error| {
            TransportError::SendFailed(format!("propagation peering key: {error:?}"))
        })?;
        let transient_id = job.transient_id.ok_or_else(|| {
            TransportError::SendFailed("propagation upload has no transient ID".into())
        })?;
        Ok(encode_value(rmpv::Value::Array(vec![
            rmpv::Value::Binary(key.to_vec()),
            rmpv::Value::Array(vec![rmpv::Value::Binary(transient_id.to_vec())]),
        ])))
    }

    pub async fn upload(
        &self,
        message_id: &str,
        deadline: std::time::Instant,
        cancellation: CancellationToken,
        dispatch_gate: Option<DispatchGate>,
    ) -> Result<StandardPropagationUploadAcceptance, TransportError> {
        let job = self
            .store
            .lock()
            .map_err(|_| TransportError::SendFailed("standard propagation store poisoned".into()))?
            .standard_propagation_client_job(message_id)
            .map_err(|error| TransportError::SendFailed(format!("propagation spool: {error}")))?
            .ok_or_else(|| TransportError::SendFailed("propagation spool is missing".into()))?;
        if job.state == "accepted" {
            return Ok(StandardPropagationUploadAcceptance::AlreadyAccepted);
        }
        if !matches!(job.state.as_str(), "spooled" | "uploading") {
            return Err(TransportError::SendFailed(
                "propagation job must be materialized before upload".into(),
            ));
        }
        let transient_id = job.transient_id.ok_or_else(|| {
            TransportError::SendFailed("materialized propagation job has no transient ID".into())
        })?;
        let lxmf_data = job.lxmf_data.as_deref().ok_or_else(|| {
            TransportError::SendFailed("materialized propagation job has no ciphertext".into())
        })?;
        let stamp = job.stamp.ok_or_else(|| {
            TransportError::SendFailed("materialized propagation job has no stamp".into())
        })?;
        self.transport.request_path(&AddressHash::new(job.propagation_destination)).await;
        let peer_identity = self
            .transport
            .resolve_identity(&AddressHash::new(job.propagation_destination))
            .await
            .ok_or_else(|| {
                TransportError::SendFailed("selected propagation identity unavailable".into())
            })?;
        if peer_identity.address_hash.as_slice() != job.peer {
            return Err(TransportError::SendFailed(
                "selected propagation destination identity mismatch".into(),
            ));
        }
        let open = self
            .transport
            .open_named_link(
                DestinationDesc {
                    identity: peer_identity,
                    address_hash: AddressHash::new(job.propagation_destination),
                    name: DestinationName::new("lxmf", "propagation"),
                },
                cancellation.clone(),
                deadline.saturating_duration_since(std::time::Instant::now()),
            )
            .await?;
        let (link_id, owned) = match open {
            LinkOpenResult::Created(id) => (id, true),
            LinkOpenResult::Reused(id) => (id, false),
        };
        let result = async {
            self.transport
                .identify_link(&hex::encode(link_id.as_slice()), &self.identity)
                .await?;
            let offer = self.offer_request(&job, deadline, &cancellation)?;
            if let Some(dispatch_gate) = &dispatch_gate {
                dispatch_gate(LinkRepresentation::Packet)?;
            }
            let response = self
                .request(
                    link_id,
                    OFFER_PATH,
                    offer,
                    hex::encode(job.attempt_id),
                    deadline,
                    &cancellation,
                )
                .await?;
            let wanted = decode_exact(&response).ok_or_else(|| {
                TransportError::SendFailed("malformed propagation offer response".into())
            })?;
            let wanted = match wanted {
                rmpv::Value::Boolean(value) => value,
                rmpv::Value::Array(values)
                    if values.iter().any(|value| {
                        matches!(value, rmpv::Value::Binary(bytes) if bytes.as_slice() == transient_id)
                    }) =>
                {
                    true
                }
                _ => {
                    return Err(TransportError::SendFailed(
                        "invalid propagation offer subset".into(),
                    ));
                }
            };
            if wanted {
                let mut stamped = lxmf_data.to_vec();
                stamped.extend_from_slice(&stamp);
                let transfer = encode_value(rmpv::Value::Array(vec![
                    rmpv::Value::F64(job.created_at as f64),
                    rmpv::Value::Array(vec![rmpv::Value::Binary(stamped)]),
                ]));
                let mut resources = self.transport.subscribe_resources();
                let mut packet_receipts = self.transport.subscribe_packet_receipts();
                match self.transport.send_on_link(&link_id, &transfer).await? {
                    rns_core::transport::delivery::LinkSendResult::Packet(packet) => {
                        let expected = packet.hash().to_bytes();
                        loop {
                            let observed = tokio::select! {
                                biased;
                                result = packet_receipts.recv() => result.map_err(|_| {
                                    TransportError::SendFailed("packet receipt observation closed".into())
                                })?,
                                () = cancellation.cancelled() => return Err(TransportError::Cancelled),
                                () = tokio::time::sleep_until(deadline.into()) => {
                                    return Err(TransportError::TimedOut);
                                }
                            };
                            if observed == expected {
                                break Ok(StandardPropagationUploadAcceptance::PacketProof(expected));
                            }
                        }
                    }
                    rns_core::transport::delivery::LinkSendResult::Resource(hash) => loop {
                        let event = tokio::select! {
                            biased;
                            result = resources.recv() => result.map_err(|_| {
                                TransportError::SendFailed("resource observation closed".into())
                            })?,
                            () = cancellation.cancelled() => return Err(TransportError::Cancelled),
                            () = tokio::time::sleep_until(deadline.into()) => {
                                return Err(TransportError::TimedOut);
                            }
                        };
                        if event.hash != hash || event.link_id != link_id {
                            continue;
                        }
                        match event.kind {
                            ResourceEventKind::OutboundComplete => {
                                break Ok(StandardPropagationUploadAcceptance::ResourceProof(
                                    hash.to_bytes(),
                                ));
                            }
                            ResourceEventKind::Failed(reason) => {
                                return Err(TransportError::SendFailed(format!(
                                    "propagation resource failed: {reason:?}"
                                )));
                            }
                            ResourceEventKind::Progress(_) | ResourceEventKind::Complete(_) => {}
                        }
                    },
                }
            } else {
                Ok(StandardPropagationUploadAcceptance::AlreadyPresent)
            }
        }
        .await;
        if result.is_err() {
            let _ = self.store.lock().map(|mut store| {
                store.standard_propagation_record_attempt_failure(
                    job.attempt_id,
                    job.peer,
                    "upload_failed",
                    None,
                    unix_time().unwrap_or(0),
                )
            });
        }
        if owned && let Err(error) = self.transport.close_link(&link_id).await {
            crate::daemon_diagnostic!(
                "[standard-propagation] owned upload link cleanup failed: {error}"
            );
        }
        result
    }

    pub async fn sync_once(
        &self,
        messaging: &crate::services::MessagingService,
        deadline: std::time::Instant,
        cancellation: CancellationToken,
    ) -> Result<usize, TransportError> {
        let peer = self.selected_peer()?;
        let propagation_destination = peer.propagation_destination.ok_or_else(|| {
            TransportError::SendFailed("selected propagation peer has no destination".into())
        })?;
        let destination_hash = AddressHash::new(propagation_destination);
        self.transport.request_path(&destination_hash).await;
        let peer_identity =
            self.transport.resolve_identity(&destination_hash).await.ok_or_else(|| {
                TransportError::SendFailed("selected propagation identity unavailable".into())
            })?;
        if peer_identity.address_hash.as_slice() != peer.identity_hash {
            return Err(TransportError::SendFailed(
                "selected propagation destination identity mismatch".into(),
            ));
        }
        let open = self
            .transport
            .open_named_link(
                DestinationDesc {
                    identity: peer_identity,
                    address_hash: destination_hash,
                    name: DestinationName::new("lxmf", "propagation"),
                },
                cancellation.clone(),
                deadline.saturating_duration_since(std::time::Instant::now()),
            )
            .await?;
        let (link_id, owned) = match open {
            LinkOpenResult::Created(id) => (id, true),
            LinkOpenResult::Reused(id) => (id, false),
        };
        let mut active_attempt = None;
        let result = async {
            self.transport.identify_link(&hex::encode(link_id.as_slice()), &self.identity).await?;
            let pending = self
                .store
                .lock()
                .map_err(|_| {
                    TransportError::SendFailed("standard propagation store poisoned".into())
                })?
                .standard_propagation_pending_haves(peer.identity_hash, MAX_GET_IDS)
                .map_err(|error| TransportError::SendFailed(format!("pending haves: {error}")))?;
            if !pending.is_empty() {
                let attempt_id = random_attempt_id();
                active_attempt = Some(attempt_id);
                let now = unix_time().unwrap_or(0);
                let attempt_deadline = now.saturating_add(
                    i64::try_from(
                        deadline.saturating_duration_since(std::time::Instant::now()).as_secs(),
                    )
                    .unwrap_or(i64::MAX),
                );
                self.store
                    .lock()
                    .map_err(|_| {
                        TransportError::SendFailed("standard propagation store poisoned".into())
                    })?
                    .standard_propagation_begin_client_attempt(
                        attempt_id,
                        peer.identity_hash,
                        crate::storage::standard_propagation::StandardPropagationGetOperation::Sync,
                        &pending,
                        "accepted",
                        now,
                        attempt_deadline,
                    )
                    .map_err(|error| {
                        TransportError::SendFailed(format!("resume haves attempt: {error}"))
                    })?;
                self.request(
                    link_id,
                    GET_PATH,
                    get_request(None, Some(&pending), None),
                    hex::encode(attempt_id),
                    deadline,
                    &cancellation,
                )
                .await?;
                self.store
                    .lock()
                    .map_err(|_| {
                        TransportError::SendFailed("standard propagation store poisoned".into())
                    })?
                    .standard_propagation_mark_haves_acknowledged(
                        peer.identity_hash,
                        &pending,
                        attempt_id,
                        unix_time().unwrap_or(0),
                    )
                    .map_err(|error| {
                        TransportError::SendFailed(format!("acknowledge haves: {error}"))
                    })?;
                self.store
                    .lock()
                    .map_err(|_| {
                        TransportError::SendFailed("standard propagation store poisoned".into())
                    })?
                    .standard_propagation_complete_client_attempt(
                        attempt_id,
                        peer.identity_hash,
                        &pending,
                        "accepted",
                        0,
                        unix_time().unwrap_or(0),
                    )
                    .map_err(|error| {
                        TransportError::SendFailed(format!("complete resumed haves: {error}"))
                    })?;
            }
            let inventory_attempt = random_attempt_id();
            active_attempt = Some(inventory_attempt);
            let inventory_now = unix_time().unwrap_or(0);
            self.store
                .lock()
                .map_err(|_| {
                    TransportError::SendFailed("standard propagation store poisoned".into())
                })?
                .standard_propagation_begin_client_attempt(
                    inventory_attempt,
                    peer.identity_hash,
                    crate::storage::standard_propagation::StandardPropagationGetOperation::Fetch,
                    &[],
                    "inventory",
                    inventory_now,
                    inventory_now.saturating_add(32),
                )
                .map_err(|error| {
                    TransportError::SendFailed(format!("inventory attempt: {error}"))
                })?;
            let inventory_response = self
                .request(
                    link_id,
                    GET_PATH,
                    get_request(None, None, None),
                    hex::encode(inventory_attempt),
                    deadline,
                    &cancellation,
                )
                .await?;
            let inventory = decode_binary_ids(&inventory_response, MAX_GET_IDS)?;
            self.store
                .lock()
                .map_err(|_| {
                    TransportError::SendFailed("standard propagation store poisoned".into())
                })?
                .standard_propagation_complete_client_attempt(
                    inventory_attempt,
                    peer.identity_hash,
                    &inventory,
                    "inventory",
                    0,
                    unix_time().unwrap_or(0),
                )
                .map_err(|error| {
                    TransportError::SendFailed(format!("complete inventory: {error}"))
                })?;
            active_attempt = None;
            if inventory.is_empty() {
                return Ok(0);
            }
            let attempt_id = random_attempt_id();
            active_attempt = Some(attempt_id);
            let download_now = unix_time().unwrap_or(0);
            self.store
                .lock()
                .map_err(|_| {
                    TransportError::SendFailed("standard propagation store poisoned".into())
                })?
                .standard_propagation_begin_client_attempt(
                    attempt_id,
                    peer.identity_hash,
                    crate::storage::standard_propagation::StandardPropagationGetOperation::Download,
                    &inventory,
                    "offered",
                    download_now,
                    download_now.saturating_add(32),
                )
                .map_err(|error| {
                    TransportError::SendFailed(format!("download attempt: {error}"))
                })?;
            let response = self
                .request(
                    link_id,
                    GET_PATH,
                    get_request(
                        Some(&inventory),
                        None,
                        Some(download_limit_kb(peer.sync_limit_kb)),
                    ),
                    hex::encode(attempt_id),
                    deadline,
                    &cancellation,
                )
                .await?;
            let payloads = decode_binary_payloads(&response, MAX_GET_IDS, MAX_GET_RESPONSE_BYTES)?;
            let wanted: BTreeSet<_> = inventory.iter().copied().collect();
            let local_destination = self.transport.destination_hash();
            let mut acknowledged = Vec::new();
            for lxmf_data in payloads {
                let (transient_id, full_wire, wire) =
                    decrypt_fetched_wire(&self.identity, local_destination, &wanted, &lxmf_data)?;
                match messaging.accept_propagated_inbound(
                    wire.destination,
                    &full_wire,
                    transient_id,
                    attempt_id,
                    peer.identity_hash,
                ) {
                    crate::services::messaging::InboundAcceptOutcome::Accepted(_)
                    | crate::services::messaging::InboundAcceptOutcome::Duplicate { .. } => {
                        acknowledged.push(transient_id);
                    }
                    crate::services::messaging::InboundAcceptOutcome::Rejected { .. } => {
                        return Err(TransportError::SendFailed(
                            "canonical inbound propagation pipeline rejected message".into(),
                        ));
                    }
                    crate::services::messaging::InboundAcceptOutcome::StorageError {
                        error,
                        ..
                    } => {
                        return Err(TransportError::SendFailed(format!(
                            "canonical inbound propagation storage: {error}"
                        )));
                    }
                }
            }
            if !acknowledged.is_empty() {
                self.request(
                    link_id,
                    GET_PATH,
                    get_request(None, Some(&acknowledged), None),
                    hex::encode(attempt_id),
                    deadline,
                    &cancellation,
                )
                .await?;
                self.store
                    .lock()
                    .map_err(|_| {
                        TransportError::SendFailed("standard propagation store poisoned".into())
                    })?
                    .standard_propagation_mark_haves_acknowledged(
                        peer.identity_hash,
                        &acknowledged,
                        attempt_id,
                        unix_time().unwrap_or(0),
                    )
                    .map_err(|error| {
                        TransportError::SendFailed(format!("acknowledge downloads: {error}"))
                    })?;
            }
            self.store
                .lock()
                .map_err(|_| {
                    TransportError::SendFailed("standard propagation store poisoned".into())
                })?
                .standard_propagation_complete_client_attempt(
                    attempt_id,
                    peer.identity_hash,
                    &acknowledged,
                    "returned",
                    0,
                    unix_time().unwrap_or(0),
                )
                .map_err(|error| {
                    TransportError::SendFailed(format!("complete download: {error}"))
                })?;
            active_attempt = None;
            Ok(acknowledged.len())
        }
        .await;
        if result.is_err()
            && let Some(attempt_id) = active_attempt
            && let Ok(mut store) = self.store.lock()
        {
            let _ = store.standard_propagation_record_attempt_failure(
                attempt_id,
                peer.identity_hash,
                "client_sync_failed",
                None,
                unix_time().unwrap_or(0),
            );
        }
        if owned && let Err(error) = self.transport.close_link(&link_id).await {
            crate::daemon_diagnostic!(
                "[standard-propagation] owned sync link cleanup failed: {error}"
            );
        }
        result
    }
}

fn decrypt_fetched_wire(
    identity: &PrivateIdentity,
    local_destination: AddressHash,
    wanted: &BTreeSet<[u8; 32]>,
    lxmf_data: &[u8],
) -> Result<([u8; 32], Vec<u8>, lxmf::WireMessage), TransportError> {
    if lxmf_data.len() < lxmf::propagation::MIN_PROPAGATED_LXMF_BYTES {
        return Err(TransportError::SendFailed("propagation payload is too short".into()));
    }
    let transient_id = lxmf::propagation::transient_id(lxmf_data);
    if !wanted.contains(&transient_id) {
        return Err(TransportError::SendFailed(
            "propagation node returned an unrequested transient".into(),
        ));
    }
    if lxmf_data[..16] != local_destination.as_slice()[..] {
        return Err(TransportError::SendFailed(
            "propagation clear destination does not match local delivery destination".into(),
        ));
    }
    let decrypted = lxmf::message::decrypt_for_identity(identity, &lxmf_data[16..], OsRng)
        .map_err(|error| TransportError::SendFailed(format!("propagation decrypt: {error}")))?;
    let mut full_wire = Vec::with_capacity(16 + decrypted.len());
    full_wire.extend_from_slice(local_destination.as_slice());
    full_wire.extend_from_slice(&decrypted);
    let wire = lxmf::WireMessage::unpack(&full_wire)
        .map_err(|error| TransportError::SendFailed(format!("propagation wire: {error}")))?;
    if wire.destination.as_slice() != local_destination.as_slice() {
        return Err(TransportError::SendFailed(
            "decrypted propagation destination mismatch".into(),
        ));
    }
    Ok((transient_id, full_wire, wire))
}

fn random_attempt_id() -> [u8; 16] {
    let mut id = [0u8; 16];
    OsRng.fill_bytes(&mut id);
    id
}

fn get_request(
    wants: Option<&[[u8; 32]]>,
    haves: Option<&[[u8; 32]]>,
    delivery_per_transfer_limit_kb: Option<usize>,
) -> Vec<u8> {
    let ids = |values: Option<&[[u8; 32]]>| {
        values.map_or(rmpv::Value::Nil, |values| {
            rmpv::Value::Array(
                values.iter().map(|value| rmpv::Value::Binary(value.to_vec())).collect(),
            )
        })
    };
    let mut parts = vec![ids(wants), ids(haves)];
    if let Some(value) = delivery_per_transfer_limit_kb {
        parts.push(rmpv::Value::from(u64::try_from(value).unwrap_or(u64::MAX)));
    }
    encode_value(rmpv::Value::Array(parts))
}

fn download_limit_kb(advertised_sync_limit_kb: Option<usize>) -> usize {
    let response_ceiling_kb = MAX_GET_RESPONSE_BYTES / DECIMAL_KB;
    debug_assert!(MAX_GET_RESPONSE_BYTES == 0 || response_ceiling_kb > 0);
    let response_ceiling_kb = response_ceiling_kb.max(usize::from(MAX_GET_RESPONSE_BYTES > 0));
    advertised_sync_limit_kb.unwrap_or(response_ceiling_kb).min(response_ceiling_kb)
}

fn decode_binary_ids(encoded: &[u8], maximum: usize) -> Result<Vec<[u8; 32]>, TransportError> {
    let value = decode_exact(encoded)
        .ok_or_else(|| TransportError::SendFailed("malformed propagation inventory".into()))?;
    let values = value
        .as_array()
        .filter(|values| values.len() <= maximum)
        .ok_or_else(|| TransportError::SendFailed("invalid propagation inventory bounds".into()))?;
    values
        .iter()
        .map(|value| match value {
            rmpv::Value::Binary(bytes) => bytes
                .as_slice()
                .try_into()
                .map_err(|_| TransportError::SendFailed("invalid propagation transient ID".into())),
            _ => Err(TransportError::SendFailed("non-binary propagation transient ID".into())),
        })
        .collect()
}

fn decode_binary_payloads(
    encoded: &[u8],
    maximum: usize,
    maximum_bytes: usize,
) -> Result<Vec<Vec<u8>>, TransportError> {
    if encoded.len() > maximum_bytes {
        return Err(TransportError::SendFailed("propagation response exceeds limit".into()));
    }
    let value = decode_exact(encoded)
        .ok_or_else(|| TransportError::SendFailed("malformed propagation response".into()))?;
    let values = value
        .as_array()
        .filter(|values| values.len() <= maximum)
        .ok_or_else(|| TransportError::SendFailed("invalid propagation response bounds".into()))?;
    let mut total = 0usize;
    values
        .iter()
        .map(|value| match value {
            rmpv::Value::Binary(bytes) => {
                total = total.saturating_add(bytes.len());
                if total > maximum_bytes {
                    Err(TransportError::SendFailed(
                        "propagation payload total exceeds limit".into(),
                    ))
                } else {
                    Ok(bytes.clone())
                }
            }
            _ => Err(TransportError::SendFailed("non-binary propagation payload".into())),
        })
        .collect()
}

/// Standard LXMF propagation endpoint backed by the guarded message database.
pub struct StandardPropagationEndpoint {
    destination: Arc<Mutex<SingleInputDestination>>,
    node_name: String,
    state: StandardPropagationState,
    shared: Arc<StdMutex<PropagationState>>,
    store: Arc<StdMutex<MessagesStore>>,
    local_identity_hash: [u8; 16],
    policy: PropagationPolicy,
    clock: Arc<dyn PropagationClock>,
    ingress_lifetime: Option<Arc<()>>,
    worker: Option<IngressWorker>,
    runtime_observation: StandardPropagationRuntimeObservation,
    events: Arc<OnceLock<Arc<EventService>>>,
}

impl StandardPropagationEndpoint {
    pub async fn register(
        transport: &mut Transport,
        identity: PrivateIdentity,
        node_name: &str,
        store: Arc<StdMutex<MessagesStore>>,
    ) -> Result<Self, StandardPropagationRegistrationError> {
        Self::register_with_policy(
            transport,
            identity,
            node_name,
            store,
            PropagationPolicy::default(),
            Arc::new(SystemPropagationClock),
        )
        .await
    }

    async fn register_with_policy(
        transport: &mut Transport,
        identity: PrivateIdentity,
        node_name: &str,
        store: Arc<StdMutex<MessagesStore>>,
        policy: PropagationPolicy,
        clock: Arc<dyn PropagationClock>,
    ) -> Result<Self, StandardPropagationRegistrationError> {
        let node_name = StandardPropagationAnnounce::inactive(0, Some(node_name))?
            .node_name
            .ok_or(StandardPropagationRegistrationError::MissingName)?;
        store
            .lock()
            .map_err(|_| StandardPropagationRegistrationError::Storage)?
            .standard_propagation_reconcile_startup(clock.now(), policy.storage())
            .map_err(|_| StandardPropagationRegistrationError::Storage)?;
        let mut local_identity_hash = [0u8; 16];
        local_identity_hash.copy_from_slice(identity.address_hash().as_slice());
        let destination = transport
            .add_destination_checked(identity, DestinationName::new("lxmf", "propagation"))
            .await
            .map_err(StandardPropagationRegistrationError::Transport)?;
        let shared = Arc::new(StdMutex::new(PropagationState::default()));
        let events = Arc::new(OnceLock::new());
        register_handlers(
            &destination,
            Arc::clone(&shared),
            Arc::clone(&store),
            local_identity_hash,
            policy,
            Arc::clone(&clock),
            Arc::clone(&events),
        )
        .await?;
        let runtime_observation =
            StandardPropagationRuntimeObservation::registered(StandardPropagationRuntimePolicy {
                target_cost: policy.target_cost,
                flexibility: policy.flexibility,
                peering_cost: policy.peering_cost,
                transfer_limit_kb: policy.transfer_limit_kb,
                sync_limit_kb: policy.sync_limit_kb,
                queue_max_count: policy.queue_max_count,
                queue_max_bytes: policy.queue_max_bytes,
                expiry_secs: policy.expiry_secs,
                throttle_secs: policy.throttle_secs,
                max_offer_links: policy.max_offer_links,
            });
        Ok(Self {
            destination,
            node_name,
            state: StandardPropagationState::HandlersReady,
            shared,
            store,
            local_identity_hash,
            policy,
            clock,
            ingress_lifetime: Some(Arc::new(())),
            worker: None,
            runtime_observation,
            events,
        })
    }

    pub fn destination(&self) -> &Arc<Mutex<SingleInputDestination>> {
        &self.destination
    }

    pub fn state(&self) -> StandardPropagationState {
        self.state
    }

    pub fn local_identity_hash(&self) -> [u8; 16] {
        self.local_identity_hash
    }

    pub fn handlers_registered(&self) -> bool {
        matches!(
            self.state,
            StandardPropagationState::HandlersReady | StandardPropagationState::Active
        )
    }

    pub fn ingress_running(&self) -> bool {
        self.worker.is_some()
    }

    pub fn runtime_observation(&self) -> StandardPropagationRuntimeObservation {
        self.runtime_observation.clone()
    }

    pub fn set_events(&self, events: Arc<EventService>) {
        let _ = self.events.set(events);
    }

    pub fn inactive_app_data(&self, unix_secs: i64) -> Result<Vec<u8>, AnnounceError> {
        StandardPropagationAnnounce::inactive(unix_secs, Some(&self.node_name))?.encode()
    }

    pub fn active_app_data(&self, unix_secs: i64) -> Result<Vec<u8>, AnnounceError> {
        let mut announce = StandardPropagationAnnounce::active(
            unix_secs,
            Some(&self.node_name),
            self.policy.transfer_limit_kb as i64,
            self.policy.sync_limit_kb as i64,
        )?;
        announce.stamp_cost = i64::from(self.policy.target_cost);
        announce.stamp_cost_flexibility = i64::from(self.policy.flexibility);
        announce.peering_cost = i64::from(self.policy.peering_cost);
        announce.encode()
    }

    pub async fn activate(
        &mut self,
        transport: Arc<dyn MeshTransport>,
        native_transport: &Transport,
    ) -> Result<SendPacketOutcome, StandardPropagationActivationError> {
        if self.state != StandardPropagationState::HandlersReady || self.worker.is_some() {
            return Err(StandardPropagationActivationError::InvalidState);
        }
        let app_data = self.active_app_data(unix_time()?)?;
        let propagation_destination = self.destination.lock().await.desc.address_hash;
        {
            let mut destination = self.destination.lock().await;
            destination.register_ingress_handler(ingress_callback(
                &self.shared,
                &self.store,
                self.policy,
                Arc::clone(&self.clock),
                self.ingress_lifetime
                    .as_ref()
                    .ok_or(StandardPropagationActivationError::InvalidState)?,
                Arc::clone(&self.events),
            ))?;
            destination.set_ingress_resource_limit(Some(self.policy.sync_limit_bytes()));
        }
        self.worker = Some(spawn_ingress_worker(
            transport,
            Arc::clone(&self.shared),
            Arc::clone(&self.store),
            propagation_destination,
            self.policy,
            Arc::clone(&self.clock),
            Arc::clone(&self.events),
        ));
        let outcome = native_transport.send_announce(&self.destination, Some(&app_data)).await;
        if !matches!(outcome, SendPacketOutcome::SentDirect | SendPacketOutcome::SentBroadcast) {
            {
                let mut destination = self.destination.lock().await;
                destination.unregister_ingress_handler();
                destination.set_ingress_resource_limit(None);
            }
            if let Some(mut worker) = self.worker.take() {
                worker.shutdown().await;
            }
            return Err(StandardPropagationActivationError::AnnounceNotSent(outcome));
        }
        self.state = StandardPropagationState::Active;
        self.runtime_observation.set_active(true);
        Ok(outcome)
    }

    pub async fn shutdown(&mut self) {
        {
            let mut destination = self.destination.lock().await;
            destination.unregister_ingress_handler();
            destination.set_ingress_resource_limit(None);
        }
        if let Some(mut worker) = self.worker.take() {
            worker.shutdown().await;
        }
        if self.state == StandardPropagationState::Active {
            self.state = StandardPropagationState::HandlersReady;
        }
        self.runtime_observation.set_active(false);
    }

    pub async fn signed_inactive_announce(&self, unix_secs: i64) -> Result<Packet, AnnounceError> {
        let app_data = self.inactive_app_data(unix_secs)?;
        self.destination
            .lock()
            .await
            .announce(OsRng, Some(&app_data))
            .map_err(|_| AnnounceError::Encoding)
    }

    #[cfg(test)]
    fn queue_snapshot(&self) -> QueueSnapshot {
        self.store
            .lock()
            .unwrap()
            .standard_propagation_snapshot(self.clock.now(), self.policy.storage())
            .unwrap()
            .into_iter()
            .map(|message| (message.transient_id, message.lxmf_data, message.stamp))
            .collect()
    }

    pub fn set_selection(
        &self,
        peer: Option<[u8; 16]>,
        mode: &str,
        now: i64,
    ) -> rusqlite::Result<()> {
        self.store
            .lock()
            .map_err(|_| {
                rusqlite::Error::InvalidParameterName("propagation store poisoned".into())
            })?
            .standard_propagation_set_selection(peer, mode, now)?;
        emit_changed(&self.events, now);
        Ok(())
    }

    pub fn selection(&self) -> rusqlite::Result<Option<StandardPropagationSelection>> {
        self.store
            .lock()
            .map_err(|_| {
                rusqlite::Error::InvalidParameterName("propagation store poisoned".into())
            })?
            .standard_propagation_selection()
    }

    pub fn queue_stats(&self, now: i64) -> rusqlite::Result<StandardPropagationStats> {
        self.store
            .lock()
            .map_err(|_| {
                rusqlite::Error::InvalidParameterName("propagation store poisoned".into())
            })?
            .standard_propagation_stats(now, self.policy.storage())
    }
}

impl Drop for StandardPropagationEndpoint {
    fn drop(&mut self) {
        self.runtime_observation.set_active(false);
        self.ingress_lifetime.take();
        if let Some(worker) = &self.worker {
            worker.abort();
        }
    }
}

fn unix_time() -> Result<i64, AnnounceError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
        .ok_or(AnnounceError::InvalidEmission)
}

async fn register_handlers(
    destination: &Arc<Mutex<SingleInputDestination>>,
    shared: Arc<StdMutex<PropagationState>>,
    store: Arc<StdMutex<MessagesStore>>,
    local_identity_hash: [u8; 16],
    policy: PropagationPolicy,
    clock: Arc<dyn PropagationClock>,
    events: Arc<OnceLock<Arc<EventService>>>,
) -> Result<(), RequestRegistrationError> {
    let offer_state = Arc::clone(&shared);
    let offer_store = Arc::clone(&store);
    let offer_clock = Arc::clone(&clock);
    let offer_events = Arc::clone(&events);
    let mut destination = destination.lock().await;
    destination.register_request_path(
        OFFER_PATH,
        RequestAccess::Public,
        MAX_OFFER_BYTES,
        MAX_OFFER_BYTES,
        Arc::new(move |data, remote, link, request_id| {
            remote.map_or_else(
                || encode_value(rmpv::Value::from(ERROR_NO_IDENTITY)),
                |remote| {
                    let mut remote_hash = [0u8; 16];
                    remote_hash.copy_from_slice(remote.address_hash.as_slice());
                    handle_offer(
                        OfferHandlerContext {
                            shared: &offer_state,
                            store: &offer_store,
                            link_id: link.link_id,
                            remote_identity: remote_hash,
                            local_identity: local_identity_hash,
                            policy,
                            now: offer_clock.now(),
                            request_id,
                            events: &offer_events,
                        },
                        data,
                    )
                },
            )
        }),
    )?;
    destination.register_request_path(
        GET_PATH,
        RequestAccess::Public,
        MAX_GET_REQUEST_BYTES,
        MAX_GET_RESPONSE_BYTES,
        Arc::new(move |data, remote, _, request_id| {
            remote.map_or_else(
                || encode_value(rmpv::Value::from(ERROR_NO_IDENTITY)),
                |remote| handle_get(&store, remote, data, policy, clock.now(), request_id, &events),
            )
        }),
    )?;
    Ok(())
}

fn decode_exact(data: &[u8]) -> Option<rmpv::Value> {
    let mut cursor = std::io::Cursor::new(data);
    let value = rmpv::decode::read_value(&mut cursor).ok()?;
    (cursor.position() == data.len() as u64).then_some(value)
}

fn decode_ids(value: &rmpv::Value, allow_nil: bool, max: usize) -> Option<Option<Vec<[u8; 32]>>> {
    if allow_nil && value.is_nil() {
        return Some(None);
    }
    let values = value.as_array().filter(|values| values.len() <= max)?;
    let mut ids = Vec::with_capacity(values.len());
    for value in values {
        let rmpv::Value::Binary(bytes) = value else { return None };
        ids.push(bytes.as_slice().try_into().ok()?);
    }
    Some(Some(ids))
}

struct OfferHandlerContext<'a> {
    shared: &'a Arc<StdMutex<PropagationState>>,
    store: &'a Arc<StdMutex<MessagesStore>>,
    link_id: AddressHash,
    remote_identity: [u8; 16],
    local_identity: [u8; 16],
    policy: PropagationPolicy,
    now: i64,
    request_id: [u8; 16],
    events: &'a Arc<OnceLock<Arc<EventService>>>,
}

fn handle_offer(context: OfferHandlerContext<'_>, data: &[u8]) -> Vec<u8> {
    let OfferHandlerContext {
        shared,
        store,
        link_id,
        remote_identity,
        local_identity,
        policy,
        now,
        request_id,
        events,
    } = context;
    let Some(value) = decode_exact(data) else { return invalid_data() };
    let Some(parts) = value.as_array().filter(|parts| parts.len() == 2) else {
        return invalid_data();
    };
    let rmpv::Value::Binary(peering_key) = &parts[0] else { return invalid_data() };
    if peering_key.len() != 32 {
        return invalid_data();
    }
    let Some(Some(offered)) = decode_ids(&parts[1], false, MAX_OFFER_IDS) else {
        return invalid_data();
    };
    {
        let mut state = shared.lock().unwrap();
        state.expire(now, policy);
        if state.throttled.get(&remote_identity).is_some_and(|until| now < *until) {
            return encode_value(rmpv::Value::from(ERROR_THROTTLED));
        }
    }
    if validate_peering_key(peering_key, &local_identity, &remote_identity, policy.peering_cost)
        .is_err()
    {
        return encode_value(rmpv::Value::from(ERROR_INVALID_KEY));
    }
    let mut unique = Vec::with_capacity(offered.len());
    let mut seen = BTreeSet::new();
    for id in offered {
        if seen.insert(id) {
            unique.push(id);
        }
    }

    let (already_pending, pending_elsewhere, pending_count, existing_attempt) = {
        let mut state = shared.lock().unwrap();
        state.expire(now, policy);
        let is_new_link = !state.pending.contains_key(&link_id);
        let active_offer_links =
            state.pending.values().filter(|pending| !pending.ids.is_empty()).count();
        if is_new_link && active_offer_links >= policy.max_offer_links {
            return encode_value(rmpv::Value::from(ERROR_THROTTLED));
        }
        let already_pending =
            state.pending.get(&link_id).map(|pending| pending.ids.clone()).unwrap_or_default();
        let existing_attempt = state.pending.get(&link_id).map(|pending| pending.attempt_id);
        let pending_elsewhere = state
            .pending
            .iter()
            .filter(|(pending_link, _)| **pending_link != link_id)
            .flat_map(|(_, pending)| pending.ids.iter().copied())
            .collect();
        (already_pending, pending_elsewhere, state.pending_count(), existing_attempt)
    };
    let mut link_bytes = [0u8; 16];
    link_bytes.copy_from_slice(link_id.as_slice());
    let mut store = match store.lock() {
        Ok(store) => store,
        Err(_) => return encode_value(rmpv::Value::from(ERROR_THROTTLED)),
    };
    let comparison =
        match store.standard_propagation_compare_offer(StandardPropagationOfferRequest {
            peer: remote_identity,
            offered: &unique,
            same_link_pending: &already_pending,
            pending_elsewhere: &pending_elsewhere,
            pending_count,
            existing_attempt,
            request_id,
            link_id: link_bytes,
            now,
            deadline: now.saturating_add(policy.throttle_secs),
            policy: policy.storage(),
        }) {
            Ok(comparison) => comparison,
            Err(_) => return encode_value(rmpv::Value::from(ERROR_THROTTLED)),
        };
    if comparison.capacity_rejected {
        drop(store);
        emit_changed(events, now);
        return encode_value(rmpv::Value::from(ERROR_THROTTLED));
    }
    let wanted = comparison.wanted;
    if !wanted.is_empty() {
        let mut state = shared.lock().unwrap();
        state.validated_links.insert(link_id, remote_identity);
        let pending = state.pending.entry(link_id).or_insert_with(|| PendingOffer {
            remote_identity,
            attempt_id: comparison.attempt_id,
            deadline: now.saturating_add(policy.throttle_secs),
            ids: BTreeSet::new(),
        });
        pending.remote_identity = remote_identity;
        pending.attempt_id = comparison.attempt_id;
        pending.deadline = now.saturating_add(policy.throttle_secs);
        pending.ids.extend(wanted.iter().copied());
    }
    drop(store);
    emit_changed(events, now);
    if wanted.is_empty() {
        encode_value(rmpv::Value::Boolean(false))
    } else if wanted.len() == unique.len() {
        encode_value(rmpv::Value::Boolean(true))
    } else {
        encode_value(rmpv::Value::Array(
            wanted.into_iter().map(|id| rmpv::Value::Binary(id.to_vec())).collect(),
        ))
    }
}

fn optional_decimal_kb_limit(value: &rmpv::Value) -> Option<usize> {
    let value = match value {
        rmpv::Value::Integer(value) => value
            .as_i64()
            .map(|value| value as f64)
            .or_else(|| value.as_u64().map(|value| value as f64)),
        rmpv::Value::F32(value) => Some(f64::from(*value)),
        rmpv::Value::F64(value) => Some(*value),
        rmpv::Value::String(value) => value.as_str().and_then(|value| value.parse().ok()),
        rmpv::Value::Binary(value) => {
            core::str::from_utf8(value).ok().and_then(|value| value.parse().ok())
        }
        _ => None,
    }?;
    if !value.is_finite() {
        return None;
    }
    if value <= 0.0 {
        return Some(0);
    }
    let bytes = value * DECIMAL_KB as f64;
    if bytes >= usize::MAX as f64 { Some(usize::MAX) } else { Some(bytes as usize) }
}

fn handle_get(
    store: &Arc<StdMutex<MessagesStore>>,
    remote: &Identity,
    data: &[u8],
    policy: PropagationPolicy,
    now: i64,
    request_id: [u8; 16],
    events: &Arc<OnceLock<Arc<EventService>>>,
) -> Vec<u8> {
    let Some(value) = decode_exact(data) else { return invalid_data() };
    let Some(parts) = value.as_array().filter(|parts| (2..=3).contains(&parts.len())) else {
        return invalid_data();
    };
    let Some(wants) = decode_ids(&parts[0], true, MAX_GET_IDS) else { return invalid_data() };
    let Some(haves) = decode_ids(&parts[1], true, MAX_GET_IDS) else { return invalid_data() };
    let client_limit = parts.get(2).and_then(optional_decimal_kb_limit);
    let response_limit = client_limit.unwrap_or(MAX_GET_RESPONSE_BYTES).min(MAX_GET_RESPONSE_BYTES);
    let recipient = SingleOutputDestination::new(*remote, DestinationName::new("lxmf", "delivery"))
        .desc
        .address_hash;
    let mut recipient_bytes = [0u8; 16];
    recipient_bytes.copy_from_slice(recipient.as_slice());
    let mut peer = [0u8; 16];
    peer.copy_from_slice(remote.address_hash.as_slice());
    let inventory = wants.is_none() && parts[0].is_nil() && parts[1].is_nil();
    let result = match store.lock() {
        Ok(mut store) => store.standard_propagation_get(StandardPropagationGetRequest {
            peer,
            request_id,
            recipient: recipient_bytes,
            wants: wants.as_deref(),
            haves: haves.as_deref(),
            inventory,
            response_limit,
            now,
            policy: policy.storage(),
        }),
        Err(_) => return invalid_data(),
    };
    let Ok(result) = result else { return invalid_data() };
    emit_changed(events, now);
    if let Some(inventory) = result.inventory {
        return bounded_array(
            inventory.into_iter().map(|id| rmpv::Value::Binary(id.to_vec())),
            MAX_GET_RESPONSE_BYTES,
        );
    }
    encode_value(rmpv::Value::Array(result.payloads.into_iter().map(rmpv::Value::Binary).collect()))
}

fn emit_changed(events: &Arc<OnceLock<Arc<EventService>>>, observed_at: i64) {
    if let Some(events) = events.get() {
        events.emit_standard_propagation_changed(observed_at.max(0));
    }
}

fn bounded_array(values: impl IntoIterator<Item = rmpv::Value>, limit: usize) -> Vec<u8> {
    let mut accepted = Vec::new();
    let mut item_bytes = 0usize;
    for value in values {
        let encoded_len = encode_value(value.clone()).len();
        let next_len = accepted.len() + 1;
        let header_len = if next_len <= 15 {
            1
        } else if next_len <= u16::MAX as usize {
            3
        } else {
            5
        };
        if item_bytes.saturating_add(encoded_len).saturating_add(header_len) > limit {
            continue;
        }
        item_bytes += encoded_len;
        accepted.push(value);
    }
    encode_value(rmpv::Value::Array(accepted))
}

fn encode_value(value: rmpv::Value) -> Vec<u8> {
    let mut encoded = Vec::new();
    if rmpv::encode::write_value(&mut encoded, &value).is_err() {
        return Vec::new();
    }
    encoded
}

fn invalid_data() -> Vec<u8> {
    encode_value(rmpv::Value::from(ERROR_INVALID_DATA))
}

fn ingress_callback(
    shared: &Arc<StdMutex<PropagationState>>,
    store: &Arc<StdMutex<MessagesStore>>,
    policy: PropagationPolicy,
    clock: Arc<dyn PropagationClock>,
    lifetime: &Arc<()>,
    events: Arc<OnceLock<Arc<EventService>>>,
) -> IngressHandler {
    let weak_state = Arc::downgrade(shared);
    let weak_store = Arc::downgrade(store);
    let weak_lifetime = Arc::downgrade(lifetime);
    Arc::new(move |data, context: &IngressContext| {
        weak_lifetime.upgrade().is_some()
            && weak_state.upgrade().zip(weak_store.upgrade()).is_some_and(|(state, store)| {
                process_transfer(
                    &state,
                    &store,
                    context.link_id,
                    data,
                    policy,
                    clock.now(),
                    &events,
                )
            })
    })
}

fn process_lifecycle_event(
    shared: &Arc<StdMutex<PropagationState>>,
    propagation_destination: &str,
    event: TransportLifecycleEvent,
) {
    match event {
        TransportLifecycleEvent::LinkClosed { link_id, peer_hash, .. }
            if peer_hash == propagation_destination =>
        {
            if let Ok(link_id) = AddressHash::new_from_hex_string(&link_id) {
                let mut state = shared.lock().unwrap();
                state.pending.remove(&link_id);
                state.validated_links.remove(&link_id);
                state.link_throttled.remove(&link_id);
            }
        }
        TransportLifecycleEvent::Disconnected | TransportLifecycleEvent::LinkReconcileRequired => {
            let mut state = shared.lock().unwrap();
            state.pending.clear();
            state.validated_links.clear();
            state.link_throttled.clear();
        }
        _ => {}
    }
}

fn spawn_ingress_worker(
    transport: Arc<dyn MeshTransport>,
    shared: Arc<StdMutex<PropagationState>>,
    store: Arc<StdMutex<MessagesStore>>,
    propagation_destination: AddressHash,
    policy: PropagationPolicy,
    clock: Arc<dyn PropagationClock>,
    events: Arc<OnceLock<Arc<EventService>>>,
) -> IngressWorker {
    let mut lifecycle_rx = transport.subscribe_lifecycle();
    let propagation_destination = hex::encode(propagation_destination.as_slice());
    let lifecycle = tokio::spawn(async move {
        let mut deadline_tick = tokio::time::interval(std::time::Duration::from_secs(1));
        deadline_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                event = lifecycle_rx.recv() => match event {
                    Ok(event) => process_lifecycle_event(&shared, &propagation_destination, event),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
                _ = deadline_tick.tick() => {
                    let now = clock.now();
                    shared.lock().unwrap().expire(now, policy);
                    if let Ok(mut store) = store.lock()
                        && store.standard_propagation_reconcile_deadlines(now).unwrap_or(0) > 0 {
                            emit_changed(&events, now);
                        }
                }
            }
        }
    });
    IngressWorker { lifecycle }
}

fn process_transfer(
    shared: &Arc<StdMutex<PropagationState>>,
    store: &Arc<StdMutex<MessagesStore>>,
    link_id: AddressHash,
    encoded: &[u8],
    policy: PropagationPolicy,
    now: i64,
    events: &Arc<OnceLock<Arc<EventService>>>,
) -> bool {
    let sync_limit = policy.sync_limit_kb.saturating_mul(DECIMAL_KB);
    if let Ok(mut store) = store.lock()
        && store.standard_propagation_reconcile_deadlines(now).unwrap_or(0) > 0
    {
        emit_changed(events, now);
    }
    let (pending, validated_remote) = {
        let mut state = shared.lock().unwrap();
        state.expire(now, policy);
        let validated_remote = state.validated_links.get(&link_id).copied();
        let link_is_throttled =
            state.link_throttled.get(&link_id).is_some_and(|until| now < *until);
        let peer_is_throttled = validated_remote
            .is_some_and(|remote| state.throttled.get(&remote).is_some_and(|until| now < *until));
        if link_is_throttled || peer_is_throttled {
            return false;
        }
        (state.pending.get(&link_id).cloned(), validated_remote)
    };
    if encoded.len() > sync_limit {
        throttle_transfer_link(shared, link_id, policy, now);
        record_pending_failure(store, pending.as_ref(), "sync_limit", now, events);
        shared.lock().unwrap().pending.remove(&link_id);
        return false;
    }
    let decoded = decode_transfer_envelope(encoded, sync_limit, MAX_TRANSFER_MESSAGES, sync_limit);
    let Ok(payloads) = decoded else {
        throttle_transfer_link(shared, link_id, policy, now);
        record_pending_failure(store, pending.as_ref(), "malformed_transfer", now, events);
        shared.lock().unwrap().pending.remove(&link_id);
        return false;
    };
    if payloads.is_empty()
        || (payloads.len() != 1 && (pending.is_none() || validated_remote.is_none()))
    {
        throttle_transfer_link(shared, link_id, policy, now);
        record_pending_failure(store, pending.as_ref(), "invalid_batch", now, events);
        shared.lock().unwrap().pending.remove(&link_id);
        return false;
    }
    let submitted_count = payloads.len();
    let transfer_limit = policy.transfer_limit_kb.saturating_mul(DECIMAL_KB);
    let mut invalid_batch = false;
    let mut validated = Vec::new();
    // Multi-message work is bounded by 4 MB/1024 items and only reaches stamp validation after
    // the link has supplied the production cost-18 peering proof through /offer.
    for stamped_payload in payloads {
        if stamped_payload.len().saturating_add(16) > transfer_limit {
            invalid_batch = true;
            continue;
        }
        match validate_propagation_stamp(&stamped_payload, policy.target_cost, policy.flexibility) {
            Ok(stamp) => {
                if let Some(destination) = propagated_destination(&stamp.lxmf_data) {
                    let received_at = now.max(0);
                    validated.push(StandardPropagationItem {
                        transient_id: stamp.transient_id,
                        destination,
                        lxmf_data: stamp.lxmf_data,
                        stamp: stamp.stamp,
                        stamp_value: stamp.value,
                        received_at,
                        expires_at: received_at.saturating_add(policy.expiry_secs),
                        stored_size: stamped_payload.len(),
                    });
                } else {
                    invalid_batch = true;
                }
            }
            Err(_) => invalid_batch = true,
        }
    }

    let accepted_ids = pending.as_ref().map(|pending| &pending.ids);
    for item in &validated {
        if accepted_ids.is_some_and(|ids| !ids.contains(&item.transient_id)) {
            invalid_batch = true;
        }
    }
    let source_peer = pending.as_ref().map(|pending| pending.remote_identity);
    let consumed: BTreeSet<_> = validated.iter().map(|item| item.transient_id).collect();
    let attempt = pending.as_ref().map_or(StandardPropagationAttemptStatus::Untracked, |pending| {
        let complete = !invalid_batch && pending.ids.iter().all(|id| consumed.contains(id));
        if complete {
            StandardPropagationAttemptStatus::Complete(pending.attempt_id)
        } else {
            StandardPropagationAttemptStatus::Partial(pending.attempt_id)
        }
    });
    let ingest = if validated.is_empty() {
        Err(rusqlite::Error::InvalidParameterName("empty validated propagation batch".into()))
    } else {
        match store.lock() {
            Ok(mut store) => {
                store.standard_propagation_ingest_batch(StandardPropagationIngestRequest {
                    items: &validated,
                    source_peer,
                    attempt,
                    protocol: if invalid_batch {
                        StandardPropagationProtocolStatus::Invalid
                    } else {
                        StandardPropagationProtocolStatus::Valid
                    },
                    now: now.max(0),
                    policy: policy.storage(),
                })
            }
            Err(_) => Err(rusqlite::Error::InvalidParameterName(
                "standard propagation store poisoned".into(),
            )),
        }
    };
    let storage_accepted = matches!(&ingest, Ok(StandardPropagationIngestOutcome::Accepted));
    if ingest.is_ok() {
        emit_changed(events, now);
    }
    if storage_accepted {
        let mut state = shared.lock().unwrap();
        if let Some(pending) = state.pending.get_mut(&link_id) {
            pending.ids.retain(|id| !consumed.contains(id));
            if pending.ids.is_empty() {
                state.pending.remove(&link_id);
            }
        }
    }
    let accepted = !invalid_batch && storage_accepted && validated.len() == submitted_count;
    let mut state = shared.lock().unwrap();
    if accepted {
        state.link_throttled.remove(&link_id);
    } else {
        set_transfer_throttle(&mut state, link_id, validated_remote, policy, now);
    }
    drop(state);
    if invalid_batch && validated.is_empty() {
        record_pending_failure(store, pending.as_ref(), "invalid_stamp", now, events);
        shared.lock().unwrap().pending.remove(&link_id);
    } else if ingest.is_err() {
        record_pending_failure(store, pending.as_ref(), "storage", now, events);
        shared.lock().unwrap().pending.remove(&link_id);
    }
    accepted
}

fn record_pending_failure(
    store: &Arc<StdMutex<MessagesStore>>,
    pending: Option<&PendingOffer>,
    code: &str,
    now: i64,
    events: &Arc<OnceLock<Arc<EventService>>>,
) {
    let Some(pending) = pending else { return };
    if let Ok(mut store) = store.lock()
        && store
            .standard_propagation_record_attempt_failure(
                pending.attempt_id,
                pending.remote_identity,
                code,
                None,
                now.max(0),
            )
            .is_ok()
    {
        emit_changed(events, now);
    }
}

fn set_transfer_throttle(
    state: &mut PropagationState,
    link_id: AddressHash,
    validated_remote: Option<[u8; 16]>,
    policy: PropagationPolicy,
    now: i64,
) {
    let until = now.saturating_add(policy.throttle_secs);
    state.link_throttled.insert(link_id, until);
    if let Some(remote) = validated_remote {
        state.throttled.insert(remote, until);
    }
}

fn throttle_transfer_link(
    shared: &Arc<StdMutex<PropagationState>>,
    link_id: AddressHash,
    policy: PropagationPolicy,
    now: i64,
) {
    let mut state = shared.lock().unwrap();
    let validated_remote = state.validated_links.get(&link_id).copied();
    set_transfer_throttle(&mut state, link_id, validated_remote, policy, now);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StandardPropagationRegistrationError {
    MissingName,
    Metadata(AnnounceError),
    Transport(DestinationRegistrationError),
    Request(RequestRegistrationError),
    Storage,
}

impl From<AnnounceError> for StandardPropagationRegistrationError {
    fn from(value: AnnounceError) -> Self {
        Self::Metadata(value)
    }
}

impl From<RequestRegistrationError> for StandardPropagationRegistrationError {
    fn from(value: RequestRegistrationError) -> Self {
        Self::Request(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StandardPropagationActivationError {
    InvalidState,
    Metadata(AnnounceError),
    Ingress(IngressRegistrationError),
    AnnounceNotSent(SendPacketOutcome),
}

impl From<AnnounceError> for StandardPropagationActivationError {
    fn from(value: AnnounceError) -> Self {
        Self::Metadata(value)
    }
}

impl From<IngressRegistrationError> for StandardPropagationActivationError {
    fn from(value: IngressRegistrationError) -> Self {
        Self::Ingress(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lxmf::propagation::transient_id;
    use rns_core::destination::{DestinationAnnounce, RequestLinkContext, request_path_hash};
    use rns_core::transport::core_transport::TransportConfig;

    fn transport(identity: &PrivateIdentity) -> Transport {
        Transport::new(TransportConfig::new("standard-propagation-test", identity, true))
    }

    async fn endpoint(name: &str) -> StandardPropagationEndpoint {
        endpoint_with_store(name, Arc::new(StdMutex::new(MessagesStore::in_memory().unwrap())))
            .await
    }

    async fn endpoint_with_store(
        name: &str,
        store: Arc<StdMutex<MessagesStore>>,
    ) -> StandardPropagationEndpoint {
        let identity = PrivateIdentity::new_from_name(name);
        let policy = PropagationPolicy {
            target_cost: 0,
            flexibility: 0,
            peering_cost: 0,
            ..PropagationPolicy::default()
        };
        StandardPropagationEndpoint::register_with_policy(
            &mut transport(&identity),
            identity,
            "Propagation Host",
            store,
            policy,
            Arc::new(SystemPropagationClock),
        )
        .await
        .unwrap()
    }

    fn encode(value: rmpv::Value) -> Vec<u8> {
        encode_value(value)
    }

    fn decode(data: &[u8]) -> rmpv::Value {
        decode_exact(data).unwrap()
    }

    fn request_response(value: rmpv::Value) -> styrene_ipc::types::RequestObservationInfo {
        let mut response = styrene_ipc::types::RequestObservationInfo::default();
        response.request_id = "55".repeat(16);
        response.state = styrene_ipc::types::RequestState::Succeeded;
        response.response = Some(encode(value));
        response
    }

    fn propagated(destination: [u8; 16], fill: u8) -> Vec<u8> {
        let mut data = vec![fill; lxmf::propagation::MIN_PROPAGATED_LXMF_BYTES + 1];
        data[..16].copy_from_slice(&destination);
        data
    }

    fn offer(ids: &[[u8; 32]]) -> Vec<u8> {
        encode(rmpv::Value::Array(vec![
            rmpv::Value::Binary(vec![0; 32]),
            rmpv::Value::Array(ids.iter().map(|id| rmpv::Value::Binary(id.to_vec())).collect()),
        ]))
    }

    fn stamped(data: &[u8]) -> Vec<u8> {
        let mut stamped = data.to_vec();
        stamped.extend_from_slice(&[0; 32]);
        stamped
    }

    fn transfer(payloads: impl IntoIterator<Item = Vec<u8>>) -> Vec<u8> {
        encode(rmpv::Value::Array(vec![
            0.into(),
            rmpv::Value::Array(payloads.into_iter().map(rmpv::Value::Binary).collect()),
        ]))
    }

    fn process(endpoint: &StandardPropagationEndpoint, link: AddressHash, encoded: &[u8]) -> bool {
        process_transfer(
            &endpoint.shared,
            &endpoint.store,
            link,
            encoded,
            endpoint.policy,
            endpoint.clock.now(),
            &endpoint.events,
        )
    }

    fn queued(data: Vec<u8>, received_at: i64) -> StandardPropagationItem {
        let destination = data[..16].try_into().unwrap();
        let stored_size = data.len() + 32;
        StandardPropagationItem {
            transient_id: transient_id(&data),
            lxmf_data: data,
            stamp: [0; 32],
            stamp_value: 0,
            destination,
            received_at,
            expires_at: received_at + DEFAULT_EXPIRY_SECS,
            stored_size,
        }
    }

    fn insert_queued(endpoint: &StandardPropagationEndpoint, item: StandardPropagationItem) {
        assert_eq!(
            endpoint
                .store
                .lock()
                .unwrap()
                .standard_propagation_ingest_batch(StandardPropagationIngestRequest {
                    items: &[item],
                    source_peer: None,
                    attempt: StandardPropagationAttemptStatus::Untracked,
                    protocol: StandardPropagationProtocolStatus::Valid,
                    now: 0,
                    policy: endpoint.policy.storage(),
                })
                .unwrap(),
            StandardPropagationIngestOutcome::Accepted
        );
    }

    fn direct_offer(
        endpoint: &StandardPropagationEndpoint,
        link: AddressHash,
        remote_hash: [u8; 16],
        now: i64,
        request_id: [u8; 16],
        ids: &[[u8; 32]],
    ) -> rmpv::Value {
        decode(&handle_offer(
            OfferHandlerContext {
                shared: &endpoint.shared,
                store: &endpoint.store,
                link_id: link,
                remote_identity: remote_hash,
                local_identity: endpoint.local_identity_hash,
                policy: endpoint.policy,
                now,
                request_id,
                events: &endpoint.events,
            },
            &offer(ids),
        ))
    }

    fn pending(ids: BTreeSet<[u8; 32]>) -> PendingOffer {
        PendingOffer {
            remote_identity: [0x11; 16],
            attempt_id: [0x12; 16],
            deadline: i64::MAX,
            ids,
        }
    }

    async fn dispatch(
        endpoint: &StandardPropagationEndpoint,
        path: &str,
        data: &[u8],
        remote: Option<&Identity>,
        link_id: AddressHash,
    ) -> Vec<u8> {
        let destination = endpoint.destination.lock().await;
        destination
            .dispatch_request(
                &request_path_hash(path),
                data,
                remote,
                &RequestLinkContext { link_id, destination: destination.desc.address_hash },
                [0x99; 16],
            )
            .unwrap()
    }

    #[tokio::test]
    async fn paths_handlers_hashes_and_active_metadata_are_exact() {
        let endpoint = endpoint("paths").await;
        let destination = endpoint.destination.lock().await;
        assert_eq!(
            destination.request_path(&request_path_hash(OFFER_PATH)).unwrap().path(),
            OFFER_PATH
        );
        assert_eq!(
            request_path_hash(OFFER_PATH),
            [
                0x94, 0xfd, 0x9f, 0xd7, 0xb0, 0x4a, 0x5c, 0xaa, 0xe5, 0x88, 0x26, 0x16, 0x44, 0x6b,
                0xb9, 0xef,
            ]
        );
        assert_eq!(
            destination.request_path(&request_path_hash(GET_PATH)).unwrap().path(),
            GET_PATH
        );
        assert_eq!(
            request_path_hash(GET_PATH),
            [
                0x9d, 0xc1, 0xa7, 0x28, 0x83, 0x46, 0x8f, 0x57, 0xfe, 0xd5, 0x71, 0xe7, 0x96, 0xe9,
                0xce, 0x98,
            ]
        );
        drop(destination);
        assert_eq!(endpoint.state(), StandardPropagationState::HandlersReady);
        let active =
            StandardPropagationAnnounce::parse(&endpoint.active_app_data(42).unwrap()).unwrap();
        assert!(active.node_active);
        assert_eq!(active.transfer_limit_kb, 256);
        assert_eq!(active.sync_limit_kb, 4000);
        assert_eq!(active.stamp_cost, 0);
        assert_eq!(active.peering_cost, 0);

        let packet = endpoint.signed_inactive_announce(1_700_000_000).await.unwrap();
        let parsed = StandardPropagationAnnounce::parse(
            DestinationAnnounce::validate(&packet).unwrap().app_data,
        )
        .unwrap();
        assert!(!parsed.node_active);
    }

    #[tokio::test]
    async fn fetch_decrypts_canonical_pipeline_and_correlates_random_reencryptions_n_to_one() {
        use crate::transport::mock_transport::MockTransport;
        use lxmf::{Payload, WireMessage};

        let local = Arc::new(PrivateIdentity::new_from_name("fetch-local"));
        let sender = PrivateIdentity::new_from_name("fetch-sender");
        let peer = PrivateIdentity::new_from_name("fetch-peer");
        let delivery = SingleOutputDestination::new(
            *local.as_identity(),
            DestinationName::new("lxmf", "delivery"),
        )
        .desc
        .address_hash;
        let propagation_destination = SingleOutputDestination::new(
            *peer.as_identity(),
            DestinationName::new("lxmf", "propagation"),
        )
        .desc
        .address_hash;
        let transport = Arc::new(MockTransport::new(local.address_hash().to_owned(), delivery));
        let store = Arc::new(StdMutex::new(MessagesStore::in_memory().unwrap()));
        let mut peer_hash = [0u8; 16];
        peer_hash.copy_from_slice(peer.address_hash().as_slice());
        store
            .lock()
            .unwrap()
            .standard_propagation_upsert_peer(
                &crate::storage::standard_propagation::StandardPropagationPeer {
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
                },
            )
            .unwrap();
        store
            .lock()
            .unwrap()
            .standard_propagation_set_selection(Some(peer_hash), "manual", 1)
            .unwrap();
        let messaging = crate::services::MessagingService::with_store(store.clone());
        let coordinator =
            StandardPropagationCoordinator::new(transport.clone(), store.clone(), local.clone());
        let mut source = [0u8; 16];
        source.copy_from_slice(sender.address_hash().as_slice());
        let mut destination = [0u8; 16];
        destination.copy_from_slice(delivery.as_slice());
        let mut wire = WireMessage::new(
            destination,
            source,
            Payload::new(1.0, Some(b"same canonical message".to_vec()), None, None, None),
        );
        wire.sign(&sender).unwrap();
        let message_id = hex::encode(wire.message_id());

        let transient = |wire: &WireMessage| {
            let (envelope, id) = wire
                .pack_propagation_with_options_and_rng(local.as_identity(), 2.0, None, OsRng)
                .unwrap();
            let payload = decode_transfer_envelope(&envelope, 4 * 1024 * 1024, 1, 4 * 1024 * 1024)
                .unwrap()
                .remove(0);
            (id, payload)
        };
        let first = transient(&wire);
        let second = transient(&wire);
        assert_ne!(first.0, second.0);
        let validation = transient(&wire);
        assert!(
            decrypt_fetched_wire(
                &PrivateIdentity::new_from_name("wrong-fetch-key"),
                delivery,
                &BTreeSet::from([validation.0]),
                &validation.1,
            )
            .is_err()
        );
        assert!(
            decrypt_fetched_wire(local.as_ref(), delivery, &BTreeSet::new(), &validation.1)
                .is_err()
        );
        let mut wrong_destination = validation.1.clone();
        wrong_destination[..16].copy_from_slice(&[0x99; 16]);
        let wrong_destination_id = lxmf::propagation::transient_id(&wrong_destination);
        assert!(
            decrypt_fetched_wire(
                local.as_ref(),
                delivery,
                &BTreeSet::from([wrong_destination_id]),
                &wrong_destination,
            )
            .is_err()
        );

        for (index, (transient_id, payload)) in [first, second].into_iter().enumerate() {
            transport.queue_resolve(Some(*peer.as_identity()));
            transport.queue_open_link(Ok(AddressHash::new([0x70 + index as u8; 16])));
            transport.queue_request(Ok(request_response(rmpv::Value::Array(vec![
                rmpv::Value::Binary(transient_id.to_vec()),
            ]))));
            transport.queue_request(Ok(request_response(rmpv::Value::Array(vec![
                rmpv::Value::Binary(payload),
            ]))));
            transport.queue_request(Ok(request_response(rmpv::Value::Array(vec![]))));
            transport.queue_close(Ok(()));
            assert_eq!(
                coordinator
                    .sync_once(
                        &messaging,
                        std::time::Instant::now() + std::time::Duration::from_secs(2),
                        CancellationToken::new(),
                    )
                    .await
                    .unwrap(),
                1
            );
        }
        let links =
            store.lock().unwrap().standard_propagation_links_for_message(&message_id, 64).unwrap();
        assert_eq!(links.len(), 2);
        assert!(links.iter().all(|link| link.state == "acknowledged"));
        assert_eq!(store.lock().unwrap().list_messages(10, None).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn offer_all_some_none_duplicate_malformed_and_unidentified() {
        let endpoint = endpoint("offers").await;
        let remote = PrivateIdentity::new_from_name("offer-remote");
        let link = AddressHash::new([1; 16]);
        let ids = [[1; 32], [2; 32]];
        let request = offer;
        assert_eq!(
            decode(
                &dispatch(&endpoint, OFFER_PATH, &request(&ids), Some(remote.as_identity()), link)
                    .await
            ),
            true.into()
        );
        let first_attempt = endpoint.shared.lock().unwrap().pending[&link].attempt_id;
        assert_eq!(
            decode(
                &dispatch(&endpoint, OFFER_PATH, &request(&ids), Some(remote.as_identity()), link)
                    .await
            ),
            true.into()
        );
        assert_eq!(endpoint.shared.lock().unwrap().pending[&link].attempt_id, first_attempt);
        let existing = propagated([0x44; 16], 0x44);
        let existing_id = transient_id(&existing);
        insert_queued(&endpoint, queued(existing, 0));
        assert_eq!(
            decode(
                &dispatch(
                    &endpoint,
                    OFFER_PATH,
                    &request(&[[3; 32], existing_id]),
                    Some(remote.as_identity()),
                    AddressHash::new([2; 16])
                )
                .await
            ),
            rmpv::Value::Array(vec![rmpv::Value::Binary(vec![3; 32])])
        );
        assert_eq!(
            decode(
                &dispatch(
                    &endpoint,
                    OFFER_PATH,
                    &request(&[[4; 32], [4; 32]]),
                    Some(remote.as_identity()),
                    AddressHash::new([3; 16])
                )
                .await
            ),
            true.into()
        );
        assert_eq!(
            decode(
                &dispatch(&endpoint, OFFER_PATH, b"bad", Some(remote.as_identity()), link).await
            )
            .as_u64(),
            Some(ERROR_INVALID_DATA)
        );
        assert_eq!(
            decode(&dispatch(&endpoint, OFFER_PATH, &request(&[[5; 32]]), None, link).await)
                .as_u64(),
            Some(ERROR_NO_IDENTITY)
        );
        let too_many = vec![[6; 32]; MAX_OFFER_IDS + 1];
        assert_eq!(
            decode(
                &dispatch(
                    &endpoint,
                    OFFER_PATH,
                    &request(&too_many),
                    Some(remote.as_identity()),
                    link,
                )
                .await
            )
            .as_u64(),
            Some(ERROR_INVALID_DATA)
        );
    }

    #[tokio::test]
    async fn accepted_transfer_is_link_scoped_recomputed_stamp_split_and_deduplicated() {
        let endpoint = endpoint("transfer").await;
        let remote = PrivateIdentity::new_from_name("transfer-remote");
        let link = AddressHash::new([4; 16]);
        let data = propagated([7; 16], 0x31);
        let id = transient_id(&data);
        let offer = offer(&[id]);
        dispatch(&endpoint, OFFER_PATH, &offer, Some(remote.as_identity()), link).await;
        let envelope = transfer([stamped(&data)]);
        let wrong_link_batch = transfer([stamped(&data), stamped(&data)]);
        process(&endpoint, AddressHash::new([5; 16]), &wrong_link_batch);
        assert!(endpoint.queue_snapshot().is_empty());
        process(&endpoint, link, &envelope);
        process(&endpoint, link, &envelope);
        let queued = endpoint.queue_snapshot();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0], (id, data, [0; 32]));
    }

    #[tokio::test]
    async fn direct_single_upload_and_partial_offer_retries_are_preserved() {
        let endpoint = endpoint("direct-and-partial").await;
        let direct = propagated([0x61; 16], 0x61);
        let direct_id = transient_id(&direct);
        let direct_envelope = transfer([stamped(&direct)]);
        assert!(process(&endpoint, AddressHash::new([0x61; 16]), &direct_envelope));
        assert!(process(&endpoint, AddressHash::new([0x61; 16]), &direct_envelope));
        assert_eq!(endpoint.queue_snapshot(), vec![(direct_id, direct, [0; 32])]);

        let remote = PrivateIdentity::new_from_name("partial-offer-remote");
        let link = AddressHash::new([0x63; 16]);
        let first = propagated([0x64; 16], 0x64);
        let second = propagated([0x65; 16], 0x65);
        let first_id = transient_id(&first);
        let second_id = transient_id(&second);
        let offer = offer(&[first_id, second_id]);
        dispatch(&endpoint, OFFER_PATH, &offer, Some(remote.as_identity()), link).await;
        let mut remote_hash = [0; 16];
        remote_hash.copy_from_slice(remote.as_identity().address_hash.as_slice());
        let attempt_id = endpoint.shared.lock().unwrap().pending[&link].attempt_id;
        let now = endpoint.clock.now();
        process_transfer(
            &endpoint.shared,
            &endpoint.store,
            link,
            &transfer([stamped(&first)]),
            endpoint.policy,
            now,
            &endpoint.events,
        );
        assert_eq!(endpoint.shared.lock().unwrap().pending[&link].ids, BTreeSet::from([second_id]));
        {
            let store = endpoint.store.lock().unwrap();
            let state = store.standard_propagation_attempt_state_for_test(attempt_id).unwrap();
            assert_eq!(state, "running");
            assert!(
                store.standard_propagation_checkpoint(remote_hash, "ingress").unwrap().is_none()
            );
        }
        process_transfer(
            &endpoint.shared,
            &endpoint.store,
            link,
            &transfer([stamped(&second)]),
            endpoint.policy,
            now,
            &endpoint.events,
        );
        assert!(!endpoint.shared.lock().unwrap().pending.contains_key(&link));
        assert_eq!(endpoint.queue_snapshot().len(), 3);
        let store = endpoint.store.lock().unwrap();
        let state = store.standard_propagation_attempt_state_for_test(attempt_id).unwrap();
        assert_eq!(state, "completed");
        assert_eq!(
            store
                .standard_propagation_checkpoint(remote_hash, "ingress")
                .unwrap()
                .unwrap()
                .item_count,
            2
        );
    }

    #[tokio::test]
    async fn mixed_offered_batch_withholds_proof_and_remains_retryable() {
        let endpoint = endpoint("mixed-batch").await;
        let remote = PrivateIdentity::new_from_name("mixed-batch-remote");
        let link = AddressHash::new([0x71; 16]);
        let valid = propagated([0x72; 16], 0x72);
        let invalid = vec![0x73; lxmf::propagation::MIN_PROPAGATED_LXMF_BYTES - 1];
        let valid_id = transient_id(&valid);
        let invalid_id = transient_id(&invalid);
        let offer = offer(&[valid_id, invalid_id]);
        dispatch(&endpoint, OFFER_PATH, &offer, Some(remote.as_identity()), link).await;
        let mut remote_hash = [0; 16];
        remote_hash.copy_from_slice(remote.as_identity().address_hash.as_slice());
        let attempt_id = endpoint.shared.lock().unwrap().pending[&link].attempt_id;
        let mixed = transfer([stamped(&valid), invalid]);

        assert!(!process(&endpoint, link, &mixed));
        assert!(endpoint.queue_snapshot().iter().any(|(id, _, _)| *id == valid_id));
        assert_eq!(
            endpoint.shared.lock().unwrap().pending[&link].ids,
            BTreeSet::from([invalid_id])
        );
        let store = endpoint.store.lock().unwrap();
        let state = store.standard_propagation_attempt_state_for_test(attempt_id).unwrap();
        assert_eq!(state, "running");
        assert!(store.standard_propagation_checkpoint(remote_hash, "ingress").unwrap().is_none());
        assert!(
            store
                .standard_propagation_failures(10)
                .unwrap()
                .iter()
                .any(|failure| failure.attempt_id == Some(attempt_id))
        );
    }

    #[tokio::test]
    async fn pending_cap_retry_is_truthful_and_cross_link_state_is_unchanged() {
        let endpoint = endpoint("pending-cap").await;
        let remote = PrivateIdentity::new_from_name("pending-cap-remote");
        let link = AddressHash::new([0x71; 16]);
        let other_link = AddressHash::new([0x72; 16]);
        let retry = [0x73; 32];
        let cross_link = [0x74; 32];
        {
            let mut state = endpoint.shared.lock().unwrap();
            state.pending.insert(link, pending(BTreeSet::from([retry])));
            let mut other = BTreeSet::new();
            other.insert(cross_link);
            for index in 0..endpoint.policy.queue_max_count - 2 {
                let mut id = [0x75; 32];
                id[..8].copy_from_slice(&(index as u64).to_be_bytes());
                other.insert(id);
            }
            state.pending.insert(other_link, pending(other));
            assert_eq!(state.pending_count(), endpoint.policy.queue_max_count);
        }
        let request = offer;
        assert_eq!(
            decode(
                &dispatch(
                    &endpoint,
                    OFFER_PATH,
                    &request(&[retry]),
                    Some(remote.as_identity()),
                    link,
                )
                .await
            ),
            true.into()
        );
        assert_eq!(
            decode(
                &dispatch(
                    &endpoint,
                    OFFER_PATH,
                    &request(&[cross_link, [0x76; 32]]),
                    Some(remote.as_identity()),
                    link,
                )
                .await
            ),
            ERROR_THROTTLED.into()
        );
        assert_eq!(
            endpoint.shared.lock().unwrap().pending_count(),
            endpoint.policy.queue_max_count
        );
    }

    #[tokio::test]
    async fn offer_capacity_accounts_for_queue_pending_retries_bytes_and_expiry() {
        let mut capacity_endpoint = endpoint("offer-capacity").await;
        capacity_endpoint.policy.queue_max_count = 2;
        let remote = PrivateIdentity::new_from_name("offer-capacity-remote");
        let mut remote_hash = [0; 16];
        remote_hash.copy_from_slice(remote.as_identity().address_hash.as_slice());
        let link = AddressHash::new([0x77; 16]);
        let retry = [0x78; 32];
        let unknown = [0x79; 32];
        let item = queued(propagated([0x7a; 16], 0x7a), 0);
        capacity_endpoint.policy.queue_max_bytes = item.stored_size;
        insert_queued(&capacity_endpoint, item);
        {
            let mut state = capacity_endpoint.shared.lock().unwrap();
            state.pending.insert(link, pending(BTreeSet::from([retry])));
        }
        assert_eq!(
            direct_offer(&capacity_endpoint, link, remote_hash, 0, [1; 16], &[retry],),
            true.into()
        );
        assert_eq!(
            direct_offer(&capacity_endpoint, link, remote_hash, 0, [2; 16], &[retry, unknown],)
                .as_u64(),
            Some(ERROR_THROTTLED)
        );
        {
            let state = capacity_endpoint.shared.lock().unwrap();
            assert!(!state.pending[&link].ids.contains(&unknown));
            assert_eq!(state.validated_links.get(&link), Some(&remote_hash));
        }

        let mut limited = endpoint("offer-slots").await;
        limited.policy.queue_max_count = 4;
        let other_link = AddressHash::new([0x7c; 16]);
        insert_queued(&limited, queued(propagated([0x7d; 16], 0x7d), 0));
        {
            let mut state = limited.shared.lock().unwrap();
            state.pending.insert(other_link, pending(BTreeSet::from([[0x7f; 32]])));
        }
        let wanted = [[0x80; 32], [0x81; 32], [0x82; 32]];
        assert_eq!(
            direct_offer(&limited, link, remote_hash, 0, [3; 16], &wanted,),
            rmpv::Value::Array(
                wanted[..2].iter().map(|id| rmpv::Value::Binary(id.to_vec())).collect()
            )
        );
        {
            let limited_state = limited.shared.lock().unwrap();
            let stats = limited
                .store
                .lock()
                .unwrap()
                .standard_propagation_stats(0, limited.policy.storage())
                .unwrap();
            assert_eq!(
                stats.queued_count + limited_state.pending_count(),
                limited.policy.queue_max_count
            );
        }

        let mut expiry = endpoint("offer-expiry-capacity").await;
        expiry.policy.queue_max_count = 1;
        expiry.policy.expiry_secs = 10;
        let expired_data = propagated([0x83; 16], 0x83);
        let mut expired_message = queued(expired_data, 0);
        expired_message.expires_at = 10;
        insert_queued(&expiry, expired_message);
        assert_eq!(
            direct_offer(&expiry, link, remote_hash, 11, [4; 16], &[[0x85; 32]],),
            true.into()
        );
    }

    #[tokio::test]
    async fn ingress_callback_is_weak_rolls_back_and_capacity_rejects() {
        let dropped_endpoint = endpoint("weak-drop").await;
        let callback = ingress_callback(
            &dropped_endpoint.shared,
            &dropped_endpoint.store,
            dropped_endpoint.policy,
            Arc::clone(&dropped_endpoint.clock),
            dropped_endpoint.ingress_lifetime.as_ref().unwrap(),
            Arc::clone(&dropped_endpoint.events),
        );
        drop(dropped_endpoint);
        let context = IngressContext {
            destination: AddressHash::new([0x81; 16]),
            link_id: AddressHash::new([0x82; 16]),
            kind: rns_core::destination::IngressKind::LinkPacket,
        };
        assert!(!callback(b"stale", &context));

        let identity = PrivateIdentity::new_from_name("activation-rollback");
        let mut native = transport(&identity);
        let mut endpoint = StandardPropagationEndpoint::register(
            &mut native,
            identity,
            "Activation rollback",
            Arc::new(StdMutex::new(MessagesStore::in_memory().unwrap())),
        )
        .await
        .unwrap();
        assert!(matches!(
            endpoint
                .activate(
                    Arc::new(crate::transport::null_transport::NullTransport::new()),
                    &native,
                )
                .await,
            Err(StandardPropagationActivationError::AnnounceNotSent(_))
        ));
        assert!(!endpoint.ingress_running());
        endpoint
            .destination
            .lock()
            .await
            .register_ingress_handler(Arc::new(|_, _| false))
            .expect("failed activation must unregister callback");
        endpoint.destination.lock().await.unregister_ingress_handler();

        let direct = propagated([0x83; 16], 0x83);
        let envelope = transfer([stamped(&direct)]);
        endpoint.policy.target_cost = 0;
        endpoint.policy.flexibility = 0;
        endpoint.policy.queue_max_count = 1;
        insert_queued(&endpoint, queued(propagated([0x84; 16], 0x84), endpoint.clock.now()));
        assert!(!process(&endpoint, context.link_id, &envelope));
    }

    #[test]
    fn unrelated_link_closure_does_not_clear_pending_state() {
        let shared = Arc::new(StdMutex::new(PropagationState::default()));
        let link = AddressHash::new([0x91; 16]);
        {
            let mut state = shared.lock().unwrap();
            state.pending.insert(link, pending(BTreeSet::from([[0x92; 32]])));
            state.validated_links.insert(link, [0x90; 16]);
            state.link_throttled.insert(link, 180);
        }
        let destination = hex::encode([0x93; 16]);
        process_lifecycle_event(
            &shared,
            &destination,
            TransportLifecycleEvent::LinkClosed {
                link_id: hex::encode(link.as_slice()),
                peer_hash: hex::encode([0x94; 16]),
                interface: None,
                rtt_ms: None,
                reason: rns_core::transport::destination_ext::link::LinkCloseReason::Teardown,
            },
        );
        {
            let state = shared.lock().unwrap();
            assert!(state.pending.contains_key(&link));
            assert!(state.validated_links.contains_key(&link));
            assert!(state.link_throttled.contains_key(&link));
        }
        process_lifecycle_event(
            &shared,
            &destination,
            TransportLifecycleEvent::LinkClosed {
                link_id: hex::encode(link.as_slice()),
                peer_hash: destination.clone(),
                interface: None,
                rtt_ms: None,
                reason: rns_core::transport::destination_ext::link::LinkCloseReason::Teardown,
            },
        );
        let state = shared.lock().unwrap();
        assert!(!state.pending.contains_key(&link));
        assert!(!state.validated_links.contains_key(&link));
        assert!(!state.link_throttled.contains_key(&link));
        drop(state);
        shared.lock().unwrap().link_throttled.insert(link, 180);
        process_lifecycle_event(&shared, &destination, TransportLifecycleEvent::Disconnected);
        assert!(shared.lock().unwrap().link_throttled.is_empty());
    }

    #[tokio::test]
    async fn authoritative_callback_accepts_offered_packet_and_resource_batches() {
        let endpoint = endpoint("ingress").await;
        let remote = PrivateIdentity::new_from_name("ingress-remote");
        let first = propagated([8; 16], 0x41);
        let second = propagated([9; 16], 0x42);
        let link = AddressHash::new([6; 16]);
        for data in [&first, &second] {
            let offer = offer(&[transient_id(data)]);
            dispatch(&endpoint, OFFER_PATH, &offer, Some(remote.as_identity()), link).await;
            let envelope = transfer([stamped(data)]);
            assert!(process(&endpoint, link, &envelope));
        }
        assert_eq!(endpoint.queue_snapshot().len(), 2);
    }

    #[tokio::test]
    async fn transfer_limit_uses_exact_decimal_kilobyte_boundary() {
        let mut endpoint = endpoint("transfer-boundary").await;
        endpoint.policy.transfer_limit_kb = 1;
        let mut exact = propagated([0xa1; 16], 0xa1);
        exact.resize(952, 0xa1);
        let exact_envelope = transfer([stamped(&exact)]);
        assert!(process(&endpoint, AddressHash::new([0xa2; 16]), &exact_envelope));

        let mut over = propagated([0xa3; 16], 0xa3);
        over.resize(953, 0xa3);
        let over_envelope = transfer([stamped(&over)]);
        assert!(!process(&endpoint, AddressHash::new([0xa4; 16]), &over_envelope));
        endpoint.policy.queue_max_count = 1;
        let capacity_link = AddressHash::new([0xa5; 16]);
        let capacity_data = propagated([0xa6; 16], 0xa6);
        assert!(!process(&endpoint, capacity_link, &transfer([stamped(&capacity_data)])));
        assert!(endpoint.shared.lock().unwrap().link_throttled.contains_key(&capacity_link));
        assert_eq!(endpoint.queue_snapshot().len(), 1);
    }

    #[test]
    fn expiry_boundary_and_queue_bytes_are_exact() {
        let policy = PropagationPolicy::default();
        let data = propagated([0xb1; 16], 0xb1);
        let message = queued(data, 10);
        let stored_size = message.stored_size;
        let mut store = MessagesStore::in_memory().unwrap();
        assert_eq!(
            store
                .standard_propagation_ingest_batch(StandardPropagationIngestRequest {
                    items: &[message],
                    source_peer: None,
                    attempt: StandardPropagationAttemptStatus::Untracked,
                    protocol: StandardPropagationProtocolStatus::Valid,
                    now: 10,
                    policy: policy.storage(),
                })
                .unwrap(),
            StandardPropagationIngestOutcome::Accepted
        );
        let stats =
            store.standard_propagation_stats(10 + policy.expiry_secs, policy.storage()).unwrap();
        assert_eq!(stats.stored_bytes, stored_size);
        let stats =
            store.standard_propagation_stats(11 + policy.expiry_secs, policy.storage()).unwrap();
        assert_eq!(
            stats,
            crate::storage::standard_propagation::StandardPropagationStats {
                queued_count: 0,
                stored_bytes: 0,
            }
        );
    }

    #[tokio::test]
    async fn invalid_transfer_throttles_validated_peer_until_exact_deadline() {
        let endpoint = endpoint("throttle-boundary").await;
        let remote = PrivateIdentity::new_from_name("throttled-peer");
        let mut remote_hash = [0; 16];
        remote_hash.copy_from_slice(remote.as_identity().address_hash.as_slice());
        let link = AddressHash::new([0xc1; 16]);
        let data = propagated([0xc2; 16], 0xc2);
        let id = transient_id(&data);
        let envelope = transfer([stamped(&data)]);
        assert_eq!(direct_offer(&endpoint, link, remote_hash, 0, [0xc0; 16], &[id],), true.into());
        assert!(!process_transfer(
            &endpoint.shared,
            &endpoint.store,
            link,
            b"invalid",
            endpoint.policy,
            0,
            &endpoint.events,
        ));
        {
            let state = endpoint.shared.lock().unwrap();
            assert_eq!(state.link_throttled[&link], endpoint.policy.throttle_secs);
            assert_eq!(state.throttled[&remote_hash], endpoint.policy.throttle_secs);
            assert!(!state.pending.contains_key(&link));
        }
        assert!(endpoint.queue_snapshot().is_empty());
        assert_eq!(
            endpoint.store.lock().unwrap().standard_propagation_failures(10).unwrap().len(),
            1
        );
        assert!(!process_transfer(
            &endpoint.shared,
            &endpoint.store,
            link,
            b"invalid",
            endpoint.policy,
            1,
            &endpoint.events,
        ));
        {
            let state = endpoint.shared.lock().unwrap();
            assert_eq!(state.link_throttled[&link], endpoint.policy.throttle_secs);
            assert_eq!(state.throttled[&remote_hash], endpoint.policy.throttle_secs);
            assert!(!state.pending.contains_key(&link));
        }
        assert!(endpoint.queue_snapshot().is_empty());
        assert_eq!(
            endpoint.store.lock().unwrap().standard_propagation_failures(10).unwrap().len(),
            1
        );
        assert!(!process_transfer(
            &endpoint.shared,
            &endpoint.store,
            link,
            &envelope,
            endpoint.policy,
            endpoint.policy.throttle_secs - 1,
            &endpoint.events,
        ));
        assert_eq!(
            direct_offer(
                &endpoint,
                AddressHash::new([0xc3; 16]),
                remote_hash,
                endpoint.policy.throttle_secs - 1,
                [0xc3; 16],
                &[[0xc4; 32]],
            )
            .as_u64(),
            Some(ERROR_THROTTLED)
        );
        assert_eq!(
            direct_offer(
                &endpoint,
                link,
                remote_hash,
                endpoint.policy.throttle_secs,
                [0xc5; 16],
                &[id],
            ),
            true.into()
        );
        assert!(process_transfer(
            &endpoint.shared,
            &endpoint.store,
            link,
            &envelope,
            endpoint.policy,
            endpoint.policy.throttle_secs,
            &endpoint.events,
        ));
        let state = endpoint.shared.lock().unwrap();
        assert!(!state.link_throttled.contains_key(&link));
        assert!(!state.throttled.contains_key(&remote_hash));
        drop(state);
        assert!(
            endpoint
                .store
                .lock()
                .unwrap()
                .standard_propagation_snapshot(
                    endpoint.policy.throttle_secs,
                    endpoint.policy.storage(),
                )
                .unwrap()
                .iter()
                .any(|item| item.transient_id == id)
        );
    }

    #[tokio::test]
    async fn oversized_correlated_transfer_terminalizes_attempt_once() {
        let mut endpoint = endpoint("oversized-transfer").await;
        endpoint.policy.sync_limit_kb = 1;
        let remote = PrivateIdentity::new_from_name("oversized-peer");
        let mut remote_hash = [0; 16];
        remote_hash.copy_from_slice(remote.as_identity().address_hash.as_slice());
        let link = AddressHash::new([0xce; 16]);
        let id = [0xcf; 32];
        assert_eq!(direct_offer(&endpoint, link, remote_hash, 0, [0xcd; 16], &[id]), true.into());
        let attempt = endpoint.shared.lock().unwrap().pending[&link].attempt_id;

        assert!(!process_transfer(
            &endpoint.shared,
            &endpoint.store,
            link,
            &vec![0; DECIMAL_KB + 1],
            endpoint.policy,
            1,
            &endpoint.events,
        ));
        let mut store = endpoint.store.lock().unwrap();
        assert_eq!(store.standard_propagation_attempt_state_for_test(attempt).unwrap(), "failed");
        assert_eq!(store.standard_propagation_reconcile_deadlines(10_000).unwrap(), 0);
        let failures = store.standard_propagation_failures(10).unwrap();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].code, "sync_limit");
        assert_eq!(failures[0].attempt_id, Some(attempt));
    }

    #[tokio::test]
    async fn fourth_concurrent_offer_link_is_rejected() {
        let endpoint = endpoint("offer-link-cap").await;
        for index in 0..=endpoint.policy.max_offer_links {
            let remote = PrivateIdentity::new_from_name(&format!("offer-peer-{index}"));
            let mut remote_hash = [0; 16];
            remote_hash.copy_from_slice(remote.as_identity().address_hash.as_slice());
            let response = direct_offer(
                &endpoint,
                AddressHash::new([index as u8; 16]),
                remote_hash,
                0,
                [index as u8; 16],
                &[[index as u8; 32]],
            );
            if index < endpoint.policy.max_offer_links {
                assert_eq!(response, true.into());
            } else {
                assert_eq!(response.as_u64(), Some(ERROR_THROTTLED));
            }
        }
    }

    #[test]
    fn production_policy_defaults_match_wire_contract() {
        let policy = PropagationPolicy::default();
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
    }

    #[test]
    fn optional_get_limit_distinguishes_absent_zero_and_decimal_values() {
        assert_eq!(optional_decimal_kb_limit(&rmpv::Value::Nil), None);
        assert_eq!(optional_decimal_kb_limit(&rmpv::Value::from(f64::NAN)), None);
        assert_eq!(optional_decimal_kb_limit(&rmpv::Value::from("bad")), None);
        assert_eq!(optional_decimal_kb_limit(&rmpv::Value::from(0)), Some(0));
        assert_eq!(optional_decimal_kb_limit(&rmpv::Value::from(-2)), Some(0));
        assert_eq!(optional_decimal_kb_limit(&rmpv::Value::from(2)), Some(2_000));
        assert_eq!(optional_decimal_kb_limit(&rmpv::Value::from(1.5)), Some(1_500));
        assert_eq!(optional_decimal_kb_limit(&rmpv::Value::from("2.5")), Some(2_500));
        assert_eq!(optional_decimal_kb_limit(&rmpv::Value::Binary(b"3.5".to_vec())), Some(3_500));
    }

    #[test]
    fn client_get_requests_have_exact_canonical_messagepack_structure() {
        let wants = [[0x11; 32], [0x12; 32]];
        let haves = [[0x21; 32]];
        assert_eq!(
            decode(&get_request(None, None, None)),
            rmpv::Value::Array(vec![rmpv::Value::Nil, rmpv::Value::Nil])
        );
        assert_eq!(
            decode(&get_request(None, Some(&haves), None)),
            rmpv::Value::Array(vec![
                rmpv::Value::Nil,
                rmpv::Value::Array(vec![rmpv::Value::Binary(haves[0].to_vec())]),
            ])
        );
        let limit = download_limit_kb(Some(17));
        assert_eq!(
            decode(&get_request(Some(&wants), None, Some(limit))),
            rmpv::Value::Array(vec![
                rmpv::Value::Array(
                    wants.iter().map(|id| rmpv::Value::Binary(id.to_vec())).collect(),
                ),
                rmpv::Value::Nil,
                rmpv::Value::from(u64::try_from(limit).unwrap()),
            ])
        );
    }

    #[test]
    fn download_limit_is_decimal_kb_and_capped_by_response_decoder_ceiling() {
        let ceiling_kb = MAX_GET_RESPONSE_BYTES / DECIMAL_KB;
        assert!(ceiling_kb >= 1);
        assert_eq!(download_limit_kb(None), ceiling_kb);
        assert_eq!(download_limit_kb(Some(ceiling_kb.saturating_add(1))), ceiling_kb);
        assert_eq!(download_limit_kb(Some(1)), 1);
        assert!(download_limit_kb(None).saturating_mul(DECIMAL_KB) <= MAX_GET_RESPONSE_BYTES);
    }

    #[test]
    fn propagation_response_decoder_accepts_exact_cap_and_rejects_cap_plus_one() {
        let exact = encode(rmpv::Value::Array(vec![rmpv::Value::Binary(vec![
            0x31;
            MAX_GET_RESPONSE_BYTES
                - 6
        ])]));
        assert_eq!(exact.len(), MAX_GET_RESPONSE_BYTES);
        assert_eq!(
            decode_binary_payloads(&exact, 1, MAX_GET_RESPONSE_BYTES).unwrap()[0].len(),
            MAX_GET_RESPONSE_BYTES - 6
        );

        let oversized = encode(rmpv::Value::Array(vec![rmpv::Value::Binary(vec![
            0x32;
            MAX_GET_RESPONSE_BYTES
                - 5
        ])]));
        assert_eq!(oversized.len(), MAX_GET_RESPONSE_BYTES + 1);
        assert!(decode_binary_payloads(&oversized, 1, MAX_GET_RESPONSE_BYTES).is_err());
    }

    #[tokio::test]
    async fn get_is_identified_recipient_isolated_ordered_and_processes_haves_first() {
        let endpoint = endpoint("get").await;
        let first_identity = PrivateIdentity::new_from_name("recipient-one");
        let second_identity = PrivateIdentity::new_from_name("recipient-two");
        let recipient = |identity: &PrivateIdentity| {
            let hash = SingleOutputDestination::new(
                *identity.as_identity(),
                DestinationName::new("lxmf", "delivery"),
            )
            .desc
            .address_hash;
            let mut bytes = [0; 16];
            bytes.copy_from_slice(hash.as_slice());
            bytes
        };
        let mut large = propagated(recipient(&first_identity), 0x51);
        large.extend_from_slice(&[0x51; 32]);
        let mut small = propagated(recipient(&first_identity), 0x52);
        small.truncate(lxmf::propagation::MIN_PROPAGATED_LXMF_BYTES);
        let other = propagated(recipient(&second_identity), 0x53);
        for data in [&large, &small, &other] {
            insert_queued(&endpoint, queued(data.clone(), endpoint.clock.now()));
        }
        let inventory = dispatch(
            &endpoint,
            GET_PATH,
            &encode(rmpv::Value::Array(vec![rmpv::Value::Nil, rmpv::Value::Nil])),
            Some(first_identity.as_identity()),
            AddressHash::new([7; 16]),
        )
        .await;
        let listed = decode(&inventory).as_array().unwrap().clone();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0], rmpv::Value::Binary(transient_id(&small).to_vec()));
        let zero_limited_inventory = dispatch(
            &endpoint,
            GET_PATH,
            &encode(rmpv::Value::Array(vec![rmpv::Value::Nil, rmpv::Value::Nil, 0.into()])),
            Some(first_identity.as_identity()),
            AddressHash::new([7; 16]),
        )
        .await;
        assert_eq!(decode(&zero_limited_inventory).as_array().map(Vec::len), Some(2));

        let request = encode(rmpv::Value::Array(vec![
            rmpv::Value::Array(vec![
                rmpv::Value::Binary(transient_id(&small).to_vec()),
                rmpv::Value::Binary(transient_id(&large).to_vec()),
            ]),
            rmpv::Value::Array(vec![
                rmpv::Value::Binary(transient_id(&small).to_vec()),
                rmpv::Value::Binary(transient_id(&other).to_vec()),
            ]),
        ]));
        let fetched = decode(
            &dispatch(
                &endpoint,
                GET_PATH,
                &request,
                Some(first_identity.as_identity()),
                AddressHash::new([7; 16]),
            )
            .await,
        );
        assert_eq!(fetched, rmpv::Value::Array(vec![rmpv::Value::Binary(large.clone())]));
        assert!(endpoint.queue_snapshot().iter().any(|(id, _, _)| *id == transient_id(&other)));
        assert_eq!(
            decode(&dispatch(&endpoint, GET_PATH, &request, None, AddressHash::new([7; 16])).await)
                .as_u64(),
            Some(ERROR_NO_IDENTITY)
        );

        let mut oversized = propagated(recipient(&first_identity), 0x54);
        oversized.resize(1200, 0x54);
        let oversized_id = transient_id(&oversized);
        insert_queued(&endpoint, queued(oversized, endpoint.clock.now()));
        let bounded = encode(rmpv::Value::Array(vec![
            rmpv::Value::Array(vec![rmpv::Value::Binary(oversized_id.to_vec())]),
            rmpv::Value::Nil,
            1.into(),
        ]));
        assert_eq!(
            decode(
                &dispatch(
                    &endpoint,
                    GET_PATH,
                    &bounded,
                    Some(first_identity.as_identity()),
                    AddressHash::new([7; 16]),
                )
                .await
            ),
            rmpv::Value::Array(Vec::new())
        );
        let small_id = transient_id(&large);
        let oversized_first = encode(rmpv::Value::Array(vec![
            rmpv::Value::Array(vec![
                rmpv::Value::Binary(oversized_id.to_vec()),
                rmpv::Value::Binary(small_id.to_vec()),
            ]),
            rmpv::Value::Nil,
            1.into(),
        ]));
        assert_eq!(
            decode(
                &dispatch(
                    &endpoint,
                    GET_PATH,
                    &oversized_first,
                    Some(first_identity.as_identity()),
                    AddressHash::new([7; 16]),
                )
                .await
            ),
            rmpv::Value::Array(vec![rmpv::Value::Binary(large.clone())])
        );
        let zero_limit = encode(rmpv::Value::Array(vec![
            rmpv::Value::Array(vec![rmpv::Value::Binary(small_id.to_vec())]),
            rmpv::Value::Nil,
            0.into(),
        ]));
        assert_eq!(
            decode(
                &dispatch(
                    &endpoint,
                    GET_PATH,
                    &zero_limit,
                    Some(first_identity.as_identity()),
                    AddressHash::new([7; 16]),
                )
                .await
            ),
            rmpv::Value::Array(Vec::new())
        );
        let malformed_limit = encode(rmpv::Value::Array(vec![
            rmpv::Value::Array(vec![rmpv::Value::Binary(small_id.to_vec())]),
            rmpv::Value::Nil,
            "bad".into(),
        ]));
        assert_eq!(
            decode(
                &dispatch(
                    &endpoint,
                    GET_PATH,
                    &malformed_limit,
                    Some(first_identity.as_identity()),
                    AddressHash::new([7; 16]),
                )
                .await
            ),
            rmpv::Value::Array(vec![rmpv::Value::Binary(large)])
        );
    }

    #[tokio::test]
    async fn durable_queue_fetch_ack_and_ephemeral_link_state_survive_expected_restarts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("messages.db");
        let recipient_identity = PrivateIdentity::new_from_name("durable-recipient");
        let recipient_hash = SingleOutputDestination::new(
            *recipient_identity.as_identity(),
            DestinationName::new("lxmf", "delivery"),
        )
        .desc
        .address_hash;
        let mut recipient = [0u8; 16];
        recipient.copy_from_slice(recipient_hash.as_slice());
        let data = propagated(recipient, 0xd1);
        let id = transient_id(&data);
        {
            let store = Arc::new(StdMutex::new(MessagesStore::open(&path).unwrap()));
            let endpoint = endpoint_with_store("durable-node", store).await;
            assert!(process(&endpoint, AddressHash::new([0xd2; 16]), &transfer([stamped(&data)])));
            let remote = PrivateIdentity::new_from_name("durable-offer-peer");
            assert_eq!(
                decode(
                    &dispatch(
                        &endpoint,
                        OFFER_PATH,
                        &offer(&[[0xd3; 32]]),
                        Some(remote.as_identity()),
                        AddressHash::new([0xd4; 16]),
                    )
                    .await
                ),
                true.into()
            );
            assert_eq!(endpoint.queue_snapshot().len(), 1);
            assert_eq!(endpoint.shared.lock().unwrap().pending_count(), 1);
            endpoint.set_selection(Some([0xd7; 16]), "manual", endpoint.clock.now()).unwrap();
        }
        {
            let store = Arc::new(StdMutex::new(MessagesStore::open(&path).unwrap()));
            let endpoint = endpoint_with_store("durable-node", store).await;
            assert!(endpoint.shared.lock().unwrap().pending.is_empty());
            assert!(endpoint.shared.lock().unwrap().validated_links.is_empty());
            assert_eq!(endpoint.queue_snapshot().len(), 1);
            assert_eq!(endpoint.selection().unwrap().unwrap().peer, Some([0xd7; 16]));
            let fetch = encode(rmpv::Value::Array(vec![
                rmpv::Value::Array(vec![rmpv::Value::Binary(id.to_vec())]),
                rmpv::Value::Nil,
            ]));
            assert_eq!(
                decode(
                    &dispatch(
                        &endpoint,
                        GET_PATH,
                        &fetch,
                        Some(recipient_identity.as_identity()),
                        AddressHash::new([0xd5; 16]),
                    )
                    .await
                ),
                rmpv::Value::Array(vec![rmpv::Value::Binary(data.clone())])
            );
            let acknowledge = encode(rmpv::Value::Array(vec![
                rmpv::Value::Nil,
                rmpv::Value::Array(vec![rmpv::Value::Binary(id.to_vec())]),
            ]));
            dispatch(
                &endpoint,
                GET_PATH,
                &acknowledge,
                Some(recipient_identity.as_identity()),
                AddressHash::new([0xd5; 16]),
            )
            .await;
        }
        {
            let store = Arc::new(StdMutex::new(MessagesStore::open(&path).unwrap()));
            let endpoint = endpoint_with_store("durable-node", store).await;
            assert!(endpoint.queue_snapshot().is_empty());
            assert!(process(&endpoint, AddressHash::new([0xd6; 16]), &transfer([stamped(&data)])));
            assert!(endpoint.queue_snapshot().is_empty());
        }
    }

    #[tokio::test]
    async fn storage_commit_failure_withholds_authoritative_acceptance() {
        let endpoint = endpoint("storage-failure").await;
        endpoint.store.lock().unwrap().standard_propagation_fail_inserts_for_test().unwrap();
        let data = propagated([0xe1; 16], 0xe1);
        let link = AddressHash::new([0xe2; 16]);
        assert!(!process(&endpoint, link, &transfer([stamped(&data)])));
        assert!(endpoint.queue_snapshot().is_empty());
        assert!(endpoint.shared.lock().unwrap().link_throttled.contains_key(&link));
        assert!(
            endpoint.store.lock().unwrap().standard_propagation_failures(10).unwrap().is_empty()
        );
    }
}
