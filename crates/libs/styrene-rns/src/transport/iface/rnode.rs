//! Bearer-neutral RNode protocol and bounded ordered-byte attempt contract.

use std::collections::VecDeque;
use std::fmt;

#[cfg(feature = "serial")]
use std::sync::{Arc, Mutex};
#[cfg(feature = "serial")]
use std::time::Duration;

#[cfg(feature = "serial")]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(feature = "serial")]
use tokio_serial::SerialStream;

use super::kiss::{CMD_DATA, KissDecoder, KissFrame, kiss_encode_command};
#[cfg(feature = "serial")]
use super::{
    Interface, InterfaceContext, InterfaceDescriptor, InterfaceEndpoint, InterfaceKind,
    InterfaceMode, InterfaceState, RxMessage,
};
#[cfg(feature = "serial")]
use crate::buffer::{InputBuffer, OutputBuffer};
#[cfg(feature = "serial")]
use crate::packet::Packet;
#[cfg(feature = "serial")]
use crate::serde::Serialize;
#[cfg(feature = "serial")]
use crate::transport::iface::ifac::{ifac_unwrap, ifac_wrap};

pub const RNODE_MTU: usize = 508;
const DEFAULT_WRITE_CAP: usize = 20;
const MAX_INPUT_CHUNK: usize = 4_096;
const MAX_PENDING_WRITES: usize = 32;
#[cfg(feature = "serial")]
const DEFAULT_BAUD_RATE: u32 = 115_200;
#[cfg(feature = "serial")]
const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(3);
#[cfg(feature = "serial")]
const DEFAULT_RECONNECT_DELAY: Duration = Duration::from_secs(3);
#[cfg(feature = "serial")]
const MIN_RECONNECT_DELAY: Duration = Duration::from_millis(100);
#[cfg(feature = "serial")]
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(60);
#[cfg(feature = "serial")]
const OPEN_SETTLE_DELAY: Duration = Duration::from_secs(2);
#[cfg(feature = "serial")]
const CONFIG_COMMAND_DELAY: Duration = Duration::from_millis(100);
#[cfg(feature = "serial")]
const RADIO_OFF_TIMEOUT: Duration = Duration::from_millis(250);
#[cfg(feature = "serial")]
const PACKET_BUFFER_SIZE: usize = 2_048;

pub const CMD_FREQUENCY: u8 = 0x01;
pub const CMD_BANDWIDTH: u8 = 0x02;
pub const CMD_TX_POWER: u8 = 0x03;
pub const CMD_SPREADING_FACTOR: u8 = 0x04;
pub const CMD_CODING_RATE: u8 = 0x05;
pub const CMD_RADIO_STATE: u8 = 0x06;
pub const CMD_DETECT: u8 = 0x08;
pub const CMD_BOARD: u8 = 0x47;
pub const CMD_PLATFORM: u8 = 0x48;
pub const CMD_MCU: u8 = 0x49;
pub const CMD_FIRMWARE_VERSION: u8 = 0x50;
pub const CMD_ROM_READ: u8 = 0x51;
pub const CMD_HASHES: u8 = 0x60;

const DETECT_REQUEST: u8 = 0x73;
const DETECT_RESPONSE: u8 = 0x46;
const RADIO_OFF: u8 = 0x00;
const RADIO_ON: u8 = 0x01;
const TARGET_FIRMWARE_HASH: u8 = 0x01;
const RUNNING_FIRMWARE_HASH: u8 = 0x02;
const FIRMWARE_HASH_LEN: usize = 32;
const RNODE_ROM_SIZE: usize = 200;
const ROM_PRODUCT: usize = 0;
const ROM_MODEL: usize = 1;
const ROM_HARDWARE_REVISION: usize = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RNodeBearerKind {
    Ble,
    BluetoothClassic,
    AndroidUsb,
    Serial,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RNodeBearerInfo {
    pub kind: RNodeBearerKind,
    pub negotiated_mtu: Option<usize>,
    pub max_write_size: Option<usize>,
}

/// Split one encoded RNode frame into ordered writes accepted by the bearer.
pub fn rnode_write_chunks(
    info: Option<RNodeBearerInfo>,
    frame: &[u8],
) -> impl Iterator<Item = &[u8]> {
    let cap = info
        .and_then(|info| info.max_write_size.or(info.negotiated_mtu))
        .unwrap_or(DEFAULT_WRITE_CAP)
        .max(1);
    frame.chunks(cap)
}

/// Cancellation-safe, one-attempt ordered-byte bearer.
///
/// Dropping an in-flight operation must not prevent an idempotent `close`.
/// Reconnect, permission, discovery, and application lifecycle belong to the caller.
#[allow(async_fn_in_trait)]
pub trait RNodeByteAttempt {
    async fn open(&mut self) -> Result<RNodeBearerInfo, String>;
    async fn read(&mut self) -> Result<Option<Vec<u8>>, String>;
    async fn write(&mut self, payload: Vec<u8>) -> Result<(), String>;
    async fn close(&mut self) -> Result<(), String>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RNodeProtocolPhase {
    Idle,
    Detecting,
    Configuring,
    Ready,
    Closed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RNodeFirmwareVersion {
    pub major: u8,
    pub minor: u8,
}

impl fmt::Display for RNodeFirmwareVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.major, self.minor)
    }
}

/// Raw, read-only facts reported by one RNode protocol attempt.
///
/// Numeric platform, MCU, board, product, and model codes remain unclassified
/// here so firmware policy cannot infer an exact target from partial evidence.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RNodeMetadata {
    pub firmware_version: Option<RNodeFirmwareVersion>,
    pub platform: Option<u8>,
    pub mcu: Option<u8>,
    pub board: Option<u8>,
    pub product: Option<u8>,
    pub model: Option<u8>,
    pub hardware_revision: Option<u8>,
    pub target_firmware_hash: Option<[u8; FIRMWARE_HASH_LEN]>,
    pub running_firmware_hash: Option<[u8; FIRMWARE_HASH_LEN]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RNodeRadioProfile {
    pub frequency_hz: u32,
    pub bandwidth_hz: u32,
    pub tx_power_dbm: u8,
    pub spreading_factor: u8,
    pub coding_rate: u8,
}

impl RNodeRadioProfile {
    pub const US_915_DEVELOPMENT: Self = Self {
        frequency_hz: 915_000_000,
        bandwidth_hz: 125_000,
        tx_power_dbm: 17,
        spreading_factor: 7,
        coding_rate: 5,
    };

    pub fn new(
        frequency_hz: u32,
        bandwidth_hz: u32,
        tx_power_dbm: u8,
        spreading_factor: u8,
        coding_rate: u8,
    ) -> Result<Self, RNodeProfileError> {
        let profile =
            Self { frequency_hz, bandwidth_hz, tx_power_dbm, spreading_factor, coding_rate };
        profile.validate()?;
        Ok(profile)
    }

