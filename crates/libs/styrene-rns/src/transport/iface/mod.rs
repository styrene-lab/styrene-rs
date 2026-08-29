pub mod driver;
pub mod hdlc;
pub mod ifac;
pub mod kiss;
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
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task;
use tokio_util::sync::CancellationToken;

use crate::RnsError;
use crate::hash::AddressHash;
use crate::hash::Hash;
use crate::packet::{MAX_HOPS, Packet};

pub use driver::{InterfaceDriver, InterfaceDriverFactory};

pub type InterfaceTxSender = mpsc::Sender<TxMessage>;
pub type InterfaceTxReceiver = mpsc::Receiver<TxMessage>;

pub type InterfaceRxSender = mpsc::Sender<RxMessage>;
pub type InterfaceRxReceiver = mpsc::Receiver<RxMessage>;

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
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
enum IngressOrigin {
    Physical,
    Local,
    Canonical,
}

impl RxMessage {
    pub fn physical(address: AddressHash, packet: Packet, mtu: usize) -> Self {
        Self { address, packet, origin: IngressOrigin::Physical, mtu: Some(mtu) }
    }

    pub fn local(address: AddressHash, packet: Packet) -> Self {
        Self { address, packet, origin: IngressOrigin::Local, mtu: None }
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

impl InterfaceChannel {
    pub fn make_rx_channel(cap: usize) -> (InterfaceRxSender, InterfaceRxReceiver) {
        mpsc::channel(cap)
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
    pub connected_peers: u32,
    /// Monotonic count of operational connection/listener generations.
    pub generation: u64,
}

#[derive(Debug)]
struct InterfaceRuntimeMetadata {
    descriptor: InterfaceDescriptor,
    state: InterfaceState,
    parent: Option<AddressHash>,
    generation: u64,
}

#[derive(Debug)]
pub(crate) struct InterfaceRuntime {
    metadata: Mutex<InterfaceRuntimeMetadata>,
}

impl InterfaceRuntime {
    fn new(descriptor: InterfaceDescriptor, parent: Option<AddressHash>) -> Self {
        Self {
            metadata: Mutex::new(InterfaceRuntimeMetadata {
                descriptor,
                state: InterfaceState::Starting,
                parent,
                generation: 0,
            }),
        }
    }

    pub(crate) fn set_state(&self, state: InterfaceState) {
        let mut metadata = self.metadata.lock().expect("interface runtime lock");
        let operational = |value| {
            matches!(
                value,
                InterfaceState::Listening | InterfaceState::Connected | InterfaceState::Active
            )
        };
        if operational(state) && !operational(metadata.state) {
            metadata.generation = metadata.generation.saturating_add(1);
        }
        metadata.state = state;
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
}

/// Per-interface byte counters for tx/rx traffic.
///
/// Stored as `Arc` so multiple tasks can read without locking
/// the `InterfaceManager` itself. All updates use relaxed ordering
/// — the counters are monotonic diagnostics, not synchronisation primitives.
pub struct InterfaceStats {
    pub tx_bytes: AtomicU64,
    pub rx_bytes: AtomicU64,
}

impl InterfaceStats {
    pub fn new() -> Self {
        Self { tx_bytes: AtomicU64::new(0), rx_bytes: AtomicU64::new(0) }
    }
}

impl Default for InterfaceStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Snapshot of per-interface byte counters returned by
/// [`InterfaceManager::interface_stats`].
#[derive(Debug, Clone, Copy, Default)]
pub struct InterfaceStatsSnapshot {
    pub tx_bytes: u64,
    pub rx_bytes: u64,
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
    pub(crate) runtime: Arc<InterfaceRuntime>,
}

pub struct InterfaceManager {
    counter: usize,
    rx_recv: Arc<tokio::sync::Mutex<InterfaceRxReceiver>>,
    rx_send: InterfaceRxSender,
    cancel: CancellationToken,
    ifaces: Vec<LocalInterface>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    /// Shared stats map so callers can look up per-interface counters without
    /// holding the `InterfaceManager` tokio mutex.
    stats_map: Arc<Mutex<HashMap<AddressHash, Arc<InterfaceStats>>>>,
}

const DEFAULT_IFACE_TX_QUEUE_CAPACITY: usize = 128;
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
        let rx_recv = Arc::new(tokio::sync::Mutex::new(rx_recv));

        Self {
            counter: 0,
            rx_recv,
            rx_send,
            cancel: CancellationToken::new(),
            ifaces: Vec::new(),
            tasks: Vec::new(),
            stats_map: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn new_channel_with_runtime(
        &mut self,
        tx_cap: usize,
        descriptor: InterfaceDescriptor,
        parent: Option<AddressHash>,
    ) -> InterfaceChannel {
        self.counter += 1;

        let counter_bytes = self.counter.to_le_bytes();
        let address = AddressHash::new_from_hash(&Hash::new_from_slice(&counter_bytes[..]));

        let (tx_send, tx_recv) = InterfaceChannel::make_tx_channel(tx_cap);

        log::debug!("iface: create channel {}", address);

        let stop = CancellationToken::new();
        let stats = Arc::new(InterfaceStats::new());
        let runtime = Arc::new(InterfaceRuntime::new(descriptor, parent));

        self.stats_map.lock().expect("interface stats lock").insert(address, stats.clone());
        self.ifaces.push(LocalInterface { address, tx_send, stop: stop.clone(), stats, runtime });

        InterfaceChannel { rx_channel: self.rx_send.clone(), tx_channel: tx_recv, address, stop }
    }

    pub fn new_channel(&mut self, tx_cap: usize) -> InterfaceChannel {
        self.new_channel_with_runtime(tx_cap, InterfaceDescriptor::default(), None)
    }

    pub fn new_context<T: Interface>(&mut self, inner: T) -> InterfaceContext<T> {
        self.new_context_with_parent(inner, None)
    }

    fn new_context_with_parent<T: Interface>(
        &mut self,
        inner: T,
        parent: Option<AddressHash>,
    ) -> InterfaceContext<T> {
        let descriptor = inner.descriptor();
        let channel =
            self.new_channel_with_runtime(DEFAULT_IFACE_TX_QUEUE_CAPACITY, descriptor, parent);
        let runtime = self.ifaces.last().expect("newly registered interface").runtime.clone();

        let inner = Arc::new(Mutex::new(inner));

        InterfaceContext::<T> {
            inner: inner.clone(),
            channel,
            cancel: self.cancel.clone(),
            ifac: None,
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
                    interface.stats.tx_bytes.load(Ordering::Relaxed),
                    interface.stats.rx_bytes.load(Ordering::Relaxed),
                )
            })
            .collect();
        let mut snapshots: Vec<_> = metadata
            .iter()
            .map(|(hash, descriptor, state, parent, generation, tx_bytes, rx_bytes)| {
                InterfaceSnapshot {
                    hash: *hash,
                    kind: descriptor.kind,
                    mode: descriptor.mode,
                    state: *state,
                    local_endpoint: descriptor.local_endpoint.clone(),
                    remote_endpoint: descriptor.remote_endpoint.clone(),
                    parent: *parent,
                    tx_bytes: *tx_bytes,
                    rx_bytes: *rx_bytes,
                    connected_peers: metadata
                        .iter()
                        .filter(|(_, _, child_state, child_parent, _, _, _)| {
                            *child_parent == Some(*hash)
                                && *child_state == InterfaceState::Connected
                        })
                        .count() as u32,
                    generation: *generation,
                }
            })
            .collect();
        snapshots.sort_by_key(|snapshot| snapshot.hash.as_slice().to_vec());
        snapshots
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
        for iface in &self.ifaces {
            let should_send = match message.tx_type {
                TxMessageType::Broadcast(address) => {
                    let mut should_send = true;
                    if let Some(address) = address {
                        should_send = address != iface.address;
                    }

                    should_send
                }
                TxMessageType::Direct(address) => address == iface.address,
            };

            if should_send && !iface.stop.is_cancelled() {
                trace.matched_ifaces += 1;
                match iface.tx_send.try_send(message) {
                    Ok(()) => {
                        trace.sent_ifaces += 1;
                        iface.stats.tx_bytes.fetch_add(pkt_bytes, Ordering::Relaxed);
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

    /// Return a snapshot of per-interface byte counters.
    pub fn interface_stats(&self) -> HashMap<AddressHash, InterfaceStatsSnapshot> {
        self.stats_map
            .lock()
            .expect("interface stats lock")
            .iter()
            .map(|(addr, stats)| {
                (
                    *addr,
                    InterfaceStatsSnapshot {
                        tx_bytes: stats.tx_bytes.load(Ordering::Relaxed),
                        rx_bytes: stats.rx_bytes.load(Ordering::Relaxed),
                    },
                )
            })
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

    #[test]
    fn shutdown_cancels_manager_and_local_interfaces() {
        let mut manager = InterfaceManager::new(1);
        let channel = manager.new_channel(1);
        let local_stop = channel.stop.clone();

        manager.shutdown();

        assert!(manager.cancel.is_cancelled());
        assert!(local_stop.is_cancelled());
    }
}
