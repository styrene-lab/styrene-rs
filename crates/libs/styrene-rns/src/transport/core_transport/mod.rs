use alloc::sync::Arc;
use announce_limits::AnnounceLimits;
use announce_table::AnnounceTable;
use link_table::LinkTable;
use packet_cache::PacketCache;
use path_requests::PathRequests;
use path_requests::TagBytes;
use path_requests::create_path_request_destination;
use path_table::PathTable;
use rand_core::OsRng;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant, SystemTime};
use tokio::time;
use tokio_util::sync::CancellationToken;

use tokio::sync::Mutex;
use tokio::sync::MutexGuard;
use tokio::sync::broadcast;
use x25519_dalek::PublicKey;

use crate::destination::DestinationAnnounce;
use crate::destination::DestinationDesc;
use crate::destination::DestinationHandleStatus;
use crate::destination::DestinationName;
use crate::destination::SingleInputDestination;
use crate::destination::SingleOutputDestination;
use crate::destination::{IngressContext, IngressHandler, IngressKind};
use crate::destination::{RequestDispatchError, RequestId, RequestPathHash};
use crate::transport::destination_ext::link::Link;
use crate::transport::destination_ext::link::LinkEvent;
use crate::transport::destination_ext::link::LinkEventData;
use crate::transport::destination_ext::link::LinkHandleResult;
use crate::transport::destination_ext::link::LinkId;
use crate::transport::destination_ext::link::LinkStatus;

use crate::hash::{AddressHash, HASH_SIZE, Hash};
use crate::identity::{Identity, PrivateIdentity};
use crate::transport::error::RnsError;

use crate::transport::iface::IngressQueueCapacities;
use crate::transport::iface::IngressSnapshot;
use crate::transport::iface::InterfaceManager;
use crate::transport::iface::InterfaceRxReceiver;
use crate::transport::iface::RxMessage;
use crate::transport::iface::TxDispatchTrace;
use crate::transport::iface::TxMessage;
use crate::transport::iface::TxMessageType;

use crate::packet::DestinationType;
use crate::packet::HeaderType;
use crate::packet::Packet;
use crate::packet::PacketContext;
use crate::packet::PacketDataBuffer;
use crate::packet::PacketType;
use crate::ratchets::{encrypt_for_public_key, now_secs};
use crate::transport::ratchet_store::RatchetStore;
use crate::transport::request::{RequestClock, RequestTracker, SystemRequestClock};
use crate::transport::resource::{
    ResourceEvent, ResourceManager, ResourceStateCounts, build_resource_cache_request_packet,
    build_resource_cancel_packet, build_resource_request_packet,
};
use crate::transport::time::{MonotonicClock, SystemMonotonicClock};

#[allow(dead_code)] // Scaffolded from upstream — awaiting integration into transport loop
mod announce_limits;
pub mod announce_table;
pub mod deadlines;
pub mod discovery;
mod link_table;
mod packet_cache;
mod path_requests;
pub mod path_table;

pub mod test_bridge {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use crate::transport::storage::messages::MessageRecord;

    pub trait InboundTestBridge: Send + Sync {
        fn accept_inbound_for_test(&self, record: MessageRecord) -> std::io::Result<()>;
    }

    thread_local! {
        static BRIDGE: RefCell<HashMap<String, Rc<dyn InboundTestBridge>>> =
            RefCell::new(HashMap::new());
    }

    pub fn reset() {
        BRIDGE.with(|bridge| bridge.borrow_mut().clear());
    }

    pub fn register(identity: impl Into<String>, daemon: Rc<dyn InboundTestBridge>) {
        BRIDGE.with(|bridge| {
            bridge.borrow_mut().insert(identity.into(), daemon);
        });
    }

    pub fn deliver_outbound(record: &MessageRecord) -> bool {
        let daemon = BRIDGE.with(|bridge| bridge.borrow().get(&record.destination).cloned());
        let Some(daemon) = daemon else {
            return false;
        };

        let inbound = MessageRecord {
            id: record.id.clone(),
            source: record.source.clone(),
            destination: record.destination.clone(),
            title: record.title.clone(),
            content: record.content.clone(),
            timestamp: record.timestamp,
            direction: "in".into(),
            fields: record.fields.clone(),
            receipt_status: None,
        };
        daemon.accept_inbound_for_test(inbound).is_ok()
    }
}