    pub fn validate(&self) -> Result<(), RNodeProfileError> {
        if !(137_000_000..=3_000_000_000).contains(&self.frequency_hz) {
            return Err(RNodeProfileError::Frequency(self.frequency_hz));
        }
        if !(7_800..=500_000).contains(&self.bandwidth_hz) {
            return Err(RNodeProfileError::Bandwidth(self.bandwidth_hz));
        }
        // Effective output power includes supported RNodes with an external PA.
        if self.tx_power_dbm > 37 {
            return Err(RNodeProfileError::TxPower(self.tx_power_dbm));
        }
        if !(5..=12).contains(&self.spreading_factor) {
            return Err(RNodeProfileError::SpreadingFactor(self.spreading_factor));
        }
        if !(5..=8).contains(&self.coding_rate) {
            return Err(RNodeProfileError::CodingRate(self.coding_rate));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RNodeProfileError {
    Frequency(u32),
    Bandwidth(u32),
    TxPower(u8),
    SpreadingFactor(u8),
    CodingRate(u8),
}

impl fmt::Display for RNodeProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Frequency(value) => write!(formatter, "invalid RNode frequency: {value}"),
            Self::Bandwidth(value) => write!(formatter, "invalid RNode bandwidth: {value}"),
            Self::TxPower(value) => write!(formatter, "invalid RNode tx power: {value}"),
            Self::SpreadingFactor(value) => {
                write!(formatter, "invalid RNode spreading factor: {value}")
            }
            Self::CodingRate(value) => write!(formatter, "invalid RNode coding rate: {value}"),
        }
    }
}

impl std::error::Error for RNodeProfileError {}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RNodeProtocolOutput {
    pub packets: Vec<Vec<u8>>,
    pub writes: Vec<Vec<u8>>,
    pub became_ready: bool,
}

#[derive(Debug, Eq, PartialEq)]
pub enum RNodeProtocolError {
    InputChunkTooLarge,
    PacketTooLarge,
    NotReady,
    Closed,
    WriteQueueFull,
    ConfigurationMismatch,
}

impl std::fmt::Display for RNodeProtocolError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InputChunkTooLarge => {
                write!(formatter, "RNode input chunk exceeds {MAX_INPUT_CHUNK} bytes")
            }
            Self::PacketTooLarge => write!(formatter, "RNode packet exceeds {RNODE_MTU} bytes"),
            Self::NotReady => formatter.write_str(
                "RNode payload transmission is gated until radio configuration is verified",
            ),
            Self::Closed => formatter.write_str("RNode protocol is closed"),
            Self::WriteQueueFull => formatter.write_str("RNode write queue is at capacity"),
            Self::ConfigurationMismatch => {
                formatter.write_str("RNode configuration readback does not match requested profile")
            }
        }
    }
}

impl std::error::Error for RNodeProtocolError {}

#[derive(Default)]
struct ObservedConfig {
    detected: bool,
    frequency_hz: Option<u32>,
    bandwidth_hz: Option<u32>,
    tx_power_dbm: Option<u8>,
    spreading_factor: Option<u8>,
    coding_rate: Option<u8>,
    radio_state: Option<u8>,
}

pub struct RNodeProtocol {
    profile: RNodeRadioProfile,
    phase: RNodeProtocolPhase,
    decoder: KissDecoder,
    observed: ObservedConfig,
    metadata: RNodeMetadata,
}

impl RNodeProtocol {
    #[must_use]
    pub fn new(profile: RNodeRadioProfile) -> Self {
        Self {
            profile,
            phase: RNodeProtocolPhase::Idle,
            decoder: KissDecoder::with_limits(RNODE_MTU + 1, MAX_PENDING_WRITES),
            observed: ObservedConfig::default(),
            metadata: RNodeMetadata::default(),
        }
    }

    #[must_use]
    pub const fn phase(&self) -> RNodeProtocolPhase {
        self.phase
    }

    #[must_use]
    pub const fn metadata(&self) -> &RNodeMetadata {
        &self.metadata
    }

