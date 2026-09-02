pub mod driver;
pub mod hdlc;
pub mod ifac;
mod ingress;
pub mod kiss;
pub mod rnode;
pub mod serial;
pub mod stream_iface;
pub mod tcp_client;
pub mod tcp_server;
pub mod udp;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::{broadcast, mpsc};
use tokio::task;
use tokio_util::sync::CancellationToken;

use crate::RnsError;
use crate::hash::AddressHash;
use crate::hash::Hash;
use crate::packet::{MAX_HOPS, Packet};

pub use driver::{InterfaceDriver, InterfaceDriverFactory};
pub use ingress::{
    IngressClass, IngressClassSnapshot, IngressEnqueueOutcome, IngressQueueCapacities,
    IngressSnapshot, InterfaceRxReceiver, InterfaceRxSendError, InterfaceRxSender,
};

pub type InterfaceTxSender = mpsc::Sender<TxMessage>;
pub type InterfaceTxReceiver = mpsc::Receiver<TxMessage>;

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum TxMessageType {
    Broadcast(Option<AddressHash>),
    Direct(AddressHash),
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub struct TxMessage {
    pub tx_type: TxMessageType,
    pub packet: Packet,
}

#[derive(Debug, Default, PartialEq, Eq, Copy, Clone)]
pub struct TxDispatchTrace {
    pub matched_ifaces: usize,
    pub sent_ifaces: usize,
    pub failed_ifaces: usize,
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub struct RxMessage {
    pub address: AddressHash,
    pub packet: Packet,
    origin: IngressOrigin,
    mtu: Option<usize>,
    ingress_class: IngressClass,
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
enum IngressOrigin {
    Physical,
    Local,
    Canonical,
}

impl RxMessage {
    pub fn physical(address: AddressHash, packet: Packet, mtu: usize) -> Self {
        Self {
            address,
            packet,
            origin: IngressOrigin::Physical,
            mtu: Some(mtu),
            ingress_class: IngressClass::Data,
        }
    }

    pub fn local(address: AddressHash, packet: Packet) -> Self {
        Self {
            address,
            packet,
            origin: IngressOrigin::Local,
            mtu: None,
            ingress_class: IngressClass::Data,
        }
    }

    pub(crate) fn ingress_limited(mut self) -> Self {
        self.ingress_class = IngressClass::IngressLimited;
        self
    }

    pub(crate) fn ingress_class(&self) -> IngressClass {
        self.ingress_class
    }

    pub fn admit(mut self) -> Result<Self, RnsError> {
        if self.packet.data.is_empty() {
            return Err(RnsError::InvalidArgument);
        }
        match self.origin {
            IngressOrigin::Physical => {
                if self.packet.header.hops >= MAX_HOPS {
                    return Err(RnsError::InvalidArgument);
                }
                if self.mtu.is_some_and(|mtu| {
                    self.packet.to_bytes().map_or(true, |bytes| bytes.len() > mtu)
                }) {
                    return Err(RnsError::InvalidArgument);
                }
                self.packet.header.hops += 1;
                self.origin = IngressOrigin::Canonical;
            }
            IngressOrigin::Local => {
                if self.packet.header.hops >= MAX_HOPS {
                    return Err(RnsError::InvalidArgument);
                }
                self.origin = IngressOrigin::Canonical;
            }
            IngressOrigin::Canonical => {}
        }
        Ok(self)
    }
}

pub struct InterfaceChannel {
    pub address: AddressHash,
    pub rx_channel: InterfaceRxSender,
    pub tx_channel: InterfaceTxReceiver,
    pub stop: CancellationToken,
}

#[derive(Clone)]
pub struct HostInterfaceControl {
    runtime: Arc<InterfaceRuntime>,
    stop: CancellationToken,
}

impl HostInterfaceControl {
    pub fn set_state(&self, state: InterfaceState) {
        self.runtime.set_state(state);
    }

    pub fn close(&self) {
        self.runtime.set_state(InterfaceState::Closed);
        self.stop.cancel();
    }
}

impl InterfaceChannel {
    pub fn make_rx_channel(cap: usize) -> (InterfaceRxSender, InterfaceRxReceiver) {
        InterfaceRxSender::channel(IngressQueueCapacities::uniform(cap), AddressHash::new([0; 16]))
    }

    pub fn make_priority_rx_channel(
        capacities: IngressQueueCapacities,
        path_request_destination: AddressHash,
    ) -> (InterfaceRxSender, InterfaceRxReceiver) {
        InterfaceRxSender::channel(capacities, path_request_destination)
    }

    pub fn make_tx_channel(cap: usize) -> (InterfaceTxSender, InterfaceTxReceiver) {
        mpsc::channel(cap)
    }

    pub fn new(
        rx_channel: InterfaceRxSender,
        tx_channel: InterfaceTxReceiver,
        address: AddressHash,
        stop: CancellationToken,
    ) -> Self {
        Self { address, rx_channel, tx_channel, stop }
    }

    pub fn address(&self) -> &AddressHash {
        &self.address
    }

    pub fn split(self) -> (InterfaceRxSender, InterfaceTxReceiver) {
        (self.rx_channel, self.tx_channel)
    }
}

pub trait Interface {
    fn mtu() -> usize;

    fn bitrate(&self) -> Option<u64> {
        None
    }

    fn hardware_mtu(&self) -> Option<usize> {
        None
    }

    fn supports_link_mtu_discovery(&self) -> bool {
        false
    }

    fn descriptor(&self) -> InterfaceDescriptor {
        InterfaceDescriptor::default()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InterfaceKind {
    TcpServer,
    TcpClient,
    Udp,
    Serial,
    Kiss,
    #[default]
    Unknown,
}

impl InterfaceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TcpServer => "tcp_server",
            Self::TcpClient => "tcp_client",
            Self::Udp => "udp",
            Self::Serial => "serial",
            Self::Kiss => "kiss",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InterfaceMode {
    Full,
    PointToPoint,
    AccessPoint,
    Roaming,
    Boundary,
    Gateway,
    Internal,
    #[default]
    Unknown,
}

impl InterfaceMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::PointToPoint => "point_to_point",
            Self::AccessPoint => "access_point",
            Self::Roaming => "roaming",
            Self::Boundary => "boundary",
            Self::Gateway => "gateway",
            Self::Internal => "internal",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InterfaceState {
    Starting,
    Listening,
    Connecting,
    Connected,
    Active,
    Retrying,
    Closed,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterfaceStateEvent {
    pub state: InterfaceState,
    pub generation: u64,
}

impl InterfaceState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Listening => "listening",
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Active => "active",
            Self::Retrying => "retrying",
            Self::Closed => "closed",
            Self::Unknown => "unknown",
        }
    }

    pub const fn is_online(self) -> bool {
        matches!(self, Self::Listening | Self::Connected | Self::Active)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterfaceEndpoint {
    Socket(SocketAddr),
    Device { path: String, baud_rate: u32 },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InterfaceDescriptor {
    pub kind: InterfaceKind,
    pub mode: InterfaceMode,
    pub local_endpoint: Option<InterfaceEndpoint>,
    pub remote_endpoint: Option<InterfaceEndpoint>,
    pub ingress_control: bool,
    pub egress_control: bool,
    /// The interface serves local client instances of this node. Announces
    /// learned through it are retransmitted even when transport forwarding
    /// is disabled, the same way a shared instance serves its local clients.
    pub shared_instance: bool,
    /// Whether this outgoing interface carries announces whose next hop is an
    /// internal-mode interface. Absent means permissive.
    pub announces_from_internal: Option<bool>,
    /// Whether an announce learned through this boundary-mode interface may
    /// cross to an internal-mode outgoing interface. Absent grants no
    /// override, so such announces stay off internal interfaces.
    pub announces_to_internal: Option<bool>,
}

impl InterfaceDescriptor {
    /// Fill fields a child interface did not set from its parent: the mode
    /// when unknown, the shared-instance marker, and both internal announce
    /// policy flags when absent.
    pub fn inherit_from(&mut self, parent: &InterfaceDescriptor) {
        if self.mode == InterfaceMode::Unknown {
            self.mode = parent.mode;
        }
        self.shared_instance |= parent.shared_instance;
        self.announces_from_internal =
            self.announces_from_internal.or(parent.announces_from_internal);
        self.announces_to_internal = self.announces_to_internal.or(parent.announces_to_internal);
    }
}

/// Internal-interface announce policy for one outgoing interface.
///
/// `next_hop` is the interface a non-local announce was learned through;
/// `None` means a local announcement, which is always permitted. An outgoing
/// interface that explicitly disables announces from internal blocks
/// announces learned through an internal-mode next hop. An internal-mode
/// outgoing interface blocks announces learned through a boundary-mode next
/// hop unless that next hop explicitly permits announces to internal.
pub fn announce_egress_permitted(
    outgoing: &InterfaceDescriptor,
    next_hop: Option<&InterfaceDescriptor>,
) -> bool {
    let Some(next_hop) = next_hop else {
        return true;
    };
    if next_hop.mode == InterfaceMode::Internal && outgoing.announces_from_internal == Some(false) {
        return false;
    }
    if outgoing.mode == InterfaceMode::Internal
        && next_hop.mode == InterfaceMode::Boundary
        && next_hop.announces_to_internal != Some(true)
    {
        return false;
    }
    true
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceSnapshot {
    pub hash: AddressHash,
    pub kind: InterfaceKind,
    pub mode: InterfaceMode,
    pub state: InterfaceState,
    pub local_endpoint: Option<InterfaceEndpoint>,
    pub remote_endpoint: Option<InterfaceEndpoint>,
    pub parent: Option<AddressHash>,
    pub tx_bytes: u64,
    pub rx_bytes: u64,
    pub violations: InterfaceViolationSnapshot,
    pub filters: InterfaceFilterSnapshot,
    pub connected_peers: u32,
    /// Monotonic count of operational connection/listener generations.
    pub generation: u64,
}

#[derive(Debug)]
struct InterfaceRuntimeMetadata {
    descriptor: InterfaceDescriptor,
    bitrate: Option<u64>,
    hardware_mtu: Option<usize>,
    link_mtu_capable: bool,
    state: InterfaceState,
    parent: Option<AddressHash>,
    generation: u64,
    outgoing_path_requests: std::collections::VecDeque<Instant>,
    incoming_path_requests: std::collections::VecDeque<Instant>,
    path_request_ingress_limited_until: Option<Instant>,
    created_at: Instant,
    force_path_request_egress_limit: bool,
}

#[derive(Debug)]
pub(crate) struct InterfaceRuntime {
    metadata: Mutex<InterfaceRuntimeMetadata>,
    state_tx: broadcast::Sender<InterfaceStateEvent>,
}

impl InterfaceRuntime {
    fn new(
        descriptor: InterfaceDescriptor,
        bitrate: Option<u64>,
        hardware_mtu: Option<usize>,
        link_mtu_capable: bool,
        parent: Option<AddressHash>,
        state_tx: broadcast::Sender<InterfaceStateEvent>,
    ) -> Self {
        Self {
            metadata: Mutex::new(InterfaceRuntimeMetadata {
                descriptor,
                bitrate,
                hardware_mtu,
                link_mtu_capable,
                state: InterfaceState::Starting,
                parent,
                generation: 0,
                outgoing_path_requests: std::collections::VecDeque::new(),
                incoming_path_requests: std::collections::VecDeque::new(),
                path_request_ingress_limited_until: None,
                created_at: Instant::now(),
                force_path_request_egress_limit: false,
            }),
            state_tx,
        }
    }

    pub(crate) fn set_state(&self, state: InterfaceState) {
        let mut metadata = self.metadata.lock().expect("interface runtime lock");
        if metadata.state == state {
            return;
        }
        if state.is_online() && !metadata.state.is_online() {
            metadata.generation = metadata.generation.saturating_add(1);
        }
        metadata.state = state;
        let event = InterfaceStateEvent { state, generation: metadata.generation };
        drop(metadata);
        let _ = self.state_tx.send(event);
    }

    pub(crate) fn set_local_endpoint(&self, endpoint: InterfaceEndpoint) {
        self.metadata.lock().expect("interface runtime lock").descriptor.local_endpoint =
            Some(endpoint);
    }

    pub(crate) fn set_remote_endpoint(&self, endpoint: InterfaceEndpoint) {
        self.metadata.lock().expect("interface runtime lock").descriptor.remote_endpoint =
            Some(endpoint);
    }

    pub(crate) fn clear_endpoints(&self) {
        let mut metadata = self.metadata.lock().expect("interface runtime lock");
        metadata.descriptor.local_endpoint = None;
        metadata.descriptor.remote_endpoint = None;
    }

    pub(crate) fn descriptor(&self) -> InterfaceDescriptor {
        self.metadata.lock().expect("interface runtime lock").descriptor.clone()
    }

    /// Hot-apply both internal announce policy flags.
    pub(crate) fn set_announce_policy(
        &self,
        announces_from_internal: Option<bool>,
        announces_to_internal: Option<bool>,
    ) {
        let mut metadata = self.metadata.lock().expect("interface runtime lock");
        metadata.descriptor.announces_from_internal = announces_from_internal;
        metadata.descriptor.announces_to_internal = announces_to_internal;
    }

    fn should_egress_limit_path_request(&self, now: Instant) -> bool {
        let mut metadata = self.metadata.lock().expect("interface runtime lock");
        if metadata.force_path_request_egress_limit {
            return true;
        }
        if !metadata.descriptor.egress_control {
            return false;
        }
        while metadata
            .outgoing_path_requests
            .front()
            .is_some_and(|oldest| now.saturating_duration_since(*oldest) > Duration::from_secs(10))
        {
            metadata.outgoing_path_requests.pop_front();
        }
        if metadata.outgoing_path_requests.len() <= 1 {
            return false;
        }
        let span = now.saturating_duration_since(metadata.outgoing_path_requests[0]);
        !span.is_zero()
            && (metadata.outgoing_path_requests.len() + 1) as f64 / span.as_secs_f64() > 5.0
    }

    fn record_outgoing_path_request(&self, now: Instant) {
        let mut metadata = self.metadata.lock().expect("interface runtime lock");
        metadata.outgoing_path_requests.push_back(now);
        while metadata.outgoing_path_requests.len() > 48 {
            metadata.outgoing_path_requests.pop_front();
        }
    }

    fn record_and_should_ingress_limit_path_request(&self, now: Instant) -> bool {
        let mut metadata = self.metadata.lock().expect("interface runtime lock");
        if !metadata.descriptor.ingress_control {
            return false;
        }
        while metadata
            .incoming_path_requests
            .front()
            .is_some_and(|oldest| now.saturating_duration_since(*oldest) > Duration::from_secs(10))
        {
            metadata.incoming_path_requests.pop_front();
        }
        metadata.incoming_path_requests.push_back(now);
        while metadata.incoming_path_requests.len() > 48 {
            metadata.incoming_path_requests.pop_front();
        }
        if metadata
            .path_request_ingress_limited_until
            .is_some_and(|limited_until| now < limited_until)
        {
            return true;
        }
        if metadata.incoming_path_requests.len() <= 2 {
            return false;
        }
        let span = now.saturating_duration_since(metadata.incoming_path_requests[0]);
        if span.is_zero() {
            return false;
        }
        let threshold = if now.saturating_duration_since(metadata.created_at)
            < Duration::from_secs(2 * 60 * 60)
        {
            3.0
        } else {
            8.0
        };
        let frequency = metadata.incoming_path_requests.len() as f64 / span.as_secs_f64();
        if frequency > threshold {
            metadata.path_request_ingress_limited_until = Some(now + Duration::from_secs(15));
            true
        } else {
            false
        }
    }

    #[cfg(any(test, feature = "testing"))]
    fn force_path_request_egress_limit(&self, limited: bool) {
        self.metadata.lock().expect("interface runtime lock").force_path_request_egress_limit =
            limited;
    }
}

/// Per-interface byte counters for tx/rx traffic.
///
/// Stored as `Arc` so multiple tasks can read without locking
/// the `InterfaceManager` itself. All updates use relaxed ordering
/// — the counters are monotonic diagnostics, not synchronisation primitives.
pub struct InterfaceStats {
    pub tx_bytes: AtomicU64,
    pub rx_bytes: AtomicU64,
    malformed_frame: AtomicU64,
    ifac_failure: AtomicU64,
    invalid_announce: AtomicU64,
    pre_validation_link: AtomicU64,
    excessive_path_request_tags: AtomicU64,
    valid_blackhole: AtomicU64,
    not_next_hop: AtomicU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterfaceDropReason {
    MalformedFrame,
    IfacFailure,
    InvalidAnnounce,
    PreValidationLink,
    ExcessivePathRequestTags,
    ValidBlackhole,
    /// A transported non-announce packet named another transport instance
    /// as its next hop; canonical policy drops it before any state changes.
    NotNextHop,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InterfaceViolationSnapshot {
    pub malformed_frame: u64,
    pub ifac_failure: u64,
    pub invalid_announce: u64,
    pub pre_validation_link: u64,
    pub excessive_path_request_tags: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InterfaceFilterSnapshot {
    pub valid_blackhole: u64,
    pub not_next_hop: u64,
}

impl InterfaceStats {
    pub fn new() -> Self {
        Self {
            tx_bytes: AtomicU64::new(0),
            rx_bytes: AtomicU64::new(0),
            malformed_frame: AtomicU64::new(0),
            ifac_failure: AtomicU64::new(0),
            invalid_announce: AtomicU64::new(0),
            pre_validation_link: AtomicU64::new(0),
            excessive_path_request_tags: AtomicU64::new(0),
            valid_blackhole: AtomicU64::new(0),
            not_next_hop: AtomicU64::new(0),
        }
    }

    fn increment(counter: &AtomicU64) {
        let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            Some(value.saturating_add(1))
        });
    }

    pub fn record_drop(&self, reason: InterfaceDropReason) {
        let counter = match reason {
            InterfaceDropReason::MalformedFrame => &self.malformed_frame,
            InterfaceDropReason::IfacFailure => &self.ifac_failure,
            InterfaceDropReason::InvalidAnnounce => &self.invalid_announce,
            InterfaceDropReason::PreValidationLink => &self.pre_validation_link,
            InterfaceDropReason::ExcessivePathRequestTags => &self.excessive_path_request_tags,
            InterfaceDropReason::ValidBlackhole => &self.valid_blackhole,
            InterfaceDropReason::NotNextHop => &self.not_next_hop,
        };
        Self::increment(counter);
    }

    pub fn snapshot(&self) -> InterfaceStatsSnapshot {
        InterfaceStatsSnapshot {
            tx_bytes: self.tx_bytes.load(Ordering::Relaxed),
            rx_bytes: self.rx_bytes.load(Ordering::Relaxed),
            violations: InterfaceViolationSnapshot {
                malformed_frame: self.malformed_frame.load(Ordering::Relaxed),
                ifac_failure: self.ifac_failure.load(Ordering::Relaxed),
                invalid_announce: self.invalid_announce.load(Ordering::Relaxed),
                pre_validation_link: self.pre_validation_link.load(Ordering::Relaxed),
                excessive_path_request_tags: self
                    .excessive_path_request_tags
                    .load(Ordering::Relaxed),
            },
            filters: InterfaceFilterSnapshot {
                valid_blackhole: self.valid_blackhole.load(Ordering::Relaxed),
                not_next_hop: self.not_next_hop.load(Ordering::Relaxed),
            },
        }
    }
}

impl Default for InterfaceStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Snapshot of per-interface counters returned by
/// [`InterfaceManager::interface_stats`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InterfaceStatsSnapshot {
    pub tx_bytes: u64,
    pub rx_bytes: u64,
    pub violations: InterfaceViolationSnapshot,
    pub filters: InterfaceFilterSnapshot,
}

struct LocalInterface {
    address: AddressHash,
    tx_send: InterfaceTxSender,
    stop: CancellationToken,
    stats: Arc<InterfaceStats>,
    runtime: Arc<InterfaceRuntime>,
}

pub struct InterfaceContext<T: Interface> {
    pub inner: Arc<Mutex<T>>,
    pub channel: InterfaceChannel,
    pub cancel: CancellationToken,
    /// Optional IFAC configuration for this interface. When `Some`, all packets
    /// are wrapped/unwrapped with IFAC authentication at the stream boundary.
    pub ifac: Option<Arc<ifac::IfacConfig>>,
    pub(crate) stats: Arc<InterfaceStats>,
    pub(crate) runtime: Arc<InterfaceRuntime>,
}

pub struct InterfaceManager {
    counter: usize,
    rx_recv: Arc<tokio::sync::Mutex<InterfaceRxReceiver>>,
    rx_send: InterfaceRxSender,
    path_request_destination: AddressHash,
    cancel: CancellationToken,
    ifaces: Vec<LocalInterface>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    /// Shared stats map so callers can look up per-interface counters without
    /// holding the `InterfaceManager` tokio mutex.
    stats_map: Arc<Mutex<HashMap<AddressHash, Arc<InterfaceStats>>>>,
    state_tx: broadcast::Sender<InterfaceStateEvent>,
}

const DEFAULT_IFACE_TX_QUEUE_CAPACITY: usize = 128;
const DEFAULT_IFACE_STATE_CAPACITY: usize = 64;
const IFACE_TX_ENQUEUE_TIMEOUT_MS: u64 = 200;

fn tx_diag_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("STYRENED_DIAGNOSTICS")
            .or_else(|_| std::env::var("RETICULUMD_DIAGNOSTICS"))
            .or_else(|_| std::env::var("RETICULUM_TRANSPORT_DIAGNOSTICS"))
            .ok()
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on" | "debug"
                )
            })
            .unwrap_or(false)
    })
}

impl InterfaceManager {
    pub fn new(rx_cap: usize) -> Self {
        let (rx_send, rx_recv) = InterfaceChannel::make_rx_channel(rx_cap);
        Self::new_with_channel(rx_send, rx_recv, AddressHash::new_empty())
    }

    pub fn new_with_ingress(
        capacities: IngressQueueCapacities,
        path_request_destination: AddressHash,
    ) -> Self {
        let (rx_send, rx_recv) =
            InterfaceChannel::make_priority_rx_channel(capacities, path_request_destination);
        Self::new_with_channel(rx_send, rx_recv, path_request_destination)
    }

    fn new_with_channel(
        rx_send: InterfaceRxSender,
        rx_recv: InterfaceRxReceiver,
        path_request_destination: AddressHash,
    ) -> Self {
        let rx_recv = Arc::new(tokio::sync::Mutex::new(rx_recv));
        let (state_tx, _) = broadcast::channel(DEFAULT_IFACE_STATE_CAPACITY);

        Self {
            counter: 0,
            rx_recv,
            rx_send,
            path_request_destination,
            cancel: CancellationToken::new(),
            ifaces: Vec::new(),
            tasks: Vec::new(),
            stats_map: Arc::new(Mutex::new(HashMap::new())),
            state_tx,
        }
    }

    fn new_channel_with_runtime(
        &mut self,
        tx_cap: usize,
        descriptor: InterfaceDescriptor,
        bitrate: Option<u64>,
        hardware_mtu: Option<usize>,
        link_mtu_capable: bool,
        parent: Option<AddressHash>,
    ) -> InterfaceChannel {
        self.counter += 1;

        let counter_bytes = self.counter.to_le_bytes();
        let address = AddressHash::new_from_hash(&Hash::new_from_slice(&counter_bytes[..]));

        let (tx_send, tx_recv) = InterfaceChannel::make_tx_channel(tx_cap);

        log::debug!("iface: create channel {}", address);

        let stop = CancellationToken::new();
        let stats = Arc::new(InterfaceStats::new());
        let runtime = Arc::new(InterfaceRuntime::new(
            descriptor,
            bitrate,
            hardware_mtu,
            link_mtu_capable,
            parent,
            self.state_tx.clone(),
        ));

        self.stats_map.lock().expect("interface stats lock").insert(address, stats.clone());
        self.ifaces.push(LocalInterface { address, tx_send, stop: stop.clone(), stats, runtime });

        InterfaceChannel { rx_channel: self.rx_send.clone(), tx_channel: tx_recv, address, stop }
    }

    pub fn new_channel(&mut self, tx_cap: usize) -> InterfaceChannel {
        self.new_channel_with_runtime(
            tx_cap,
            InterfaceDescriptor::default(),
            None,
            None,
            false,
            None,
        )
    }

    /// Register a channel whose byte transport and lifecycle are driven by an embedding host.
    pub fn new_host_channel(
        &mut self,
        tx_cap: usize,
        descriptor: InterfaceDescriptor,
    ) -> (InterfaceChannel, HostInterfaceControl) {
        let channel = self.new_channel_with_runtime(tx_cap, descriptor, None, None, false, None);
        let runtime = self.ifaces.last().expect("newly registered host interface").runtime.clone();
        let control = HostInterfaceControl { runtime, stop: channel.stop.clone() };
        (channel, control)
    }

    pub fn new_context<T: Interface>(&mut self, inner: T) -> InterfaceContext<T> {
        self.new_context_with_parent(inner, None)
    }

    fn new_context_with_parent<T: Interface>(
        &mut self,
        inner: T,
        parent: Option<AddressHash>,
    ) -> InterfaceContext<T> {
        let mut descriptor = inner.descriptor();
        if let Some(parent) = parent
            && let Some(parent_descriptor) = self.descriptor_of(&parent)
        {
            descriptor.inherit_from(&parent_descriptor);
        }
        let bitrate = inner.bitrate();
        let hardware_mtu = inner.hardware_mtu();
        let link_mtu_capable = inner.supports_link_mtu_discovery();
        let channel = self.new_channel_with_runtime(
            DEFAULT_IFACE_TX_QUEUE_CAPACITY,
            descriptor,
            bitrate,
            hardware_mtu,
            link_mtu_capable,
            parent,
        );
        let runtime = self.ifaces.last().expect("newly registered interface").runtime.clone();
        let stats = self.ifaces.last().expect("newly registered interface").stats.clone();

        let inner = Arc::new(Mutex::new(inner));

        InterfaceContext::<T> {
            inner: inner.clone(),
            channel,
            cancel: self.cancel.clone(),
            ifac: None,
            stats,
            runtime,
        }
    }

    /// Spawn an interface with an optional IFAC configuration.
    ///
    /// When `ifac` is `Some`, the interface authenticates all packets using the
    /// shared IFAC key. TCP servers should pass their own IFAC config so that
    /// accepted client connections inherit it.
    pub fn spawn_with_ifac<T: Interface, F, R>(
        &mut self,
        inner: T,
        worker: F,
        ifac: Option<Arc<ifac::IfacConfig>>,
    ) -> AddressHash
    where
        F: FnOnce(InterfaceContext<T>) -> R,
        R: std::future::Future<Output = ()> + Send + 'static,
        R::Output: Send + 'static,
    {
        let mut context = self.new_context(inner);
        context.ifac = ifac;
        let address = *context.channel.address();

        self.tasks.push(task::spawn(worker(context)));

        address
    }

    pub fn spawn<T: Interface, F, R>(&mut self, inner: T, worker: F) -> AddressHash
    where
        F: FnOnce(InterfaceContext<T>) -> R,
        R: std::future::Future<Output = ()> + Send + 'static,
        R::Output: Send + 'static,
    {
        let context = self.new_context(inner);
        let address = *context.channel.address();

        self.tasks.push(task::spawn(worker(context)));

        address
    }

    pub fn spawn_child_with_ifac<T: Interface, F, R>(
        &mut self,
        parent: AddressHash,
        inner: T,
        worker: F,
        ifac: Option<Arc<ifac::IfacConfig>>,
    ) -> AddressHash
    where
        F: FnOnce(InterfaceContext<T>) -> R,
        R: std::future::Future<Output = ()> + Send + 'static,
        R::Output: Send + 'static,
    {
        let mut context = self.new_context_with_parent(inner, Some(parent));
        context.ifac = ifac;
        let address = *context.channel.address();
        self.tasks.push(task::spawn(worker(context)));
        address
    }

    pub fn interface_snapshots(&self) -> Vec<InterfaceSnapshot> {
        let metadata: Vec<_> = self
            .ifaces
            .iter()
            .map(|interface| {
                let runtime = interface.runtime.metadata.lock().expect("interface runtime lock");
                (
                    interface.address,
                    runtime.descriptor.clone(),
                    runtime.state,
                    runtime.parent,
                    runtime.generation,
                    interface.stats.snapshot(),
                )
            })
            .collect();
        let mut snapshots: Vec<_> = metadata
            .iter()
            .map(|(hash, descriptor, state, parent, generation, stats)| InterfaceSnapshot {
                hash: *hash,
                kind: descriptor.kind,
                mode: descriptor.mode,
                state: *state,
                local_endpoint: descriptor.local_endpoint.clone(),
                remote_endpoint: descriptor.remote_endpoint.clone(),
                parent: *parent,
                tx_bytes: stats.tx_bytes,
                rx_bytes: stats.rx_bytes,
                violations: stats.violations,
                filters: stats.filters,
                connected_peers: metadata
                    .iter()
                    .filter(|(_, _, child_state, child_parent, _, _)| {
                        *child_parent == Some(*hash) && *child_state == InterfaceState::Connected
                    })
                    .count() as u32,
                generation: *generation,
            })
            .collect();
        snapshots.sort_by_key(|snapshot| snapshot.hash.as_slice().to_vec());
        snapshots
    }

    pub fn subscribe_state_changes(&self) -> broadcast::Receiver<InterfaceStateEvent> {
        self.state_tx.subscribe()
    }

    pub fn descriptor_of(&self, hash: &AddressHash) -> Option<InterfaceDescriptor> {
        self.ifaces
            .iter()
            .find(|interface| interface.address == *hash)
            .map(|interface| interface.runtime.descriptor())
    }

    /// Whether the interface serves local client instances, which keeps
    /// announces learned through it in the retransmission queue on a node
    /// that does not otherwise forward.
    pub fn is_shared_instance(&self, hash: &AddressHash) -> bool {
        self.descriptor_of(hash).is_some_and(|descriptor| descriptor.shared_instance)
    }

    /// Both internal announce policy flags of one interface.
    pub fn announce_policy(&self, hash: &AddressHash) -> Option<(Option<bool>, Option<bool>)> {
        self.descriptor_of(hash).map(|descriptor| {
            (descriptor.announces_from_internal, descriptor.announces_to_internal)
        })
    }

    /// Hot-apply both internal announce policy flags to one interface.
    pub fn set_announce_policy(
        &self,
        hash: &AddressHash,
        announces_from_internal: Option<bool>,
        announces_to_internal: Option<bool>,
    ) -> bool {
        match self.ifaces.iter().find(|interface| interface.address == *hash) {
            Some(interface) => {
                interface
                    .runtime
                    .set_announce_policy(announces_from_internal, announces_to_internal);
                true
            }
            None => false,
        }
    }

    pub fn interface_mode(&self, hash: &AddressHash) -> InterfaceMode {
        self.ifaces
            .iter()
            .find(|interface| interface.address == *hash)
            .map(|interface| {
                interface.runtime.metadata.lock().expect("interface runtime lock").descriptor.mode
            })
            .unwrap_or_default()
    }

    pub fn active_interface_hashes(&self) -> Vec<AddressHash> {
        self.ifaces
            .iter()
            .filter(|interface| !interface.stop.is_cancelled())
            .map(|interface| interface.address)
            .collect()
    }

    pub fn lowest_online_positive_bitrate(&self) -> Option<u64> {
        self.ifaces
            .iter()
            .filter(|interface| !interface.stop.is_cancelled())
            .filter_map(|interface| {
                let metadata = interface.runtime.metadata.lock().expect("interface runtime lock");
                metadata.state.is_online().then_some(metadata.bitrate).flatten()
            })
            .filter(|bitrate| *bitrate > 0)
            .min()
    }

    pub fn online_positive_bitrate(&self, address: &AddressHash) -> Option<u64> {
        self.ifaces
            .iter()
            .find(|interface| interface.address == *address && !interface.stop.is_cancelled())
            .and_then(|interface| {
                let metadata = interface.runtime.metadata.lock().expect("interface runtime lock");
                metadata
                    .state
                    .is_online()
                    .then_some(metadata.bitrate)
                    .flatten()
                    .filter(|bitrate| *bitrate > 0)
            })
    }

    pub fn set_interface_bitrate(&self, address: &AddressHash, bitrate: Option<u64>) -> bool {
        let Some(interface) = self.ifaces.iter().find(|interface| interface.address == *address)
        else {
            return false;
        };
        interface.runtime.metadata.lock().expect("interface runtime lock").bitrate = bitrate;
        true
    }

    pub fn online_link_mtu(&self, address: &AddressHash) -> Option<usize> {
        self.ifaces
            .iter()
            .find(|interface| interface.address == *address && !interface.stop.is_cancelled())
            .and_then(|interface| {
                let metadata = interface.runtime.metadata.lock().expect("interface runtime lock");
                (metadata.state.is_online() && metadata.link_mtu_capable)
                    .then_some(metadata.hardware_mtu)
                    .flatten()
                    .filter(|mtu| *mtu >= crate::packet::MTU)
                    .map(|mtu| mtu.min(crate::packet::MAX_LINK_MTU))
            })
    }

    pub fn set_interface_link_mtu(
        &self,
        address: &AddressHash,
        hardware_mtu: Option<usize>,
        capable: bool,
    ) -> bool {
        let Some(interface) = self.ifaces.iter().find(|interface| interface.address == *address)
        else {
            return false;
        };
        let mut metadata = interface.runtime.metadata.lock().expect("interface runtime lock");
        metadata.hardware_mtu = hardware_mtu;
        metadata.link_mtu_capable = capable;
        true
    }

    #[cfg(test)]
    pub(crate) fn set_interface_state(&self, address: &AddressHash, state: InterfaceState) -> bool {
        let Some(interface) = self.ifaces.iter().find(|interface| interface.address == *address)
        else {
            return false;
        };
        interface.runtime.set_state(state);
        true
    }

    pub fn can_egress_path_request(&self, excluded: Option<AddressHash>) -> bool {
        let now = Instant::now();
        self.ifaces.iter().any(|interface| {
            !interface.stop.is_cancelled()
                && excluded != Some(interface.address)
                && !interface.runtime.should_egress_limit_path_request(now)
        })
    }

    pub fn can_egress_path_request_to(&self, address: &AddressHash) -> bool {
        let now = Instant::now();
        self.ifaces.iter().any(|interface| {
            interface.address == *address
                && !interface.stop.is_cancelled()
                && !interface.runtime.should_egress_limit_path_request(now)
        })
    }

    pub fn classify_path_request_ingress(&self, address: &AddressHash) -> bool {
        self.ifaces
            .iter()
            .find(|interface| interface.address == *address && !interface.stop.is_cancelled())
            .is_some_and(|interface| {
                interface.runtime.record_and_should_ingress_limit_path_request(Instant::now())
            })
    }

    pub fn set_path_request_rate_controls(
        &self,
        address: &AddressHash,
        ingress_control: bool,
        egress_control: bool,
    ) -> bool {
        let Some(interface) = self.ifaces.iter().find(|interface| interface.address == *address)
        else {
            return false;
        };
        let mut metadata = interface.runtime.metadata.lock().expect("interface runtime lock");
        metadata.descriptor.ingress_control = ingress_control;
        metadata.descriptor.egress_control = egress_control;
        true
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn force_path_request_egress_limit_for_test(
        &self,
        address: &AddressHash,
        limited: bool,
    ) -> bool {
        let Some(interface) = self.ifaces.iter().find(|interface| interface.address == *address)
        else {
            return false;
        };
        interface.runtime.force_path_request_egress_limit(limited);
        true
    }

    /// Cancel one owned interface without stopping the transport.
    #[cfg(feature = "testing")]
    pub fn cancel_interface_for_test(&self, hash: &AddressHash) -> bool {
        if let Some(interface) = self.ifaces.iter().find(|interface| interface.address == *hash) {
            interface.stop.cancel();
            true
        } else {
            false
        }
    }

    pub fn receiver(&self) -> Arc<tokio::sync::Mutex<InterfaceRxReceiver>> {
        self.rx_recv.clone()
    }

    pub fn ingress_snapshot(&self) -> IngressSnapshot {
        self.rx_send.snapshot()
    }

    pub(crate) fn ingress_sender(&self) -> InterfaceRxSender {
        self.rx_send.clone()
    }

    pub fn cleanup(&mut self) {
        let mut map = self.stats_map.lock().expect("interface stats lock");
        self.ifaces.retain(|iface| {
            let alive = !iface.stop.is_cancelled();
            if !alive {
                map.remove(&iface.address);
            }
            alive
        });
    }

    /// Cancel the manager and every interface currently attached to it.
    pub fn shutdown(&self) {
        self.cancel.cancel();
        for interface in &self.ifaces {
            interface.stop.cancel();
        }
    }

    /// Clone the manager cancellation token for failure-path ownership guards.
    pub fn shutdown_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// Transfer ownership of spawned interface tasks to the shutdown caller.
    pub fn take_tasks(&mut self) -> Vec<tokio::task::JoinHandle<()>> {
        std::mem::take(&mut self.tasks)
    }

    /// Abort retained interface tasks when asynchronous cleanup is unavailable.
    pub fn abort_tasks(&self) {
        for task in &self.tasks {
            task.abort();
        }
    }

    pub async fn send(&self, message: TxMessage) -> TxDispatchTrace {
        let mut trace = TxDispatchTrace::default();
        let pkt_bytes = message.packet.data.len() as u64;
        let is_path_request = message.packet.header.packet_type == crate::packet::PacketType::Data
            && message.packet.destination == self.path_request_destination;
        // A rebroadcast announce carries the interface it was learned through;
        // the internal-interface policy is decided per outgoing interface.
        let announce_next_hop = match message.tx_type {
            TxMessageType::Broadcast(Some(address))
                if message.packet.header.packet_type == crate::packet::PacketType::Announce =>
            {
                self.descriptor_of(&address)
            }
            _ => None,
        };
        for iface in &self.ifaces {
            let should_send = match message.tx_type {
                TxMessageType::Broadcast(address) => {
                    let mut should_send = true;
                    if let Some(address) = address {
                        should_send = address != iface.address;
                    }
                    if should_send && let Some(next_hop) = announce_next_hop.as_ref() {
                        should_send =
                            announce_egress_permitted(&iface.runtime.descriptor(), Some(next_hop));
                    }

                    should_send
                }
                TxMessageType::Direct(address) => address == iface.address,
            };

            let now = Instant::now();
            if should_send
                && !iface.stop.is_cancelled()
                && !(is_path_request && iface.runtime.should_egress_limit_path_request(now))
            {
                trace.matched_ifaces += 1;
                match iface.tx_send.try_send(message) {
                    Ok(()) => {
                        trace.sent_ifaces += 1;
                        iface.stats.tx_bytes.fetch_add(pkt_bytes, Ordering::Relaxed);
                        if is_path_request {
                            iface.runtime.record_outgoing_path_request(now);
                        }
                    }
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        match tokio::time::timeout(
                            Duration::from_millis(IFACE_TX_ENQUEUE_TIMEOUT_MS),
                            iface.tx_send.send(message),
                        )
                        .await
                        {
                            Ok(Ok(())) => {
                                trace.sent_ifaces += 1;
                                iface.stats.tx_bytes.fetch_add(pkt_bytes, Ordering::Relaxed);
                                if is_path_request {
                                    iface.runtime.record_outgoing_path_request(now);
                                }
                                if tx_diag_enabled() {
                                    log::warn!(
                                        "iface: recovered from full tx queue on {} for {:?}",
                                        iface.address,
                                        message.tx_type
                                    );
                                }
                            }
                            Ok(Err(_)) => {
                                trace.failed_ifaces += 1;
                                log::warn!(
                                    "iface: tx queue closed on {} for {:?}",
                                    iface.address,
                                    message.tx_type
                                );
                            }
                            Err(_) => {
                                trace.failed_ifaces += 1;
                                log::warn!(
                                    "iface: tx queue full timeout on {} for {:?}",
                                    iface.address,
                                    message.tx_type
                                );
                            }
                        }
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        trace.failed_ifaces += 1;
                        log::warn!(
                            "iface: tx queue closed on {} for {:?}",
                            iface.address,
                            message.tx_type
                        );
                    }
                }
            }
        }

        trace
    }

    /// Record received bytes for an interface (called from the transport loop
    /// when an `RxMessage` arrives).
    pub fn record_rx(&self, address: &AddressHash, bytes: u64) {
        if let Some(stats) = self.stats_map.lock().expect("interface stats lock").get(address) {
            stats.rx_bytes.fetch_add(bytes, Ordering::Relaxed);
        }
    }

    pub fn record_drop(&self, address: &AddressHash, reason: InterfaceDropReason) {
        if let Some(stats) = self.stats_map.lock().expect("interface stats lock").get(address) {
            stats.record_drop(reason);
        }
    }

    /// Return a snapshot of per-interface byte counters.
    pub fn interface_stats(&self) -> HashMap<AddressHash, InterfaceStatsSnapshot> {
        self.stats_map
            .lock()
            .expect("interface stats lock")
            .iter()
            .map(|(addr, stats)| (*addr, stats.snapshot()))
            .collect()
    }

    /// Return the shared stats map so callers can read counters without
    /// holding the `InterfaceManager` tokio mutex.
    pub fn stats_map(&self) -> Arc<Mutex<HashMap<AddressHash, Arc<InterfaceStats>>>> {
        self.stats_map.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct BitrateMatrix {
        online_bitrate_selection: Vec<BitrateSelectionCase>,
    }

    #[derive(Deserialize)]
    struct BitrateSelectionCase {
        interfaces: Vec<BitrateInterfaceCase>,
        expected_lowest: Option<u64>,
    }

    #[derive(Deserialize)]
    struct BitrateInterfaceCase {
        online: bool,
        bitrate: Option<u64>,
    }

    #[test]
    fn shutdown_cancels_manager_and_local_interfaces() {
        let mut manager = InterfaceManager::new(1);
        let channel = manager.new_channel(1);
        let local_stop = channel.stop.clone();

        manager.shutdown();

        assert!(manager.cancel.is_cancelled());
        assert!(local_stop.is_cancelled());
    }

    #[tokio::test]
    async fn path_request_egress_is_rechecked_immediately_before_dispatch() {
        let path_destination = AddressHash::new([0x44; 16]);
        let mut manager =
            InterfaceManager::new_with_ingress(IngressQueueCapacities::default(), path_destination);
        let mut channel = manager.new_channel(1);
        assert!(manager.can_egress_path_request(None));
        assert!(manager.force_path_request_egress_limit_for_test(&channel.address, true));

        let message = TxMessage {
            tx_type: TxMessageType::Direct(channel.address),
            packet: Packet {
                header: crate::packet::Header {
                    packet_type: crate::packet::PacketType::Data,
                    ..Default::default()
                },
                destination: path_destination,
                data: crate::packet::PacketDataBuffer::new_from_slice(b"path request"),
                ..Default::default()
            },
        };
        let trace = manager.send(message).await;

        assert_eq!(trace.sent_ifaces, 0);
        assert!(channel.tx_channel.try_recv().is_err());
    }

    #[test]
    fn path_request_rate_controls_are_disabled_by_default() {
        let (state_tx, _state_rx) = broadcast::channel(1);
        let runtime = InterfaceRuntime::new(
            InterfaceDescriptor::default(),
            None,
            None,
            false,
            None,
            state_tx,
        );
        let now = Instant::now();
        for offset in 0..20 {
            assert!(
                !runtime.record_and_should_ingress_limit_path_request(
                    now + Duration::from_millis(offset)
                )
            );
            runtime.record_outgoing_path_request(now + Duration::from_millis(offset));
        }
        assert!(!runtime.should_egress_limit_path_request(now + Duration::from_millis(20)));
    }

    #[test]
    fn path_request_ingress_burst_activates_and_expires_limiting() {
        let (state_tx, _state_rx) = broadcast::channel(1);
        let runtime = InterfaceRuntime::new(
            InterfaceDescriptor { ingress_control: true, ..Default::default() },
            None,
            None,
            false,
            None,
            state_tx,
        );
        let now = Instant::now();
        assert!(!runtime.record_and_should_ingress_limit_path_request(now));
        assert!(
            !runtime.record_and_should_ingress_limit_path_request(now + Duration::from_millis(100))
        );
        assert!(
            runtime.record_and_should_ingress_limit_path_request(now + Duration::from_millis(200))
        );
        assert!(
            !runtime.record_and_should_ingress_limit_path_request(now + Duration::from_secs(16))
        );
    }

    #[test]
    fn bitrate_queries_track_online_runtime_metadata() {
        let mut manager = InterfaceManager::new(1);
        let online_slow = manager.new_channel(1).address;
        let online_fast = manager.new_channel(1).address;
        let offline = manager.new_channel(1).address;
        let missing = manager.new_channel(1).address;

        assert!(manager.set_interface_bitrate(&online_slow, Some(500)));
        assert!(manager.set_interface_bitrate(&online_fast, Some(1_000)));
        assert!(manager.set_interface_bitrate(&offline, Some(5)));
        assert!(manager.set_interface_bitrate(&missing, Some(0)));
        for address in [online_slow, online_fast, missing] {
            manager
                .ifaces
                .iter()
                .find(|interface| interface.address == address)
                .expect("registered interface")
                .runtime
                .set_state(InterfaceState::Active);
        }

        assert_eq!(manager.lowest_online_positive_bitrate(), Some(500));
        assert_eq!(manager.online_positive_bitrate(&online_fast), Some(1_000));
        assert_eq!(manager.online_positive_bitrate(&offline), None);
        assert_eq!(manager.online_positive_bitrate(&missing), None);

        manager
            .ifaces
            .iter()
            .find(|interface| interface.address == online_slow)
            .expect("registered interface")
            .runtime
            .set_state(InterfaceState::Retrying);
        assert_eq!(manager.lowest_online_positive_bitrate(), Some(1_000));
        assert!(manager.set_interface_bitrate(&online_fast, None));
        assert_eq!(manager.lowest_online_positive_bitrate(), None);
    }

    #[test]
    fn link_mtu_queries_require_online_capable_bounded_metadata() {
        let mut manager = InterfaceManager::new(1);
        let supported = manager.new_channel(1).address;
        let unsupported = manager.new_channel(1).address;
        let offline = manager.new_channel(1).address;

        assert!(manager.set_interface_link_mtu(&supported, Some(1280), true));
        assert!(manager.set_interface_link_mtu(&unsupported, Some(2048), false));
        assert!(manager.set_interface_link_mtu(&offline, Some(1024), true));
        for address in [supported, unsupported] {
            manager
                .ifaces
                .iter()
                .find(|interface| interface.address == address)
                .expect("registered interface")
                .runtime
                .set_state(InterfaceState::Active);
        }

        assert_eq!(manager.online_link_mtu(&supported), Some(1280));
        assert_eq!(manager.online_link_mtu(&unsupported), None);
        assert_eq!(manager.online_link_mtu(&offline), None);

        assert!(manager.set_interface_link_mtu(&supported, Some(0), true));
        assert_eq!(manager.online_link_mtu(&supported), None);
        assert!(manager.set_interface_link_mtu(&supported, Some(4096), true));
        assert_eq!(manager.online_link_mtu(&supported), Some(crate::packet::MAX_LINK_MTU));
    }

    #[test]
    fn lowest_online_bitrate_matches_pinned_reticulum_matrix() {
        let matrix: BitrateMatrix = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../tests/interop/fixtures/rns/rns-1.5.1/bitrate-deadlines.json"
        )))
        .expect("valid canonical bitrate matrix");

        for case in matrix.online_bitrate_selection {
            let mut manager = InterfaceManager::new(1);
            for configured in case.interfaces {
                let address = manager.new_channel(1).address;
                assert!(manager.set_interface_bitrate(&address, configured.bitrate));
                manager
                    .ifaces
                    .iter()
                    .find(|interface| interface.address == address)
                    .expect("registered interface")
                    .runtime
                    .set_state(if configured.online {
                        InterfaceState::Active
                    } else {
                        InterfaceState::Retrying
                    });
            }
            assert_eq!(manager.lowest_online_positive_bitrate(), case.expected_lowest);
        }
    }

    fn descriptor(mode: InterfaceMode) -> InterfaceDescriptor {
        InterfaceDescriptor { mode, ..Default::default() }
    }

    #[test]
    fn internal_announce_policy_decision_table() {
        let internal = descriptor(InterfaceMode::Internal);
        let boundary = descriptor(InterfaceMode::Boundary);
        let full = descriptor(InterfaceMode::Full);
        let permit_to_internal = InterfaceDescriptor {
            announces_to_internal: Some(true),
            ..descriptor(InterfaceMode::Boundary)
        };
        let deny_to_internal = InterfaceDescriptor {
            announces_to_internal: Some(false),
            ..descriptor(InterfaceMode::Boundary)
        };
        let full_no_from_internal = InterfaceDescriptor {
            announces_from_internal: Some(false),
            ..descriptor(InterfaceMode::Full)
        };
        let full_from_internal = InterfaceDescriptor {
            announces_from_internal: Some(true),
            ..descriptor(InterfaceMode::Full)
        };
        let internal_no_from_internal = InterfaceDescriptor {
            announces_from_internal: Some(false),
            ..descriptor(InterfaceMode::Internal)
        };

        let rows: [(&str, &InterfaceDescriptor, Option<&InterfaceDescriptor>, bool); 11] = [
            ("local announce on any interface", &internal_no_from_internal, None, true),
            ("absent from-internal is permissive", &full, Some(&internal), true),
            ("explicit from-internal true permits", &full_from_internal, Some(&internal), true),
            (
                "explicit from-internal false blocks internal next hop",
                &full_no_from_internal,
                Some(&internal),
                false,
            ),
            (
                "from-internal false ignores non-internal next hop",
                &full_no_from_internal,
                Some(&full),
                true,
            ),
            (
                "internal outgoing blocks boundary next hop by default",
                &internal,
                Some(&boundary),
                false,
            ),
            (
                "internal outgoing blocks boundary next hop denying override",
                &internal,
                Some(&deny_to_internal),
                false,
            ),
            (
                "internal outgoing accepts boundary next hop with override",
                &internal,
                Some(&permit_to_internal),
                true,
            ),
            ("internal outgoing accepts full next hop", &internal, Some(&full), true),
            ("full outgoing accepts boundary next hop", &full, Some(&boundary), true),
            ("internal to internal stays permissive by default", &internal, Some(&internal), true),
        ];
        for (row, outgoing, next_hop, expected) in rows {
            assert_eq!(announce_egress_permitted(outgoing, next_hop), expected, "{row}");
        }
    }

    struct PlainInterface;

    impl Interface for PlainInterface {
        fn mtu() -> usize {
            500
        }
    }

    #[test]
    fn announce_policy_is_carried_from_startup_through_children_and_hot_apply() {
        let mut manager = InterfaceManager::new(4);
        let (parent, _control) = manager.new_host_channel(
            4,
            InterfaceDescriptor {
                mode: InterfaceMode::Internal,
                announces_from_internal: Some(false),
                announces_to_internal: None,
                shared_instance: true,
                ..Default::default()
            },
        );
        assert_eq!(manager.announce_policy(&parent.address), Some((Some(false), None)));
        assert!(manager.is_shared_instance(&parent.address));

        let child = manager.new_context_with_parent(PlainInterface, Some(parent.address));
        let child_address = *child.channel.address();
        assert_eq!(manager.interface_mode(&child_address), InterfaceMode::Internal);
        assert_eq!(manager.announce_policy(&child_address), Some((Some(false), None)));
        assert!(manager.is_shared_instance(&child_address));

        assert!(manager.set_announce_policy(&parent.address, None, Some(true)));
        assert_eq!(manager.announce_policy(&parent.address), Some((None, Some(true))));
        assert_eq!(
            manager.announce_policy(&child_address),
            Some((Some(false), None)),
            "hot apply changes only the addressed interface"
        );
        assert!(!manager.set_announce_policy(&AddressHash::new([9; 16]), None, None));
        assert_eq!(manager.announce_policy(&AddressHash::new([9; 16])), None);
    }

    fn rebroadcast_announce(next_hop: AddressHash) -> TxMessage {
        TxMessage {
            tx_type: TxMessageType::Broadcast(Some(next_hop)),
            packet: Packet {
                header: crate::packet::Header {
                    packet_type: crate::packet::PacketType::Announce,
                    ..Default::default()
                },
                destination: AddressHash::new([0x51; 16]),
                data: crate::packet::PacketDataBuffer::new_from_slice(b"announce"),
                ..Default::default()
            },
        }
    }

    #[tokio::test]
    async fn announce_rebroadcast_applies_the_internal_policy_per_outgoing_interface() {
        let mut manager = InterfaceManager::new(4);
        let (internal_hop, _a) = manager.new_host_channel(4, descriptor(InterfaceMode::Internal));
        let (boundary_hop, _b) = manager.new_host_channel(4, descriptor(InterfaceMode::Boundary));
        let (mut full_out, _c) = manager.new_host_channel(4, descriptor(InterfaceMode::Full));
        let (mut internal_out, _d) =
            manager.new_host_channel(4, descriptor(InterfaceMode::Internal));

        let trace = manager.send(rebroadcast_announce(internal_hop.address)).await;
        assert_eq!(trace.sent_ifaces, 3, "permissive default sends everywhere but the hop");
        assert!(full_out.tx_channel.try_recv().is_ok());
        assert!(internal_out.tx_channel.try_recv().is_ok());

        assert!(manager.set_announce_policy(&full_out.address, Some(false), None));
        let trace = manager.send(rebroadcast_announce(internal_hop.address)).await;
        assert_eq!(trace.sent_ifaces, 2);
        assert!(full_out.tx_channel.try_recv().is_err(), "from-internal false blocks the announce");
        assert!(internal_out.tx_channel.try_recv().is_ok());

        let trace = manager.send(rebroadcast_announce(boundary_hop.address)).await;
        assert_eq!(trace.sent_ifaces, 1, "both internal-mode interfaces block the boundary hop");
        assert!(full_out.tx_channel.try_recv().is_ok(), "the boundary hop is not internal");
        assert!(
            internal_out.tx_channel.try_recv().is_err(),
            "boundary to internal needs an override"
        );

        assert!(manager.set_announce_policy(&boundary_hop.address, None, Some(true)));
        let trace = manager.send(rebroadcast_announce(boundary_hop.address)).await;
        assert_eq!(trace.sent_ifaces, 3);
        assert!(internal_out.tx_channel.try_recv().is_ok(), "the override admits the announce");

        let local = TxMessage {
            tx_type: TxMessageType::Broadcast(None),
            ..rebroadcast_announce(boundary_hop.address)
        };
        assert!(manager.set_announce_policy(&boundary_hop.address, None, None));
        let trace = manager.send(local).await;
        assert_eq!(trace.sent_ifaces, 4, "local announcements ignore the policy");
    }
}
