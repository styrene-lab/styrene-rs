//! Hardware TRNG coprocessor source — nRF52840 via UART.
//!
//! Communicates with an nRF52840 module running the Styrene entropy coprocessor
//! firmware over a UART link at 1 Mbaud. The protocol is defined in
//! `styrene/research/entropy-coprocessor.md`.
//!
//! Enabled by the `hardware-trng` feature.

use serialport::SerialPort;
use std::{io, time::Duration};

use crate::{
    health::{HealthChecker, HealthError, SourceHealth},
    pool::{EntropyPool, SourceId},
};
use super::EntropySource;

/// Frame sync byte marking the start of every message.
const SYNC_BYTE: u8 = 0xAA;

/// Message types from the nRF52840 firmware.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MsgType {
    /// Conditioned DRBG output bytes.
    EntropyData = 0x01,
    /// Health report from the coprocessor.
    HealthReport = 0x02,
    /// Request N bytes from host (response: EntropyData).
    Request = 0x03,
    /// Request TRNG reseed cycle from host.
    Reset = 0x04,
    /// Query firmware version and source health.
    Status = 0x05,
}

impl TryFrom<u8> for MsgType {
    type Error = u8;
    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v {
            0x01 => Ok(Self::EntropyData),
            0x02 => Ok(Self::HealthReport),
            0x03 => Ok(Self::Request),
            0x04 => Ok(Self::Reset),
            0x05 => Ok(Self::Status),
            other => Err(other),
        }
    }
}

/// CRC-8 (MAXIM/Dallas) for frame integrity checking.
fn crc8(data: &[u8]) -> u8 {
    let mut crc: u8 = 0x00;
    for &byte in data {
        let mut cur = byte;
        for _ in 0..8 {
            if ((crc ^ cur) & 0x01) != 0 {
                crc = (crc >> 1) ^ 0x8C;
            } else {
                crc >>= 1;
            }
            cur >>= 1;
        }
    }
    crc
}

/// A decoded frame from the coprocessor.
#[derive(Debug)]
struct Frame {
    msg_type: MsgType,
    payload: Vec<u8>,
}

/// nRF52840 UART entropy coprocessor source.
///
/// Opens a serial port and reads conditioned entropy output from the coprocessor
/// firmware. Health reports from the coprocessor update local health state.
///
/// # Example
///
/// ```no_run
/// use styrene_entropy::source::HardwareSource;
///
/// let src = HardwareSource::open("/dev/ttyUSB0").expect("failed to open serial port");
/// ```
pub struct HardwareSource {
    port: Box<dyn SerialPort>,
    health: SourceHealth,
    checker: HealthChecker,
    port_path: String,
}

impl std::fmt::Debug for HardwareSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HardwareSource")
            .field("port_path", &self.port_path)
            .field("health", &self.health)
            .finish()
    }
}

impl HardwareSource {
    /// Open a serial port to the nRF52840 entropy coprocessor.
    ///
    /// Baud rate: 1,000,000 (1 Mbaud). Read timeout: 500 ms.
    pub fn open(port_path: &str) -> Result<Self, serialport::Error> {
        let port = serialport::new(port_path, 1_000_000)
            .timeout(Duration::from_millis(500))
            .open()?;

        Ok(Self {
            port,
            health: SourceHealth::Ok,
            checker: HealthChecker::default(),
            port_path: port_path.to_owned(),
        })
    }