    pub fn start(&mut self) -> Result<Vec<Vec<u8>>, RNodeProtocolError> {
        if self.phase == RNodeProtocolPhase::Closed {
            return Err(RNodeProtocolError::Closed);
        }
        self.phase = RNodeProtocolPhase::Detecting;
        self.observed = ObservedConfig::default();
        self.metadata = RNodeMetadata::default();
        Ok(vec![
            kiss_encode_command(CMD_DETECT, &[DETECT_REQUEST]),
            kiss_encode_command(CMD_FIRMWARE_VERSION, &[0]),
            kiss_encode_command(CMD_PLATFORM, &[0]),
            kiss_encode_command(CMD_MCU, &[0]),
            kiss_encode_command(CMD_BOARD, &[0]),
            kiss_encode_command(CMD_ROM_READ, &[0]),
            kiss_encode_command(CMD_HASHES, &[TARGET_FIRMWARE_HASH]),
            kiss_encode_command(CMD_HASHES, &[RUNNING_FIRMWARE_HASH]),
        ])
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Result<RNodeProtocolOutput, RNodeProtocolError> {
        if self.phase == RNodeProtocolPhase::Closed {
            return Err(RNodeProtocolError::Closed);
        }
        if bytes.len() > MAX_INPUT_CHUNK {
            return Err(RNodeProtocolError::InputChunkTooLarge);
        }
        self.decoder.feed(bytes);
        let mut output = RNodeProtocolOutput::default();
        while let Some(frame) = self.decoder.take_kiss_frame() {
            self.accept_frame(frame, &mut output)?;
        }
        Ok(output)
    }

    pub fn encode_packet(&self, packet: &[u8]) -> Result<Vec<u8>, RNodeProtocolError> {
        if self.phase == RNodeProtocolPhase::Closed {
            return Err(RNodeProtocolError::Closed);
        }
        if self.phase != RNodeProtocolPhase::Ready {
            return Err(RNodeProtocolError::NotReady);
        }
        if packet.len() > RNODE_MTU {
            return Err(RNodeProtocolError::PacketTooLarge);
        }
        Ok(kiss_encode_command(CMD_DATA, packet))
    }

    pub fn close(&mut self) -> Vec<u8> {
        self.phase = RNodeProtocolPhase::Closed;
        kiss_encode_command(CMD_RADIO_STATE, &[RADIO_OFF])
    }

    fn accept_frame(
        &mut self,
        frame: KissFrame,
        output: &mut RNodeProtocolOutput,
    ) -> Result<(), RNodeProtocolError> {
        let was_ready = self.phase == RNodeProtocolPhase::Ready;
        let is_configuration_readback = matches!(
            frame.command,
            CMD_FREQUENCY
                | CMD_BANDWIDTH
                | CMD_TX_POWER
                | CMD_SPREADING_FACTOR
                | CMD_CODING_RATE
                | CMD_RADIO_STATE
        );
        match frame.command {
            CMD_DATA if self.phase == RNodeProtocolPhase::Ready && !frame.payload.is_empty() => {
                if frame.payload.len() <= RNODE_MTU {
                    output.packets.push(frame.payload);
                }
            }
            CMD_DETECT => {
                self.observed.detected = frame.payload.first() == Some(&DETECT_RESPONSE);
                if self.observed.detected && self.phase == RNodeProtocolPhase::Detecting {
                    self.phase = RNodeProtocolPhase::Configuring;
                    output.writes = self.configuration_frames();
                }
            }
            CMD_FREQUENCY => self.observed.frequency_hz = read_u32(&frame.payload),
            CMD_BANDWIDTH => self.observed.bandwidth_hz = read_u32(&frame.payload),
            CMD_TX_POWER => self.observed.tx_power_dbm = frame.payload.first().copied(),
            CMD_SPREADING_FACTOR => {
                self.observed.spreading_factor = frame.payload.first().copied();
            }
            CMD_CODING_RATE => self.observed.coding_rate = frame.payload.first().copied(),
            CMD_RADIO_STATE => self.observed.radio_state = frame.payload.first().copied(),
            CMD_FIRMWARE_VERSION if frame.payload.len() == 2 => {
                self.metadata.firmware_version =
                    Some(RNodeFirmwareVersion { major: frame.payload[0], minor: frame.payload[1] });
            }
            CMD_PLATFORM if frame.payload.len() == 1 => {
                self.metadata.platform = frame.payload.first().copied();
            }
            CMD_MCU if frame.payload.len() == 1 => {
                self.metadata.mcu = frame.payload.first().copied();
            }
            CMD_BOARD if frame.payload.len() == 1 => {
                self.metadata.board = frame.payload.first().copied();
            }
            CMD_ROM_READ if frame.payload.len() == RNODE_ROM_SIZE => {
                self.metadata.product = frame.payload.get(ROM_PRODUCT).copied();
                self.metadata.model = frame.payload.get(ROM_MODEL).copied();
                self.metadata.hardware_revision = frame.payload.get(ROM_HARDWARE_REVISION).copied();
            }
            CMD_HASHES if frame.payload.len() == FIRMWARE_HASH_LEN + 1 => {
                let mut hash = [0; FIRMWARE_HASH_LEN];
                hash.copy_from_slice(&frame.payload[1..]);
                match frame.payload[0] {
                    TARGET_FIRMWARE_HASH => self.metadata.target_firmware_hash = Some(hash),
                    RUNNING_FIRMWARE_HASH => self.metadata.running_firmware_hash = Some(hash),
                    _ => {}
                }
            }
            _ => {}
        }
        if was_ready && is_configuration_readback && !self.configuration_matches() {
            self.phase = RNodeProtocolPhase::Configuring;
            return Err(RNodeProtocolError::ConfigurationMismatch);
        }
        if self.phase == RNodeProtocolPhase::Configuring && self.configuration_matches() {
            self.phase = RNodeProtocolPhase::Ready;
            output.became_ready = true;
        }
        if output.writes.len() > MAX_PENDING_WRITES {
            return Err(RNodeProtocolError::WriteQueueFull);
        }
        Ok(())
    }

    fn configuration_frames(&self) -> Vec<Vec<u8>> {
        vec![
            kiss_encode_command(CMD_FREQUENCY, &self.profile.frequency_hz.to_be_bytes()),
            kiss_encode_command(CMD_BANDWIDTH, &self.profile.bandwidth_hz.to_be_bytes()),
            kiss_encode_command(CMD_TX_POWER, &[self.profile.tx_power_dbm]),
            kiss_encode_command(CMD_SPREADING_FACTOR, &[self.profile.spreading_factor]),
            kiss_encode_command(CMD_CODING_RATE, &[self.profile.coding_rate]),
            kiss_encode_command(CMD_RADIO_STATE, &[RADIO_ON]),
        ]
    }

    fn configuration_matches(&self) -> bool {
        self.observed.detected
            && self.observed.frequency_hz == Some(self.profile.frequency_hz)
            && self.observed.bandwidth_hz == Some(self.profile.bandwidth_hz)
            && self.observed.tx_power_dbm == Some(self.profile.tx_power_dbm)
            && self.observed.spreading_factor == Some(self.profile.spreading_factor)
            && self.observed.coding_rate == Some(self.profile.coding_rate)
            && self.observed.radio_state == Some(RADIO_ON)
    }
}

/// Shared protocol runtime over one platform-owned ordered-byte attempt.
pub struct RNodeEngine<B> {
    backend: B,
    protocol: RNodeProtocol,
    info: Option<RNodeBearerInfo>,
    pending_writes: VecDeque<Vec<u8>>,
}

impl<B: RNodeByteAttempt> RNodeEngine<B> {
    #[must_use]
    pub fn new(backend: B, profile: RNodeRadioProfile) -> Self {
        Self {
            backend,
            protocol: RNodeProtocol::new(profile),
            info: None,
            pending_writes: VecDeque::new(),
        }
    }

    pub async fn open(&mut self) -> Result<RNodeBearerInfo, String> {
        let info = self.backend.open().await?;
        self.info = Some(info);
        let writes = self.protocol.start().map_err(|error| error.to_string())?;
        self.write_all(writes).await?;
        Ok(info)
    }

    pub async fn poll(&mut self) -> Result<RNodeProtocolOutput, String> {
        let Some(bytes) = self.backend.read().await? else {
            return Ok(RNodeProtocolOutput::default());
        };
        let mut output = self.protocol.feed(&bytes).map_err(|error| error.to_string())?;
        self.write_all(std::mem::take(&mut output.writes)).await?;
        Ok(output)
    }

    #[must_use]
    pub const fn metadata(&self) -> &RNodeMetadata {
        self.protocol.metadata()
    }

    pub async fn send_packet(&mut self, packet: &[u8]) -> Result<(), String> {
        let frame = self.protocol.encode_packet(packet).map_err(|error| error.to_string())?;
        self.write_frame(frame).await
    }

    pub async fn close(&mut self) -> Result<(), String> {
        let shutdown = self.protocol.close();
        let write_result =
            if self.info.is_some() { self.write_frame(shutdown).await } else { Ok(()) };
        let close_result = self.backend.close().await;
        self.info = None;
        write_result.and(close_result)
    }

    #[must_use]
    pub fn into_backend(self) -> B {
        self.backend
    }