// Transport-wide packet tracing remains off by default to keep runtime noise low.
const PACKET_TRACE: bool = false;
pub const PATHFINDER_M: usize = 128; // Max hops

const INTERVAL_LINKS_CHECK: Duration = Duration::from_secs(1);
const INTERVAL_INPUT_LINK_CLEANUP: Duration = Duration::from_secs(20);
const INTERVAL_OLD_ANNOUNCES_RETRANSMIT: Duration = Duration::from_secs(300);
#[allow(dead_code)] // Used when output link restart is implemented
const INTERVAL_OUTPUT_LINK_RESTART: Duration = Duration::from_secs(60);
const INTERVAL_OUTPUT_LINK_REPEAT: Duration = Duration::from_secs(6);
#[allow(dead_code)] // Used when output link keepalive is implemented
const INTERVAL_OUTPUT_LINK_KEEP: Duration = Duration::from_secs(5);
const INTERVAL_IFACE_CLEANUP: Duration = Duration::from_secs(10);
const INTERVAL_PATH_CULL: Duration = Duration::from_secs(5);
const INTERVAL_PROTOCOL_SCHEDULER: Duration = Duration::from_millis(25);
const INTERVAL_ANNOUNCES_RETRANSMIT: Duration = Duration::from_secs(1);
const INTERVAL_KEEP_PACKET_CACHED: Duration = Duration::from_secs(180);
const INTERVAL_PACKET_CACHE_CLEANUP: Duration = Duration::from_secs(90);

// Other constants
const KEEP_ALIVE_REQUEST: u8 = 0xFF;
const KEEP_ALIVE_RESPONSE: u8 = 0xFE;

