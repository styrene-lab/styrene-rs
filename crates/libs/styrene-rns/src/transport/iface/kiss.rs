//! KISS framing — byte-stuffed protocol for TNC (Terminal Node Controller) devices.
//!
//! KISS wraps raw data frames in FEND/FESC byte-stuffing for serial transport
//! to LoRa hardware (RNode, RP2040, ESP32). Each KISS frame is:
//!
//! ```text
//! [FEND][CMD][DATA (escaped)][FEND]
//! ```
//!
//! Where CMD = 0x00 for data frames. FEND (0xC0) and FESC (0xDB) within DATA
//! are escaped as FESC+TFEND and FESC+TFESC respectively.
//!
//! Reference: <https://en.wikipedia.org/wiki/KISS_(amateur_radio_protocol)>

/// Frame End delimiter.
pub const FEND: u8 = 0xC0;
/// Frame Escape.
pub const FESC: u8 = 0xDB;
/// Transposed Frame End (follows FESC to represent a literal 0xC0).
pub const TFEND: u8 = 0xDC;
/// Transposed Frame Escape (follows FESC to represent a literal 0xDB).
pub const TFESC: u8 = 0xDD;

/// KISS command: data frame.
pub const CMD_DATA: u8 = 0x00;

/// Encode a raw data frame into KISS framing.
pub fn kiss_encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 4);
    out.push(FEND);
    out.push(CMD_DATA);
    for &b in data {
        match b {
            FEND => {
                out.push(FESC);
                out.push(TFEND);
            }
            FESC => {
                out.push(FESC);
                out.push(TFESC);
            }
            _ => out.push(b),
        }
    }
    out.push(FEND);
    out
}

/// Accumulator for KISS frame decoding from a byte stream.
///
/// Feed bytes via `feed()` and collect complete frames from `take_frame()`.
pub struct KissDecoder {
    buf: Vec<u8>,
    in_frame: bool,
    escape: bool,
    ready: std::collections::VecDeque<Vec<u8>>,
}

impl KissDecoder {
    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(512),
            in_frame: false,
            escape: false,
            ready: std::collections::VecDeque::new(),
        }
    }

    /// Feed raw bytes from the serial port. Call `take_frame()` after to
    /// retrieve any complete frames.
    pub fn feed(&mut self, data: &[u8]) {
        for &b in data {
            if self.escape {
                self.escape = false;
                match b {
                    TFEND => self.buf.push(FEND),
                    TFESC => self.buf.push(FESC),
                    _ => {
                        // Invalid escape — drop byte (per KISS spec)
                    }
                }
                continue;
            }

            match b {
                FEND => {
                    if self.in_frame && !self.buf.is_empty() {
                        // Frame complete — queue it
                        let frame = std::mem::take(&mut self.buf);
                        if let Some(data) = Self::strip_cmd(frame) {
                            self.ready.push_back(data);
                        }
                    }
                    // Reset for next frame
                    self.buf.clear();
                    self.in_frame = true;
                }
                FESC => {
                    self.escape = true;
                }
                _ => {
                    if self.in_frame {
                        self.buf.push(b);
                    }
                }
            }
        }
    }

    /// Strip the CMD byte from a raw frame. Returns data payload for data frames.
    fn strip_cmd(frame: Vec<u8>) -> Option<Vec<u8>> {
        if frame.first() == Some(&CMD_DATA) && frame.len() > 1 {
            Some(frame[1..].to_vec())
        } else if frame.is_empty() {
            None
        } else {
            Some(frame)
        }
    }

    /// Take a complete decoded frame, if available.
    pub fn take_frame(&mut self) -> Option<Vec<u8>> {
        self.ready.pop_front()
    }
}

impl Default for KissDecoder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_simple() {
        let data = b"hello";
        let encoded = kiss_encode(data);
        assert_eq!(encoded[0], FEND);
        assert_eq!(encoded[1], CMD_DATA);
        assert_eq!(&encoded[2..7], b"hello");
        assert_eq!(encoded[7], FEND);
    }

    #[test]
    fn encode_escapes_fend() {
        let data = &[0x01, FEND, 0x02];
        let encoded = kiss_encode(data);
        // Should be: FEND CMD 0x01 FESC TFEND 0x02 FEND
        assert_eq!(encoded, &[FEND, CMD_DATA, 0x01, FESC, TFEND, 0x02, FEND]);
    }

    #[test]
    fn encode_escapes_fesc() {
        let data = &[0x01, FESC, 0x02];
        let encoded = kiss_encode(data);
        assert_eq!(encoded, &[FEND, CMD_DATA, 0x01, FESC, TFESC, 0x02, FEND]);
    }

    #[test]
    fn roundtrip() {
        let data = b"test data with special bytes";
        let encoded = kiss_encode(data);
        let mut decoder = KissDecoder::new();
        decoder.feed(&encoded);
        let decoded = decoder.take_frame().expect("frame");
        assert_eq!(decoded, data);
    }

    #[test]
    fn roundtrip_with_special_bytes() {
        let data = &[0x00, FEND, 0xFF, FESC, 0x42, FEND, FESC];
        let encoded = kiss_encode(data);
        let mut decoder = KissDecoder::new();
        decoder.feed(&encoded);
        let decoded = decoder.take_frame().expect("frame");
        assert_eq!(decoded, data);
    }

    #[test]
    fn multiple_frames() {
        let frame1 = kiss_encode(b"first");
        let frame2 = kiss_encode(b"second");
        let mut combined = frame1;
        combined.extend_from_slice(&frame2);

        let mut decoder = KissDecoder::new();
        decoder.feed(&combined);
        let f1 = decoder.take_frame().expect("frame1");
        assert_eq!(f1, b"first");
        let f2 = decoder.take_frame().expect("frame2");
        assert_eq!(f2, b"second");
        assert!(decoder.take_frame().is_none());
    }

    #[test]
    fn incremental_feed() {
        let encoded = kiss_encode(b"hello");
        let mut decoder = KissDecoder::new();

        // Feed byte by byte
        for &b in &encoded {
            decoder.feed(&[b]);
            // Only take frame after final FEND
        }
        let decoded = decoder.take_frame().expect("incremental frame");
        assert_eq!(decoded, b"hello");
    }

    #[test]
    fn empty_data_produces_no_frame() {
        let mut decoder = KissDecoder::new();
        decoder.feed(&[FEND, FEND]); // empty frame
        assert!(decoder.take_frame().is_none());
    }

    #[test]
    fn binary_payload_roundtrip() {
        // Full 256-byte range
        let data: Vec<u8> = (0..=255).collect();
        let encoded = kiss_encode(&data);
        let mut decoder = KissDecoder::new();
        decoder.feed(&encoded);
        let decoded = decoder.take_frame().expect("binary frame");
        assert_eq!(decoded, data);
    }
}