    async fn write_all(&mut self, writes: Vec<Vec<u8>>) -> Result<(), String> {
        if self.pending_writes.len().saturating_add(writes.len()) > MAX_PENDING_WRITES {
            return Err(RNodeProtocolError::WriteQueueFull.to_string());
        }
        self.pending_writes.extend(writes);
        while let Some(frame) = self.pending_writes.pop_front() {
            self.write_frame(frame).await?;
        }
        Ok(())
    }

    async fn write_frame(&mut self, frame: Vec<u8>) -> Result<(), String> {
        for chunk in rnode_write_chunks(self.info, &frame) {
            self.backend.write(chunk.to_vec()).await?;
        }
        Ok(())
    }
}

#[cfg(feature = "serial")]
struct SerialAttempt {
    path: String,
    baud_rate: u32,
    stream: Option<SerialStream>,
}

#[cfg(feature = "serial")]
impl RNodeByteAttempt for SerialAttempt {
    async fn open(&mut self) -> Result<RNodeBearerInfo, String> {
        use tokio_serial::SerialPortBuilderExt;

        let stream = tokio_serial::new(&self.path, self.baud_rate)
            .open_native_async()
            .map_err(|_| "serial open failed".to_string())?;
        self.stream = Some(stream);
        tokio::time::sleep(OPEN_SETTLE_DELAY).await;
        Ok(RNodeBearerInfo {
            kind: RNodeBearerKind::Serial,
            negotiated_mtu: None,
            max_write_size: Some(RNODE_MTU + 4),
        })
    }

    async fn read(&mut self) -> Result<Option<Vec<u8>>, String> {
        let stream = self.stream.as_mut().ok_or_else(|| "serial stream is closed".to_string())?;
        let mut bytes = vec![0; MAX_INPUT_CHUNK];
        let count = stream.read(&mut bytes).await.map_err(|_| "serial read failed".to_string())?;
        if count == 0 {
            return Err("serial stream closed".to_string());
        }
        bytes.truncate(count);
        Ok(Some(bytes))
    }

    async fn write(&mut self, payload: Vec<u8>) -> Result<(), String> {
        let stream = self.stream.as_mut().ok_or_else(|| "serial stream is closed".to_string())?;
        stream.write_all(&payload).await.map_err(|_| "serial write failed".to_string())?;
        stream.flush().await.map_err(|_| "serial flush failed".to_string())?;
        if payload.get(1).is_some_and(|command| {
            matches!(
                *command,
                CMD_FREQUENCY
                    | CMD_BANDWIDTH
                    | CMD_TX_POWER
                    | CMD_SPREADING_FACTOR
                    | CMD_CODING_RATE
                    | CMD_RADIO_STATE
            )
        }) {
            tokio::time::sleep(CONFIG_COMMAND_DELAY).await;
        }
        Ok(())
    }

    async fn close(&mut self) -> Result<(), String> {
        let Some(mut stream) = self.stream.take() else {
            return Ok(());
        };
        stream.shutdown().await.map_err(|_| "serial close failed".to_string())
    }
}

#[cfg(feature = "serial")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RNodeState {
    Disconnected,
    Detecting,
    Configuring,
    Online,
}

#[cfg(feature = "serial")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RNodeStatusSnapshot {
    pub state: RNodeState,
    pub generation: u64,
    pub metadata: RNodeMetadata,
}

#[cfg(feature = "serial")]
#[derive(Clone)]
pub struct RNodeStatus {
    snapshot: Arc<Mutex<RNodeStatusSnapshot>>,
}

#[cfg(feature = "serial")]
impl RNodeStatus {
    pub fn state(&self) -> RNodeState {
        self.snapshot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).state
    }

    pub fn snapshot(&self) -> RNodeStatusSnapshot {
        self.snapshot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }

    fn set(&self, state: RNodeState) {
        self.snapshot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).state = state;
    }

    fn begin_attempt(&self) -> Result<(), &'static str> {
        let mut snapshot = self.snapshot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        snapshot.generation =
            snapshot.generation.checked_add(1).ok_or("RNode status generation exhausted")?;
        snapshot.metadata = RNodeMetadata::default();
        Ok(())
    }

    fn set_metadata(&self, metadata: RNodeMetadata) {
        self.snapshot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).metadata = metadata;
    }
}

/// Native serial interface for stock RNode firmware.
#[cfg(feature = "serial")]
pub struct RNodeInterface {
    path: String,
    baud_rate: u32,
    profile: RNodeRadioProfile,
    reconnect_delay: Duration,
    command_timeout: Duration,
    status: RNodeStatus,
}

#[cfg(feature = "serial")]
impl RNodeInterface {
    pub fn new(
        path: impl Into<String>,
        profile: RNodeRadioProfile,
    ) -> Result<Self, RNodeProfileError> {
        profile.validate()?;
        Ok(Self {
            path: path.into(),
            baud_rate: DEFAULT_BAUD_RATE,
            profile,
            reconnect_delay: DEFAULT_RECONNECT_DELAY,
            command_timeout: DEFAULT_COMMAND_TIMEOUT,
            status: RNodeStatus {
                snapshot: Arc::new(Mutex::new(RNodeStatusSnapshot {
                    state: RNodeState::Disconnected,
                    generation: 0,
                    metadata: RNodeMetadata::default(),
                })),
            },
        })
    }

    #[must_use]
    pub fn with_baud_rate(mut self, baud_rate: u32) -> Self {
        self.baud_rate = baud_rate;
        self
    }

    #[must_use]
    pub fn with_reconnect_delay(mut self, reconnect_delay: Duration) -> Self {
        self.reconnect_delay = reconnect_delay.clamp(MIN_RECONNECT_DELAY, MAX_RECONNECT_DELAY);
        self
    }

    #[must_use]
    pub fn status(&self) -> RNodeStatus {
        self.status.clone()
    }

    pub async fn spawn(context: InterfaceContext<Self>) {
        let iface_stop = context.channel.stop.clone();
        let (path, baud_rate, profile, reconnect_delay, command_timeout, status) = {
            let inner = context.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            (
                inner.path.clone(),
                inner.baud_rate,
                inner.profile,
                inner.reconnect_delay,
                inner.command_timeout,
                inner.status.clone(),
            )
        };
        let address = context.channel.address;
        let runtime = context.runtime.clone();
        let stats = context.stats.clone();
        let ifac = context.ifac.clone();
        let (rx_channel, mut tx_channel) = context.channel.split();

        while !context.cancel.is_cancelled() && !iface_stop.is_cancelled() {
            if status.begin_attempt().is_err() {
                crate::transport_diagnostic!("[rnode] status generation exhausted");
                break;
            }
            status.set(RNodeState::Disconnected);
            runtime.set_state(InterfaceState::Connecting);
            let attempt = SerialAttempt { path: path.clone(), baud_rate, stream: None };
            let mut engine = RNodeEngine::new(attempt, profile);
            let result = run_serial_session(
                &mut engine,
                address,
                &rx_channel,
                &mut tx_channel,
                &context.cancel,
                &iface_stop,
                ifac.as_deref(),
                command_timeout,
                &status,
                &runtime,
                &stats,
            )
            .await;
            let _ = tokio::time::timeout(RADIO_OFF_TIMEOUT, engine.close()).await;
            if let Err(error) = result {
                crate::transport_diagnostic!("[rnode] disconnected error={error}");
            }
            status.set(RNodeState::Disconnected);
            runtime.set_state(InterfaceState::Retrying);
            tokio::select! {
                _ = context.cancel.cancelled() => break,
                _ = iface_stop.cancelled() => break,
                _ = tokio::time::sleep(reconnect_delay) => {}
            }
        }

        status.set(RNodeState::Disconnected);
        runtime.set_state(InterfaceState::Closed);
        iface_stop.cancel();
    }
}