#[derive(Clone)]
pub struct ReceivedData {
    pub destination: AddressHash,
    pub link_id: Option<LinkId>,
    pub data: PacketDataBuffer,
    pub payload_mode: ReceivedPayloadMode,
    pub ratchet_used: bool,
    pub context: Option<PacketContext>,
    pub request_id: Option<[u8; 16]>,
    pub hops: Option<u8>,
    pub interface: Option<Vec<u8>>,
    /// Hash of the received wire packet for single-destination deliveries, so
    /// the application can prove receipt to the sender the way Reticulum
    /// destinations do. `None` for link-delivered payloads.
    pub packet_hash: Option<[u8; HASH_SIZE]>,
    /// Interface the packet arrived on; proofs are returned the same way.
    pub receiving_iface: Option<AddressHash>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceivedPayloadMode {
    FullWire,
    DestinationStripped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerRequestOutcome {
    Handled(Vec<u8>),
    Denied,
    Malformed,
    PathNotFound,
    RequestTooLarge,
    ResponseTooLarge,
}

impl From<RequestDispatchError> for ServerRequestOutcome {
    fn from(error: RequestDispatchError) -> Self {
        match error {
            RequestDispatchError::PathNotFound => Self::PathNotFound,
            RequestDispatchError::RequestTooLarge => Self::RequestTooLarge,
            RequestDispatchError::Unauthorized => Self::Denied,
            RequestDispatchError::ResponseTooLarge => Self::ResponseTooLarge,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerRequestEvent {
    pub destination: AddressHash,
    pub link_id: LinkId,
    pub request_id: Option<RequestId>,
    pub path_hash: Option<RequestPathHash>,
    pub outcome: ServerRequestOutcome,
}

pub struct TransportConfig {
    name: String,
    identity: PrivateIdentity,
    broadcast: bool,
    retransmit: bool,
    announce_cache_capacity: usize,
    announce_retry_limit: u8,
    announce_queue_len: usize,
    announce_cap: usize,
    path_request_timeout_secs: u64,
    link_proof_timeout_secs: Option<u64>,
    link_proof_timeout_per_hop_secs: u64,
    link_mtu_discovery: bool,
    link_idle_timeout_secs: u64,
    resource_retry_interval_secs: u64,
    resource_retry_limit: u8,
    ratchet_store_path: Option<PathBuf>,
    blackholed_identities: HashSet<AddressHash>,
    ingress_queue_capacities: IngressQueueCapacities,
}

pub struct DeliveryReceipt {
    pub message_id: [u8; 32],
}

impl DeliveryReceipt {
    pub fn new(message_id: [u8; 32]) -> Self {
        Self { message_id }
    }
}

pub trait ReceiptHandler: Send + Sync {
    fn on_receipt(&self, receipt: &DeliveryReceipt);
}

const TERMINAL_RECEIPT_HISTORY_CAPACITY: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DestinationRegistrationError {
    Duplicate(AddressHash),
}

#[derive(Clone)]
pub struct AnnounceEvent {
    pub destination: Arc<Mutex<SingleOutputDestination>>,
    pub app_data: PacketDataBuffer,
    pub ratchet: Option<[u8; crate::destination::RATCHET_LENGTH]>,
    pub name_hash: [u8; crate::destination::NAME_HASH_LENGTH],
    pub hops: u8,
    pub interface: Vec<u8>,
}

const PENDING_PACKET_RECEIPT_CAPACITY: usize = 4096;
const PENDING_PACKET_RECEIPT_TTL: Duration = Duration::from_secs(600);

/// Outbound single packet whose delivery proof may still arrive.
///
/// Canonical Reticulum proofs are addressed to the truncated hash of the proved
/// packet and are implicit (signature only) by default, so the transport must
/// remember the full hash and destination of packets it transmitted in order
/// to validate them. Entries expire and the registry is bounded.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PendingPacketReceipt {
    pub(crate) truncated: AddressHash,
    pub(crate) packet_hash: [u8; HASH_SIZE],
    pub(crate) destination: AddressHash,
    registered_at: Instant,
}

pub(crate) struct TransportHandler {
    config: TransportConfig,
    iface_manager: Arc<Mutex<InterfaceManager>>,
    announce_tx: broadcast::Sender<AnnounceEvent>,
    route_tx: broadcast::Sender<path_table::RouteEvent>,

    path_table: PathTable,
    announce_table: AnnounceTable,
    link_table: LinkTable,
    single_in_destinations: HashMap<AddressHash, Arc<Mutex<SingleInputDestination>>>,
    single_out_destinations: HashMap<AddressHash, Arc<Mutex<SingleOutputDestination>>>,

    announce_limits: AnnounceLimits,

    out_links: HashMap<AddressHash, Arc<Mutex<Link>>>,
    in_links: HashMap<AddressHash, Arc<Mutex<Link>>>,
    terminal_link_history: VecDeque<crate::transport::destination_ext::link::LinkStateSnapshot>,

    packet_cache: Mutex<PacketCache>,

    path_requests: PathRequests,

    link_in_event_tx: broadcast::Sender<LinkEventData>,
    received_data_tx: broadcast::Sender<ReceivedData>,
    ratchet_store: Option<RatchetStore>,

    resource_manager: ResourceManager,
    resource_events_tx: broadcast::Sender<ResourceEvent>,
    server_request_tx: broadcast::Sender<ServerRequestEvent>,
    request_tracker: RequestTracker,
    protocol_clock: Arc<dyn MonotonicClock>,

    fixed_dest_path_requests: AddressHash,

    cancel: CancellationToken,
    supervision: Option<SupervisionOutcome>,
    receipt_handler: Option<Arc<dyn ReceiptHandler>>,
    terminal_receipt_history: VecDeque<[u8; HASH_SIZE]>,
    pending_packet_receipts: VecDeque<PendingPacketReceipt>,
}

impl TransportHandler {
    /// Remember a transmitted single packet so a later proof addressed to its
    /// truncated hash can be validated against the destination identity.
    pub(super) fn register_pending_packet_receipt(
        &mut self,
        packet_hash: [u8; HASH_SIZE],
        destination: AddressHash,
    ) {
        let now = Instant::now();
        self.pending_packet_receipts.retain(|pending| {
            now.duration_since(pending.registered_at) < PENDING_PACKET_RECEIPT_TTL
        });
        if self.pending_packet_receipts.len() >= PENDING_PACKET_RECEIPT_CAPACITY {
            self.pending_packet_receipts.pop_front();
        }
        self.pending_packet_receipts.push_back(PendingPacketReceipt {
            truncated: AddressHash::new_from_hash(&Hash::new(packet_hash)),
            packet_hash,
            destination,
            registered_at: now,
        });
    }

    pub(super) fn pending_packet_receipt(
        &self,
        truncated: &AddressHash,
    ) -> Option<PendingPacketReceipt> {
        let now = Instant::now();
        self.pending_packet_receipts
            .iter()
            .rev()
            .find(|pending| {
                pending.truncated == *truncated
                    && now.duration_since(pending.registered_at) < PENDING_PACKET_RECEIPT_TTL
            })
            .copied()
    }

    fn conclude_receipt(
        &mut self,
        message_id: [u8; HASH_SIZE],
    ) -> Option<(DeliveryReceipt, Arc<dyn ReceiptHandler>)> {
        if self.terminal_receipt_history.contains(&message_id) {
            return None;
        }
        self.pending_packet_receipts.retain(|pending| pending.packet_hash != message_id);
        if self.terminal_receipt_history.len() >= TERMINAL_RECEIPT_HISTORY_CAPACITY {
            self.terminal_receipt_history.pop_front();
        }
        self.terminal_receipt_history.push_back(message_id);
        Some((DeliveryReceipt::new(message_id), self.receipt_handler.clone()?))
    }
}

pub struct Transport {
    name: String,
    link_in_event_tx: broadcast::Sender<LinkEventData>,
    link_out_event_tx: broadcast::Sender<LinkEventData>,
    received_data_tx: broadcast::Sender<ReceivedData>,
    iface_messages_tx: broadcast::Sender<RxMessage>,
    resource_events_tx: broadcast::Sender<ResourceEvent>,
    server_request_tx: broadcast::Sender<ServerRequestEvent>,
    handler: Arc<Mutex<TransportHandler>>,
    iface_manager: Arc<Mutex<InterfaceManager>>,
    cancel: CancellationToken,
    manager_task: StdMutex<Option<tokio::task::JoinHandle<()>>>,
}

#[derive(Clone)]
pub struct TransportChannel {
    pub(crate) handler: Arc<Mutex<TransportHandler>>,
    pub(crate) link_id: AddressHash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendPacketOutcome {
    SentDirect,
    SentBroadcast,
    DroppedMissingDestinationIdentity,
    DroppedCiphertextTooLarge,
    DroppedEncryptFailed,
    DroppedNoRoute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SendPacketTrace {
    pub outcome: SendPacketOutcome,
    pub direct_iface: Option<AddressHash>,
    pub broadcast: bool,
    pub dispatch: TxDispatchTrace,
    /// Hash of the transmitted packet when it was a single data packet that
    /// can be proved by its receiver; `None` for other packets or drops.
    pub packet_hash: Option<[u8; HASH_SIZE]>,
}

// Transport internals are decomposed by concern for testability and bounded change sets.
// announce: announce handling and retransmit scheduling primitives.
mod announce;
// config: transport configuration builders and defaults.
mod config;
// core: construction and minimal high-level transport API methods.
mod core;
// handler: packet send pipeline and routing/encryption outcomes.
mod handler;
// jobs: background maintenance loops and periodic work.
mod jobs;
// links: link lifecycle and link-scoped data/resource operations.
mod links;
pub use links::LinkDispatch;
// path: path request/response forwarding and intermediate handling.
mod path;
// requests: native Reticulum server request decoding and destination dispatch.
mod requests;
mod supervisor;
pub use supervisor::{SupervisionOutcome, WorkerExit, WorkerFailure};
// wire: inbound packet handlers and wire-level packet logic.
mod wire;

#[allow(dead_code)] // Used in trace logging (PACKET_TRACE)
fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(&mut out, "{:02x}", byte);
    }
    out
}

#[cfg(test)]
mod tests;
