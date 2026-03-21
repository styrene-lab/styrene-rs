//! CBOR encode/decode for [`TelemetryBatch`].
//!
//! Follows the same pattern as `styrene-content/src/manifest.rs`:
//! - Decoding is available in all zones (reads from `&[u8]`)
//! - Encoding requires the `alloc` feature (needs a growable buffer)
//!
//! Wire format: a single CBOR item — the [`TelemetryBatch`] struct serialised
//! by ciborium. The struct's serde derives handle field ordering; ciborium
//! uses deterministic CBOR map encoding.

use crate::types::{TelemetryBatch, MAX_BATCH_RECORDS};

/// Conservative upper bound on encoded batch size.
///
/// Used for heapless output buffers. Actual size is typically much smaller
/// (an aircraft batch of 20 records is ~1–2 KB).
pub const MAX_ENCODED_BYTES: usize = 16_384; // 16 KiB

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Errors from [`encode`] / [`encode_to_heapless`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodeError {
    /// CBOR serialisation failed.
    Encode,
    /// Encoded size exceeded [`MAX_ENCODED_BYTES`].
    TooLarge,
    /// Encoding not available without `alloc` feature.
    NotSupported,
}

/// Errors from [`decode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// CBOR deserialisation failed.
    Decode,
    /// Batch version field is not recognised.
    InvalidVersion,
}

// ---------------------------------------------------------------------------
// Decode (all zones)
// ---------------------------------------------------------------------------

/// Decode a [`TelemetryBatch`] from CBOR bytes.
///
/// Available in all zones — no heap allocation required.
/// Unknown record types decode to [`crate::types::TelemetryRecord::Unknown`].
pub fn decode(bytes: &[u8]) -> Result<TelemetryBatch, DecodeError> {
    let batch: TelemetryBatch =
        ciborium::from_reader(bytes).map_err(|_| DecodeError::Decode)?;
    if batch.version != 1 {
        return Err(DecodeError::InvalidVersion);
    }
    Ok(batch)
}

// ---------------------------------------------------------------------------
// Encode (alloc feature)
// ---------------------------------------------------------------------------

/// Encode a [`TelemetryBatch`] to a CBOR byte vector.
///
/// Requires the `alloc` feature.
#[cfg(feature = "alloc")]
pub fn encode(batch: &TelemetryBatch) -> Result<alloc::vec::Vec<u8>, EncodeError> {
    let mut out = alloc::vec::Vec::new();
    ciborium::into_writer(batch, &mut out).map_err(|_| EncodeError::Encode)?;
    Ok(out)
}

/// Encode a [`TelemetryBatch`] into a fixed-size heapless buffer.
///
/// Returns `Err(EncodeError::TooLarge)` if the encoded output exceeds
/// [`MAX_ENCODED_BYTES`]. Useful in contexts that have `alloc` temporarily
/// available but need to hand off a fixed-size buffer.
pub fn encode_to_heapless(
    batch: &TelemetryBatch,
) -> Result<heapless::Vec<u8, MAX_ENCODED_BYTES>, EncodeError> {
    #[cfg(feature = "alloc")]
    {
        let bytes = encode(batch)?;
        heapless::Vec::from_slice(&bytes).map_err(|_| EncodeError::TooLarge)
    }
    #[cfg(not(feature = "alloc"))]
    {
        let _ = batch;
        Err(EncodeError::NotSupported)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(all(test, feature = "alloc"))]
mod tests {
    use super::*;
    use crate::records::AircraftPosition;
    use crate::types::{TelemetryRecord, TelemetryType};
    use heapless::String;

    fn sample_batch() -> TelemetryBatch {
        let mut batch = TelemetryBatch::new([0xAAu8; 16], 1_700_000_000);
        batch
            .push(TelemetryRecord::AircraftPosition(AircraftPosition {
                icao: String::try_from("AABBCC").unwrap(),
                callsign: Some(String::try_from("TEST1").unwrap()),
                lat: 40.712_8,
                lon: -74.006,
                alt_ft: Some(35_000),
                ground_speed_kt: Some(480),
                track_deg: Some(270),
                squawk: Some(String::try_from("7700").unwrap()),
                timestamp: 1_700_000_000,
            }))
            .unwrap();
        batch
    }

    #[test]
    fn encode_decode_roundtrip() {
        let original = sample_batch();
        let bytes = encode(&original).expect("encode failed");
        assert!(!bytes.is_empty());

        let decoded = decode(&bytes).expect("decode failed");
        assert_eq!(decoded.version, original.version);
        assert_eq!(decoded.timestamp, original.timestamp);
        assert_eq!(decoded.origin, original.origin);
        assert_eq!(decoded.len(), original.len());
        assert_eq!(
            decoded.records[0].record_type(),
            TelemetryType::AircraftPosition as u16
        );
    }

    #[test]
    fn empty_batch_roundtrip() {
        let original = TelemetryBatch::new([0u8; 16], 0);
        let bytes = encode(&original).expect("encode failed");
        let decoded = decode(&bytes).expect("decode failed");
        assert!(decoded.is_empty());
    }

    #[test]
    fn decode_rejects_wrong_version() {
        let mut batch = sample_batch();
        batch.version = 99;
        let bytes = encode(&batch).expect("encode failed");
        assert!(matches!(decode(&bytes), Err(DecodeError::InvalidVersion)));
    }

    #[test]
    fn decode_rejects_garbage() {
        assert!(matches!(
            decode(b"not cbor data!!!"),
            Err(DecodeError::Decode)
        ));
    }

    #[test]
    fn encode_to_heapless_roundtrip() {
        let batch = sample_batch();
        let buf = encode_to_heapless(&batch).expect("encode_to_heapless failed");
        let decoded = decode(&buf).expect("decode failed");
        assert_eq!(decoded.timestamp, batch.timestamp);
    }
}
