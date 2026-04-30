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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task;
use tokio_util::sync::CancellationToken;

use crate::hash::AddressHash;
use crate::hash::Hash;
use crate::packet::Packet;

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
}

pub struct InterfaceContext<T: Interface> {
    pub inner: Arc<Mutex<T>>,
    pub channel: InterfaceChannel,
    pub cancel: CancellationToken,
    /// Optional IFAC configuration for this interface. When `Some`, all packets
    /// are wrapped/unwrapped with IFAC authentication at the stream boundary.
    pub ifac: Option<Arc<ifac::IfacConfig>>,
}

pub struct InterfaceManager {
    counter: usize,
    rx_recv: Arc<tokio::sync::Mutex<InterfaceRxReceiver>>,
    rx_send: InterfaceRxSender,
    cancel: CancellationToken,
    ifaces: Vec<LocalInterface>,
    /// Shared stats map so callers can look up per-interface counters without
    /// holding the `InterfaceManager` tokio mutex.
    stats_map: Arc<Mutex<HashMap<AddressHash, Arc<InterfaceStats>>>>,
}

const DEFAULT_IFACE_TX_QUEUE_CAPACITY: usize = 128;
const IFACE_TX_ENQUEUE_TIMEOUT_MS: u64 = 200;

fn tx_diag_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("RETICULUMD_DIAGNOSTICS")
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
            stats_map: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn new_channel(&mut self, tx_cap: usize) -> InterfaceChannel {
        self.counter += 1;

        let counter_bytes = self.counter.to_le_bytes();
        let address = AddressHash::new_from_hash(&Hash::new_from_slice(&counter_bytes[..]));

        let (tx_send, tx_recv) = InterfaceChannel::make_tx_channel(tx_cap);

        log::debug!("iface: create channel {}", address);

        let stop = CancellationToken::new();
        let stats = Arc::new(InterfaceStats::new());

        self.stats_map.lock().unwrap().insert(address, stats.clone());
        self.ifaces.push(LocalInterface { address, tx_send, stop: stop.clone(), stats });

        InterfaceChannel { rx_channel: self.rx_send.clone(), tx_channel: tx_recv, address, stop }
    }

    pub fn new_context<T: Interface>(&mut self, inner: T) -> InterfaceContext<T> {
        let channel = self.new_channel(DEFAULT_IFACE_TX_QUEUE_CAPACITY);

        let inner = Arc::new(Mutex::new(inner));

        InterfaceContext::<T> {
            inner: inner.clone(),
            channel,
            cancel: self.cancel.clone(),
            ifac: None,
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

        task::spawn(worker(context));

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

        task::spawn(worker(context));

        address
    }

    pub fn receiver(&self) -> Arc<tokio::sync::Mutex<InterfaceRxReceiver>> {
        self.rx_recv.clone()
    }

    pub fn cleanup(&mut self) {
        let mut map = self.stats_map.lock().unwrap();
        self.ifaces.retain(|iface| {
            let alive = !iface.stop.is_cancelled();
            if !alive {
                map.remove(&iface.address);
            }
            alive
        });
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
        if let Some(stats) = self.stats_map.lock().unwrap().get(address) {
            stats.rx_bytes.fetch_add(bytes, Ordering::Relaxed);
        }
    }

    /// Return a snapshot of per-interface byte counters.
    pub fn interface_stats(&self) -> HashMap<AddressHash, InterfaceStatsSnapshot> {
        self.stats_map
            .lock()
            .unwrap()
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