#[cfg(feature = "serial")]
impl Interface for RNodeInterface {
    fn mtu() -> usize {
        RNODE_MTU
    }

    fn hardware_mtu(&self) -> Option<usize> {
        Some(RNODE_MTU)
    }

    fn bitrate(&self) -> Option<u64> {
        // LoRa nominal bitrate: SF * (4 / CR) * BW / 2^SF.
        let numerator = u64::from(self.profile.spreading_factor)
            .saturating_mul(4)
            .saturating_mul(u64::from(self.profile.bandwidth_hz));
        let denominator = (1_u64 << self.profile.spreading_factor)
            .saturating_mul(u64::from(self.profile.coding_rate));
        Some(numerator / denominator)
    }

    fn supports_link_mtu_discovery(&self) -> bool {
        true
    }

    fn descriptor(&self) -> InterfaceDescriptor {
        InterfaceDescriptor {
            kind: InterfaceKind::Kiss,
            mode: InterfaceMode::Full,
            local_endpoint: Some(InterfaceEndpoint::Device {
                path: self.path.clone(),
                baud_rate: self.baud_rate,
            }),
            ..Default::default()
        }
    }
}

#[cfg(feature = "serial")]
#[allow(clippy::too_many_arguments)]
async fn run_serial_session(
    engine: &mut RNodeEngine<SerialAttempt>,
    address: crate::hash::AddressHash,
    rx_channel: &super::InterfaceRxSender,
    tx_channel: &mut super::InterfaceTxReceiver,
    cancel: &tokio_util::sync::CancellationToken,
    iface_stop: &tokio_util::sync::CancellationToken,
    ifac: Option<&super::ifac::IfacConfig>,
    command_timeout: Duration,
    status: &RNodeStatus,
    runtime: &super::InterfaceRuntime,
    stats: &super::InterfaceStats,
) -> Result<(), String> {
    status.set(RNodeState::Detecting);
    tokio::select! {
        _ = cancel.cancelled() => return Ok(()),
        _ = iface_stop.cancelled() => return Ok(()),
        result = engine.open() => result?,
    };

    status.set(RNodeState::Configuring);
    let configure = async {
        loop {
            let output = engine.poll().await?;
            status.set_metadata(engine.metadata().clone());
            if output.became_ready {
                return Ok::<(), String>(());
            }
        }
    };
    tokio::select! {
        _ = cancel.cancelled() => return Ok(()),
        _ = iface_stop.cancelled() => return Ok(()),
        result = tokio::time::timeout(command_timeout, configure) => {
            result.map_err(|_| "radio configuration readback timed out".to_string())??;
        }
    }

    status.set(RNodeState::Online);
    runtime.set_state(InterfaceState::Active);
    crate::transport_diagnostic!(
        "[rnode] online frequency={} bandwidth={} tx_power={} sf={} cr={}",
        engine.protocol.profile.frequency_hz,
        engine.protocol.profile.bandwidth_hz,
        engine.protocol.profile.tx_power_dbm,
        engine.protocol.profile.spreading_factor,
        engine.protocol.profile.coding_rate
    );

    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            _ = iface_stop.cancelled() => return Ok(()),
            result = engine.poll() => {
                for bytes in result?.packets {
                    let packet_bytes = if let Some(config) = ifac {
                        let packet = ifac_unwrap(&bytes, config);
                        if packet.is_none() {
                            stats.record_drop(super::InterfaceDropReason::IfacFailure);
                        }
                        packet
                    } else if bytes.first().is_some_and(|byte| byte & 0x80 != 0) {
                        stats.record_drop(super::InterfaceDropReason::IfacFailure);
                        None
                    } else {
                        Some(bytes)
                    };
                    if let Some(bytes) = packet_bytes {
                        match Packet::deserialize(&mut InputBuffer::new(&bytes)) {
                            Ok(packet) => {
                                let _ = rx_channel.send(RxMessage::physical(address, packet, RNODE_MTU)).await;
                            }
                            Err(_) => stats.record_drop(super::InterfaceDropReason::MalformedFrame),
                        }
                    }
                }
            }
            message = tx_channel.recv() => {
                let Some(message) = message else {
                    return Ok(());
                };
                let mut packet_buffer = [0; PACKET_BUFFER_SIZE];
                let mut output = OutputBuffer::new(&mut packet_buffer);
                if message.packet.serialize(&mut output).is_err() {
                    crate::transport_diagnostic!("[rnode] packet serialization failed");
                    continue;
                }
                let bytes = if let Some(config) = ifac {
                    ifac_wrap(output.as_slice(), config)
                } else {
                    output.as_slice().to_vec()
                };
                engine.send_packet(&bytes).await?;
            }
        }
    }
}