    /// Read and decode one frame from the serial port.
    fn read_frame(&mut self) -> io::Result<Frame> {
        // Scan for sync byte
        let mut sync = [0u8; 1];
        loop {
            self.port.read_exact(&mut sync)?;
            if sync[0] == SYNC_BYTE {
                break;
            }
        }

        let mut header = [0u8; 2]; // [LEN, TYPE]
        self.port.read_exact(&mut header)?;
        let len = header[0] as usize;
        let type_byte = header[1];

        let mut payload = vec![0u8; len];
        self.port.read_exact(&mut payload)?;

        let mut crc_buf = [0u8; 1];
        self.port.read_exact(&mut crc_buf)?;
        let received_crc = crc_buf[0];

        // Verify CRC over TYPE || PAYLOAD
        let mut crc_data = vec![type_byte];
        crc_data.extend_from_slice(&payload);
        let expected_crc = crc8(&crc_data);

        if received_crc != expected_crc {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("CRC mismatch: got 0x{received_crc:02x}, expected 0x{expected_crc:02x}"),
            ));
        }

        let msg_type = MsgType::try_from(type_byte).map_err(|unknown| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown message type: 0x{unknown:02x}"),
            )
        })?;

        Ok(Frame { msg_type, payload })
    }

    /// Send a REQUEST frame asking for `n` bytes of entropy.
    fn send_request(&mut self, n: u16) -> io::Result<()> {
        let payload = n.to_le_bytes();
        let type_byte = MsgType::Request as u8;
        let crc = crc8(&[type_byte, payload[0], payload[1]]);
        let frame = [SYNC_BYTE, 2, type_byte, payload[0], payload[1], crc];
        self.port.write_all(&frame)?;
        Ok(())
    }

    /// Send a RESET frame to trigger coprocessor TRNG reseed.
    fn send_reset(&mut self) -> io::Result<()> {
        let type_byte = MsgType::Reset as u8;
        let crc = crc8(&[type_byte]);
        let frame = [SYNC_BYTE, 0, type_byte, crc];
        self.port.write_all(&frame)?;
        Ok(())
    }

    /// Transition to degraded state and log the reason.
    fn degrade(&mut self, reason: String) {
        log::warn!("HardwareSource {}: degraded — {}", self.port_path, reason);
        self.health = SourceHealth::Degraded(reason);
    }

    /// Process a decoded frame, updating health and returning entropy bytes if any.
    fn process_frame(&mut self, frame: Frame) -> Option<Vec<u8>> {
        match frame.msg_type {
            MsgType::EntropyData => {
                // Run health checks on raw output
                match self.checker.update(&frame.payload) {
                    Ok(()) => Some(frame.payload),
                    Err(HealthError::RepetitionCount { byte, count, limit }) => {
                        self.degrade(format!(
                            "RCT failure: byte 0x{byte:02x} repeated {count}/{limit}"
                        ));
                        // Signal coprocessor to reseed
                        let _ = self.send_reset();
                        None
                    }
                    Err(HealthError::AdaptiveProportion { ones, window, pct }) => {
                        self.degrade(format!(
                            "APT failure: {ones}/{window} ones ({pct:.1}%)"
                        ));
                        let _ = self.send_reset();
                        None
                    }
                }
            }
            MsgType::HealthReport => {
                // Payload: [ok: u8, raw_bias_pct: u8, stuck_bit_mask: u8, rct_failures: u8]
                if frame.payload.len() >= 4 {
                    let ok = frame.payload[0] != 0;
                    if !ok {
                        let bias = frame.payload[1];
                        let stuck = frame.payload[2];
                        self.degrade(format!(
                            "coprocessor health report: bias={bias}%, stuck_mask=0x{stuck:02x}"
                        ));
                        let _ = self.send_reset();
                    } else if !self.health.is_ok() {
                        // Coprocessor recovered
                        log::info!("HardwareSource {}: recovered", self.port_path);
                        self.health = SourceHealth::Ok;
                        self.checker.reset();
                    }
                }
                None
            }
            // Other frame types are informational — not entropy data
            _ => None,
        }
    }
}

impl EntropySource for HardwareSource {
    fn source_id(&self) -> SourceId {
        SourceId::HARDWARE
    }

    fn health(&self) -> SourceHealth {
        self.health.clone()
    }

    fn poll(&mut self, pool: &mut EntropyPool) {
        if !self.health.is_ok() {
            return;
        }

        // Request 64 bytes from coprocessor
        if let Err(e) = self.send_request(64) {
            self.degrade(format!("serial write error: {e}"));
            return;
        }

        // Read frames until we get EntropyData or timeout
        for _ in 0..4 {
            match self.read_frame() {
                Ok(frame) => {
                    if let Some(entropy) = self.process_frame(frame) {
                        pool.add(SourceId::HARDWARE, &entropy);
                        return;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::TimedOut => {
                    self.degrade(format!("read timeout: {e}"));
                    return;
                }
                Err(e) => {
                    self.degrade(format!("read error: {e}"));
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc8_known_value() {
        // CRC-8/MAXIM of [0x01] = 0x2F
        assert_eq!(crc8(&[0x01]), 0x2f);
    }

    #[test]
    fn crc8_empty_is_zero() {
        assert_eq!(crc8(&[]), 0x00);
    }

    #[test]
    fn crc8_detects_corruption() {
        let data = [0x01, 0x02, 0x03, 0x04];
        let crc = crc8(&data);
        let mut corrupted = data;
        corrupted[2] ^= 0xFF;
        assert_ne!(crc, crc8(&corrupted));
    }
}
