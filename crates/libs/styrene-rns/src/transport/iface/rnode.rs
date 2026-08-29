//! Bearer-neutral RNode protocol and bounded ordered-byte attempt contract.

use std::collections::VecDeque;

use super::kiss::{CMD_DATA, KissDecoder, KissFrame, kiss_encode_command};

pub const RNODE_MTU: usize = 508;
const DEFAULT_WRITE_CAP: usize = 20;
const MAX_INPUT_CHUNK: usize = 4_096;
const MAX_PENDING_WRITES: usize = 32;

pub const CMD_FREQUENCY: u8 = 0x01;
pub const CMD_BANDWIDTH: u8 = 0x02;
pub const CMD_TX_POWER: u8 = 0x03;
pub const CMD_SPREADING_FACTOR: u8 = 0x04;
pub const CMD_CODING_RATE: u8 = 0x05;
pub const CMD_RADIO_STATE: u8 = 0x06;
pub const CMD_DETECT: u8 = 0x08;
pub const CMD_PLATFORM: u8 = 0x48;
pub const CMD_MCU: u8 = 0x49;
pub const CMD_FIRMWARE_VERSION: u8 = 0x50;

const DETECT_REQUEST: u8 = 0x73;
const DETECT_RESPONSE: u8 = 0x46;
const RADIO_OFF: u8 = 0x00;
const RADIO_ON: u8 = 0x01;

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
}

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
}

impl RNodeProtocol {
    #[must_use]
    pub fn new(profile: RNodeRadioProfile) -> Self {
        Self {
            profile,
            phase: RNodeProtocolPhase::Idle,
            decoder: KissDecoder::with_limits(RNODE_MTU + 1, MAX_PENDING_WRITES),
            observed: ObservedConfig::default(),
        }
    }

    #[must_use]
    pub const fn phase(&self) -> RNodeProtocolPhase {
        self.phase
    }

    pub fn start(&mut self) -> Result<Vec<Vec<u8>>, RNodeProtocolError> {
        if self.phase == RNodeProtocolPhase::Closed {
            return Err(RNodeProtocolError::Closed);
        }
        self.phase = RNodeProtocolPhase::Detecting;
        Ok(vec![
            kiss_encode_command(CMD_DETECT, &[DETECT_REQUEST]),
            kiss_encode_command(CMD_FIRMWARE_VERSION, &[0]),
            kiss_encode_command(CMD_PLATFORM, &[0]),
            kiss_encode_command(CMD_MCU, &[0]),
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
            _ => {}
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
        let cap = self
            .info
            .and_then(|info| info.max_write_size.or(info.negotiated_mtu))
            .unwrap_or(DEFAULT_WRITE_CAP)
            .max(1);
        for chunk in frame.chunks(cap) {
            self.backend.write(chunk.to_vec()).await?;
        }
        Ok(())
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
        assert_eq!(startup.len(), 4);
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