fn read_u32(payload: &[u8]) -> Option<u32> {
    let bytes: [u8; 4] = payload.try_into().ok()?;
    Some(u32::from_be_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use std::future::pending;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use std::time::Duration;

    use super::*;

    fn configured_responses(profile: RNodeRadioProfile) -> Vec<u8> {
        [
            kiss_encode_command(CMD_DETECT, &[DETECT_RESPONSE]),
            kiss_encode_command(CMD_FREQUENCY, &profile.frequency_hz.to_be_bytes()),
            kiss_encode_command(CMD_BANDWIDTH, &profile.bandwidth_hz.to_be_bytes()),
            kiss_encode_command(CMD_TX_POWER, &[profile.tx_power_dbm]),
            kiss_encode_command(CMD_SPREADING_FACTOR, &[profile.spreading_factor]),
            kiss_encode_command(CMD_CODING_RATE, &[profile.coding_rate]),
            kiss_encode_command(CMD_RADIO_STATE, &[RADIO_ON]),
        ]
        .concat()
    }

    #[test]
    fn payload_is_gated_until_exact_configuration_readback() {
        let profile = RNodeRadioProfile::US_915_DEVELOPMENT;
        let mut protocol = RNodeProtocol::new(profile);
        let startup = protocol.start().expect("startup frames");
        assert_eq!(startup.len(), 8);
        assert_eq!(protocol.encode_packet(b"blocked"), Err(RNodeProtocolError::NotReady));

        let output = protocol.feed(&configured_responses(profile)).expect("responses");
        assert!(output.became_ready);
        assert_eq!(protocol.phase(), RNodeProtocolPhase::Ready);
        assert_eq!(
            protocol.encode_packet(b"packet").expect("packet"),
            kiss_encode_command(0, b"packet")
        );
    }

    #[test]
    fn metadata_responses_are_retained_across_fragmented_feeds() {
        let mut protocol = RNodeProtocol::new(RNodeRadioProfile::US_915_DEVELOPMENT);
        let startup = protocol.start().expect("startup frames");
        let commands = startup
            .iter()
            .map(|write| {
                let mut decoder = KissDecoder::new();
                decoder.feed(write);
                decoder.take_kiss_frame().expect("inspection frame")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            commands,
            [
                KissFrame { command: CMD_DETECT, payload: vec![DETECT_REQUEST] },
                KissFrame { command: CMD_FIRMWARE_VERSION, payload: vec![0] },
                KissFrame { command: CMD_PLATFORM, payload: vec![0] },
                KissFrame { command: CMD_MCU, payload: vec![0] },
                KissFrame { command: CMD_BOARD, payload: vec![0] },
                KissFrame { command: CMD_ROM_READ, payload: vec![0] },
                KissFrame { command: CMD_HASHES, payload: vec![TARGET_FIRMWARE_HASH] },
                KissFrame { command: CMD_HASHES, payload: vec![RUNNING_FIRMWARE_HASH] },
            ]
        );

        let version = kiss_encode_command(CMD_FIRMWARE_VERSION, &[1, 86]);
        protocol.feed(&version[..2]).expect("version fragment");
        protocol.feed(&version[2..]).expect("version completion");
        protocol.feed(&kiss_encode_command(CMD_PLATFORM, &[0x70])).expect("platform");
        protocol.feed(&kiss_encode_command(CMD_MCU, &[0x71])).expect("MCU");
        protocol.feed(&kiss_encode_command(CMD_BOARD, &[0x51])).expect("board");

        let mut rom = vec![0xff; RNODE_ROM_SIZE];
        rom[ROM_PRODUCT] = 0x10;
        rom[ROM_MODEL] = 0x12;
        rom[ROM_HARDWARE_REVISION] = 0x01;
        protocol.feed(&kiss_encode_command(CMD_ROM_READ, &rom)).expect("ROM");

        let target_hash = [0x11; FIRMWARE_HASH_LEN];
        let running_hash = [0x22; FIRMWARE_HASH_LEN];
        protocol
            .feed(&kiss_encode_command(
                CMD_HASHES,
                &[&[TARGET_FIRMWARE_HASH], target_hash.as_slice()].concat(),
            ))
            .expect("target hash");
        protocol
            .feed(&kiss_encode_command(
                CMD_HASHES,
                &[&[RUNNING_FIRMWARE_HASH], running_hash.as_slice()].concat(),
            ))
            .expect("running hash");

        assert_eq!(
            protocol.metadata(),
            &RNodeMetadata {
                firmware_version: Some(RNodeFirmwareVersion { major: 1, minor: 86 }),
                platform: Some(0x70),
                mcu: Some(0x71),
                board: Some(0x51),
                product: Some(0x10),
                model: Some(0x12),
                hardware_revision: Some(0x01),
                target_firmware_hash: Some(target_hash),
                running_firmware_hash: Some(running_hash),
            }
        );
    }

    #[test]
    fn malformed_metadata_does_not_replace_valid_observations() {
        let mut protocol = RNodeProtocol::new(RNodeRadioProfile::US_915_DEVELOPMENT);
        protocol.start().expect("startup");
        protocol.feed(&kiss_encode_command(CMD_FIRMWARE_VERSION, &[1, 86])).expect("version");
        protocol.feed(&kiss_encode_command(CMD_PLATFORM, &[0x70])).expect("platform");
        protocol.feed(&kiss_encode_command(CMD_MCU, &[0x71])).expect("MCU");
        protocol.feed(&kiss_encode_command(CMD_BOARD, &[0x51])).expect("board");
        let retained = protocol.metadata().clone();

        for (command, payload) in [
            (CMD_FIRMWARE_VERSION, vec![2]),
            (CMD_PLATFORM, vec![0x80, 0x81]),
            (CMD_MCU, Vec::new()),
            (CMD_BOARD, vec![0x52, 0x53]),
            (CMD_ROM_READ, vec![0; RNODE_ROM_SIZE - 1]),
            (CMD_HASHES, vec![TARGET_FIRMWARE_HASH; FIRMWARE_HASH_LEN]),
        ] {
            protocol.feed(&kiss_encode_command(command, &payload)).expect("malformed response");
        }

        assert_eq!(protocol.metadata(), &retained);
    }

    #[test]
    fn profile_validates_every_radio_field() {
        assert!(RNodeRadioProfile::new(915_000_000, 125_000, 17, 7, 5).is_ok());
        assert_eq!(
            RNodeRadioProfile::new(1, 125_000, 17, 7, 5),
            Err(RNodeProfileError::Frequency(1))
        );
        assert_eq!(
            RNodeRadioProfile::new(915_000_000, 1, 17, 7, 5),
            Err(RNodeProfileError::Bandwidth(1))
        );
        assert_eq!(
            RNodeRadioProfile::new(915_000_000, 125_000, 38, 7, 5),
            Err(RNodeProfileError::TxPower(38))
        );
        assert_eq!(
            RNodeRadioProfile::new(915_000_000, 125_000, 17, 4, 5),
            Err(RNodeProfileError::SpreadingFactor(4))
        );
        assert_eq!(
            RNodeRadioProfile::new(915_000_000, 125_000, 17, 7, 9),
            Err(RNodeProfileError::CodingRate(9))
        );
    }

    #[cfg(feature = "serial")]
    #[test]
    fn native_interface_reports_the_canonical_rnode_mtu() {
        let interface =
            RNodeInterface::new("/dev/test-rnode", RNodeRadioProfile::US_915_DEVELOPMENT)
                .expect("valid native interface");

        assert_eq!(RNodeInterface::mtu(), RNODE_MTU);
        assert_eq!(interface.hardware_mtu(), Some(RNODE_MTU));
        assert_eq!(interface.bitrate(), Some(5_468));
        assert!(interface.supports_link_mtu_discovery());
    }

    #[cfg(feature = "serial")]
    #[test]
    fn native_status_scopes_metadata_to_an_attempt_generation() {
        let interface =
            RNodeInterface::new("/dev/test-rnode", RNodeRadioProfile::US_915_DEVELOPMENT)
                .expect("valid native interface");
        let status = interface.status();
        let clone = status.clone();
        status.begin_attempt().expect("begin first attempt");
        status.set_metadata(RNodeMetadata {
            platform: Some(0x70),
            mcu: Some(0x71),
            ..RNodeMetadata::default()
        });

        assert_eq!(clone.snapshot().generation, 1);
        assert_eq!(clone.snapshot().metadata.platform, Some(0x70));
        status.begin_attempt().expect("begin second attempt");
        assert_eq!(clone.snapshot().generation, 2);
        assert_eq!(clone.snapshot().metadata, RNodeMetadata::default());
    }

    #[test]
    fn post_ready_mismatch_revokes_payload_readiness() {
        let profile = RNodeRadioProfile::US_915_DEVELOPMENT;
        let mut protocol = RNodeProtocol::new(profile);
        protocol.start().expect("startup");
        protocol.feed(&configured_responses(profile)).expect("configuration");

        let error = protocol
            .feed(&kiss_encode_command(CMD_FREQUENCY, &914_000_000_u32.to_be_bytes()))
            .expect_err("changed readback must fail closed");

        assert_eq!(error, RNodeProtocolError::ConfigurationMismatch);
        assert_eq!(protocol.phase(), RNodeProtocolPhase::Configuring);
        assert_eq!(protocol.encode_packet(b"blocked"), Err(RNodeProtocolError::NotReady));
    }

    #[test]
    fn fragmented_detection_emits_exact_configuration_order() {
        let profile = RNodeRadioProfile::US_915_DEVELOPMENT;
        let mut protocol = RNodeProtocol::new(profile);
        protocol.start().expect("startup");
        let detect = kiss_encode_command(CMD_DETECT, &[DETECT_RESPONSE]);
        assert!(protocol.feed(&detect[..2]).expect("fragment one").writes.is_empty());
        let output = protocol.feed(&detect[2..]).expect("fragment two");

        let commands = output
            .writes
            .iter()
            .map(|write| {
                let mut decoder = KissDecoder::new();
                decoder.feed(write);
                decoder.take_kiss_frame().expect("configuration frame").command
            })
            .collect::<Vec<_>>();
        assert_eq!(
            commands,
            [
                CMD_FREQUENCY,
                CMD_BANDWIDTH,
                CMD_TX_POWER,
                CMD_SPREADING_FACTOR,
                CMD_CODING_RATE,
                CMD_RADIO_STATE
            ]
        );
    }

    #[test]
    fn mismatched_readback_never_enables_payload() {
        let profile = RNodeRadioProfile::US_915_DEVELOPMENT;
        let responses = [
            kiss_encode_command(CMD_DETECT, &[DETECT_RESPONSE]),
            kiss_encode_command(CMD_FREQUENCY, &profile.frequency_hz.to_be_bytes()),
            kiss_encode_command(CMD_BANDWIDTH, &profile.bandwidth_hz.to_be_bytes()),
            kiss_encode_command(CMD_TX_POWER, &[profile.tx_power_dbm - 1]),
            kiss_encode_command(CMD_SPREADING_FACTOR, &[profile.spreading_factor]),
            kiss_encode_command(CMD_CODING_RATE, &[profile.coding_rate]),
            kiss_encode_command(CMD_RADIO_STATE, &[RADIO_ON]),
        ]
        .concat();
        let mut protocol = RNodeProtocol::new(profile);
        protocol.start().expect("startup");
        protocol.feed(&responses).expect("responses");

        assert_eq!(protocol.encode_packet(b"blocked"), Err(RNodeProtocolError::NotReady));
    }

    #[test]
    fn ready_protocol_decodes_fragmented_data_and_bounds_input() {
        let profile = RNodeRadioProfile::US_915_DEVELOPMENT;
        let mut protocol = RNodeProtocol::new(profile);
        protocol.start().expect("startup");
        protocol.feed(&configured_responses(profile)).expect("configuration");
        let frame = kiss_encode_command(CMD_DATA, b"hello");
        assert!(protocol.feed(&frame[..3]).expect("fragment").packets.is_empty());
        assert_eq!(
            protocol.feed(&frame[3..]).expect("completion").packets,
            vec![b"hello".to_vec()]
        );
        assert_eq!(
            protocol.feed(&vec![0; MAX_INPUT_CHUNK + 1]),
            Err(RNodeProtocolError::InputChunkTooLarge)
        );
    }

    #[derive(Default)]
    struct TestAttempt {
        info: Option<RNodeBearerInfo>,
        reads: VecDeque<Vec<u8>>,
        writes: Vec<Vec<u8>>,
        closes: usize,
        fail_writes: bool,
    }

    impl RNodeByteAttempt for TestAttempt {
        async fn open(&mut self) -> Result<RNodeBearerInfo, String> {
            self.info.ok_or_else(|| "missing bearer metadata".to_string())
        }

        async fn read(&mut self) -> Result<Option<Vec<u8>>, String> {
            Ok(self.reads.pop_front())
        }

        async fn write(&mut self, payload: Vec<u8>) -> Result<(), String> {
            if self.fail_writes {
                return Err("write failed".to_string());
            }
            self.writes.push(payload);
            Ok(())
        }

        async fn close(&mut self) -> Result<(), String> {
            self.closes += 1;
            Ok(())
        }
    }

    #[tokio::test]
    async fn attempt_metadata_caps_ordered_writes_and_empty_reads_remain_distinct() {
        let info = RNodeBearerInfo {
            kind: RNodeBearerKind::AndroidUsb,
            negotiated_mtu: Some(64),
            max_write_size: Some(3),
        };
        let mut engine = RNodeEngine::new(
            TestAttempt { info: Some(info), ..Default::default() },
            RNodeRadioProfile::US_915_DEVELOPMENT,
        );

        assert_eq!(engine.open().await.expect("open"), info);
        assert!(engine.into_backend().writes.iter().all(|write| write.len() <= 3));

        let mut engine = RNodeEngine::new(
            TestAttempt { info: Some(info), ..Default::default() },
            RNodeRadioProfile::US_915_DEVELOPMENT,
        );
        engine.open().await.expect("open");
        assert_eq!(engine.poll().await.expect("empty read"), RNodeProtocolOutput::default());
    }

    #[tokio::test]
    async fn engine_retains_metadata_after_poll_output_is_consumed() {
        let info = RNodeBearerInfo {
            kind: RNodeBearerKind::AndroidUsb,
            negotiated_mtu: None,
            max_write_size: Some(512),
        };
        let responses = [
            kiss_encode_command(CMD_FIRMWARE_VERSION, &[1, 86]),
            kiss_encode_command(CMD_PLATFORM, &[0x70]),
            kiss_encode_command(CMD_MCU, &[0x71]),
        ]
        .concat();
        let mut engine = RNodeEngine::new(
            TestAttempt {
                info: Some(info),
                reads: VecDeque::from([responses]),
                ..Default::default()
            },
            RNodeRadioProfile::US_915_DEVELOPMENT,
        );
        engine.open().await.expect("open");
        engine.poll().await.expect("metadata poll");

        assert_eq!(
            engine.metadata().firmware_version,
            Some(RNodeFirmwareVersion { major: 1, minor: 86 })
        );
        assert_eq!(engine.metadata().platform, Some(0x70));
        assert_eq!(engine.metadata().mcu, Some(0x71));
    }

    #[test]
    fn write_chunks_share_the_engine_metadata_precedence_and_preserve_bytes() {
        let frame = (0..45).collect::<Vec<_>>();
        let cases = [
            (
                Some(RNodeBearerInfo {
                    kind: RNodeBearerKind::Ble,
                    negotiated_mtu: Some(64),
                    max_write_size: Some(3),
                }),
                3,
            ),
            (
                Some(RNodeBearerInfo {
                    kind: RNodeBearerKind::Ble,
                    negotiated_mtu: Some(4),
                    max_write_size: None,
                }),
                4,
            ),
            (
                Some(RNodeBearerInfo {
                    kind: RNodeBearerKind::Ble,
                    negotiated_mtu: None,
                    max_write_size: None,
                }),
                20,
            ),
            (
                Some(RNodeBearerInfo {
                    kind: RNodeBearerKind::Ble,
                    negotiated_mtu: None,
                    max_write_size: Some(0),
                }),
                1,
            ),
        ];

        for (info, expected_cap) in cases {
            let chunks = rnode_write_chunks(info, &frame).map(<[u8]>::to_vec).collect::<Vec<_>>();
            assert!(chunks.iter().all(|chunk| chunk.len() <= expected_cap));
            assert_eq!(chunks.concat(), frame);
        }
    }

    #[tokio::test]
    async fn equivalent_bearers_produce_identical_protocol_writes() {
        let mut writes = Vec::new();
        for kind in [RNodeBearerKind::Ble, RNodeBearerKind::AndroidUsb] {
            let info = RNodeBearerInfo { kind, negotiated_mtu: None, max_write_size: Some(512) };
            let mut engine = RNodeEngine::new(
                TestAttempt {
                    info: Some(info),
                    reads: VecDeque::from([configured_responses(
                        RNodeRadioProfile::US_915_DEVELOPMENT,
                    )]),
                    ..Default::default()
                },
                RNodeRadioProfile::US_915_DEVELOPMENT,
            );
            engine.open().await.expect("open");
            assert!(engine.poll().await.expect("startup response").became_ready);
            engine.send_packet(b"same packet").await.expect("packet");
            writes.push(engine.into_backend().writes);
        }

        assert_eq!(writes[0], writes[1]);
    }

    #[derive(Clone, Copy)]
    enum BlockOperation {
        Open,
        Read,
        Write,
    }

    struct BlockingAttempt {
        operation: BlockOperation,
        closed: Arc<AtomicBool>,
        block_write: Arc<AtomicBool>,
        reads: VecDeque<Vec<u8>>,
    }

    impl RNodeByteAttempt for BlockingAttempt {
        async fn open(&mut self) -> Result<RNodeBearerInfo, String> {
            if matches!(self.operation, BlockOperation::Open) {
                pending::<()>().await;
            }
            Ok(RNodeBearerInfo {
                kind: RNodeBearerKind::AndroidUsb,
                negotiated_mtu: None,
                max_write_size: Some(512),
            })
        }

        async fn read(&mut self) -> Result<Option<Vec<u8>>, String> {
            if matches!(self.operation, BlockOperation::Read) {
                pending::<()>().await;
            }
            Ok(self.reads.pop_front())
        }

        async fn write(&mut self, _payload: Vec<u8>) -> Result<(), String> {
            if matches!(self.operation, BlockOperation::Write)
                && self.block_write.load(Ordering::Acquire)
            {
                pending::<()>().await;
            }
            Ok(())
        }

        async fn close(&mut self) -> Result<(), String> {
            self.closed.store(true, Ordering::Release);
            Ok(())
        }
    }

    #[tokio::test]
    async fn cancelled_attempt_operations_still_allow_idempotent_close() {
        for operation in [BlockOperation::Open, BlockOperation::Read, BlockOperation::Write] {
            let closed = Arc::new(AtomicBool::new(false));
            let block_write = Arc::new(AtomicBool::new(false));
            let reads = if matches!(operation, BlockOperation::Write) {
                VecDeque::from([configured_responses(RNodeRadioProfile::US_915_DEVELOPMENT)])
            } else {
                VecDeque::new()
            };
            let mut engine = RNodeEngine::new(
                BlockingAttempt {
                    operation,
                    closed: closed.clone(),
                    block_write: block_write.clone(),
                    reads,
                },
                RNodeRadioProfile::US_915_DEVELOPMENT,
            );

            let completed = match operation {
                BlockOperation::Open => tokio::select! {
                    _ = engine.open() => true,
                    () = tokio::time::sleep(Duration::from_millis(5)) => false,
                },
                BlockOperation::Read => {
                    engine.open().await.expect("open before blocked read");
                    tokio::select! {
                        _ = engine.poll() => true,
                        () = tokio::time::sleep(Duration::from_millis(5)) => false,
                    }
                }
                BlockOperation::Write => {
                    engine.open().await.expect("open before blocked write");
                    engine.poll().await.expect("configuration readback");
                    block_write.store(true, Ordering::Release);
                    let completed = tokio::select! {
                        _ = engine.send_packet(b"blocked") => true,
                        () = tokio::time::sleep(Duration::from_millis(5)) => false,
                    };
                    block_write.store(false, Ordering::Release);
                    completed
                }
            };
            assert!(!completed);
            engine.close().await.expect("close after cancellation");
            engine.close().await.expect("idempotent close");
            assert!(closed.load(Ordering::Acquire));
        }
    }

    #[tokio::test]
    async fn shutdown_write_failure_still_closes_attempt() {
        let info = RNodeBearerInfo {
            kind: RNodeBearerKind::AndroidUsb,
            negotiated_mtu: None,
            max_write_size: None,
        };
        let mut engine = RNodeEngine::new(
            TestAttempt { info: Some(info), fail_writes: true, ..Default::default() },
            RNodeRadioProfile::US_915_DEVELOPMENT,
        );

        assert_eq!(engine.open().await.unwrap_err(), "write failed");
        assert_eq!(engine.close().await.unwrap_err(), "write failed");
        assert_eq!(engine.into_backend().closes, 1);
    }
}
