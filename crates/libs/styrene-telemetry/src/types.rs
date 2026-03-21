//! Core telemetry types: type registry, `TelemetryRecord`, `TelemetryBatch`.
//!
//! All types are Zone 0 — `no_std`, no `alloc`, [`heapless`] collections.

use heapless::Vec as HVec;
use serde::{Deserialize, Serialize};

#[allow(unused_imports)]
use crate::records::{
    AircraftPosition, AprsPosition, MeshtasticNode, NodeStatus, SatellitePass,
    ServiceAnnouncement, ShipPosition, WeatherObservation,
};

// ---------------------------------------------------------------------------
// Type registry
// ---------------------------------------------------------------------------

/// Maximum records in a [`TelemetryBatch`] in Zone 0.
pub const MAX_BATCH_RECORDS: usize = 128;

/// Maximum raw bytes stored for an `Unknown` record in Zone 0.
pub const MAX_UNKNOWN_BYTES: usize = 512;

/// Telemetry record type codes — u16 append-only registry.
///
/// Unknown codes produce [`TelemetryRecord::Unknown`] — never an error.
/// Existing codes are **never reassigned**.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u16)]
pub enum TelemetryType {
    // 0x0001–0x00FF: Position and tracking
    /// ADS-B aircraft position.
    AircraftPosition    = 0x0001,
    /// APRS position report.
    AprsPosition        = 0x0002,
    /// Meshtastic network node.
    MeshtasticNode      = 0x0003,
    /// AIS ship position.
    ShipPosition        = 0x0004,
    /// Generic ground vehicle position.
    VehiclePosition     = 0x0005,

    // 0x0100–0x01FF: Environmental / sensor
    /// Weather observation.
    WeatherObservation  = 0x0100,
    /// Air quality metrics (PM2.5, PM10, AQI).
    AirQuality          = 0x0101,
    /// Sky quality meter reading (SQM-LE, mag/arcsec²).
    SkyQuality          = 0x0102,

    // 0x0200–0x02FF: RF intelligence
    /// Upcoming or completed satellite pass.
    SatellitePass       = 0x0200,
    /// Decoded telemetry from a satellite pass.
    SatelliteTelemetry  = 0x0201,
    /// Spectrum power observation at a specific frequency.
    SpectrumObservation = 0x0202,
    /// Ham radio signal report (RST, frequency, mode, callsign).
    SignalReport        = 0x0203,

    // 0x0300–0x03FF: Mesh service coordination
    /// Styrene Service Directory announcement.
    ServiceAnnouncement = 0x0300,
    /// Service Directory query.
    ServiceQuery        = 0x0301,
    /// Service Directory response.
    ServiceResponse     = 0x0302,

    // 0x0400–0x0FFF: Styrene fleet / infrastructure
    /// Node status report (uptime, capabilities, storage).
    NodeStatus          = 0x0400,
    /// Content inventory update (ZIM collections, Ollama models).
    ContentInventory    = 0x0401,
}

impl TelemetryType {
    /// Convert a raw u16 to a [`TelemetryType`], returning `None` for unknown codes.
    pub fn from_u16(v: u16) -> Option<Self> {
        match v {
            0x0001 => Some(Self::AircraftPosition),
            0x0002 => Some(Self::AprsPosition),
            0x0003 => Some(Self::MeshtasticNode),
            0x0004 => Some(Self::ShipPosition),
            0x0005 => Some(Self::VehiclePosition),
            0x0100 => Some(Self::WeatherObservation),
            0x0101 => Some(Self::AirQuality),
            0x0102 => Some(Self::SkyQuality),
            0x0200 => Some(Self::SatellitePass),
            0x0201 => Some(Self::SatelliteTelemetry),
            0x0202 => Some(Self::SpectrumObservation),
            0x0203 => Some(Self::SignalReport),
            0x0300 => Some(Self::ServiceAnnouncement),
            0x0301 => Some(Self::ServiceQuery),
            0x0302 => Some(Self::ServiceResponse),
            0x0400 => Some(Self::NodeStatus),
            0x0401 => Some(Self::ContentInventory),
            _ => None,
        }
    }

    /// The raw u16 wire code.
    #[inline]
    pub fn as_u16(self) -> u16 {
        self as u16
    }

    /// Human-readable category name for logging.
    pub fn category(self) -> &'static str {
        match self {
            Self::AircraftPosition | Self::AprsPosition
            | Self::MeshtasticNode | Self::ShipPosition
            | Self::VehiclePosition => "position",
            Self::WeatherObservation | Self::AirQuality
            | Self::SkyQuality => "environmental",
            Self::SatellitePass | Self::SatelliteTelemetry
            | Self::SpectrumObservation | Self::SignalReport => "rf-intelligence",
            Self::ServiceAnnouncement | Self::ServiceQuery
            | Self::ServiceResponse => "coordination",
            Self::NodeStatus | Self::ContentInventory => "fleet",
        }
    }
}

// ---------------------------------------------------------------------------
// TelemetryRecord
// ---------------------------------------------------------------------------

/// A single typed observation.
///
/// Unknown type codes produce the `Unknown` variant with the raw CBOR bytes
/// preserved — forward-compatible forwarding without decode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TelemetryRecord {
    /// ADS-B aircraft position.
    AircraftPosition(AircraftPosition),
    /// APRS amateur radio position.
    AprsPosition(AprsPosition),
    /// Meshtastic network node observation.
    MeshtasticNode(MeshtasticNode),
    /// AIS ship position.
    ShipPosition(ShipPosition),
    /// Weather observation.
    WeatherObservation(WeatherObservation),
    /// Satellite pass window.
    SatellitePass(SatellitePass),
    /// Mesh service announcement.
    ServiceAnnouncement(ServiceAnnouncement),
    /// Node status report.
    NodeStatus(NodeStatus),
    /// Unrecognised type — raw CBOR payload preserved for forwarding.
    Unknown {
        /// The u16 type code from the wire.
        type_code: u16,
        /// CBOR-encoded payload (up to [`MAX_UNKNOWN_BYTES`]).
        raw_bytes: HVec<u8, MAX_UNKNOWN_BYTES>,
    },
}

impl TelemetryRecord {
    /// The u16 type code for this record (matches [`TelemetryType::as_u16`]).
    pub fn record_type(&self) -> u16 {
        match self {
            Self::AircraftPosition(_)    => TelemetryType::AircraftPosition as u16,
            Self::AprsPosition(_)        => TelemetryType::AprsPosition as u16,
            Self::MeshtasticNode(_)      => TelemetryType::MeshtasticNode as u16,
            Self::ShipPosition(_)        => TelemetryType::ShipPosition as u16,
            Self::WeatherObservation(_)  => TelemetryType::WeatherObservation as u16,
            Self::SatellitePass(_)       => TelemetryType::SatellitePass as u16,
            Self::ServiceAnnouncement(_) => TelemetryType::ServiceAnnouncement as u16,
            Self::NodeStatus(_)          => TelemetryType::NodeStatus as u16,
            Self::Unknown { type_code, .. } => *type_code,
        }
    }
}

// ---------------------------------------------------------------------------
// TelemetryBatch
// ---------------------------------------------------------------------------

/// A batch of typed observations, carried in LXMF `FIELD_TELEMETRY` (0x02).
///
/// Wire format: CBOR-encoded [`TelemetryBatch`] via [`crate::encode`].
///
/// Addressing: published to a well-known channel destination derived from
/// `("styrene", "telemetry", type_hex)` and stored in the hub propagation
/// node. Peers pull on their normal sync cycle. Unknown record types are
/// preserved as [`TelemetryRecord::Unknown`] for transparent forwarding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryBatch {
    /// Schema version (currently `1`).
    pub version: u8,
    /// Unix timestamp (seconds) when this batch was assembled.
    pub timestamp: u64,
    /// Origin node identity hash (first 16 bytes of identity hash).
    pub origin: [u8; 16],
    /// Observation records — heterogeneous, ordered by observation time.
    pub records: HVec<TelemetryRecord, MAX_BATCH_RECORDS>,
}

impl TelemetryBatch {
    /// Create an empty batch for the given origin and timestamp.
    pub fn new(origin: [u8; 16], timestamp: u64) -> Self {
        Self {
            version: 1,
            timestamp,
            origin,
            records: HVec::new(),
        }
    }

    /// Add a record to the batch.
    ///
    /// Returns `Err(record)` if the batch is already at [`MAX_BATCH_RECORDS`].
    pub fn push(&mut self, record: TelemetryRecord) -> Result<(), TelemetryRecord> {
        self.records.push(record).map_err(|r| r)
    }

    /// Number of records in this batch.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// True if the batch contains no records.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// True if the batch is at capacity (`MAX_BATCH_RECORDS`).
    pub fn is_full(&self) -> bool {
        self.records.len() == MAX_BATCH_RECORDS
    }

    /// Iterate over records with the given type code.
    pub fn of_type(&self, type_code: u16) -> impl Iterator<Item = &TelemetryRecord> {
        self.records.iter().filter(move |r| r.record_type() == type_code)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::AircraftPosition;
    use heapless::String;

    fn sample_aircraft() -> TelemetryRecord {
        TelemetryRecord::AircraftPosition(AircraftPosition {
            icao: String::try_from("AABBCC").unwrap(),
            callsign: None,
            lat: 40.7,
            lon: -74.0,
            alt_ft: Some(10_000),
            ground_speed_kt: None,
            track_deg: None,
            squawk: None,
            timestamp: 1_700_000_000,
        })
    }

    #[test]
    fn batch_starts_empty() {
        let b = TelemetryBatch::new([0u8; 16], 0);
        assert!(b.is_empty());
        assert_eq!(b.len(), 0);
        assert!(!b.is_full());
    }

    #[test]
    fn batch_push_and_len() {
        let mut b = TelemetryBatch::new([0u8; 16], 1_700_000_000);
        b.push(sample_aircraft()).unwrap();
        assert_eq!(b.len(), 1);
        assert!(!b.is_empty());
    }

    #[test]
    fn batch_full_returns_err_on_push() {
        let mut b = TelemetryBatch::new([0u8; 16], 0);
        for _ in 0..MAX_BATCH_RECORDS {
            b.push(sample_aircraft()).unwrap();
        }
        assert!(b.is_full());
        let result = b.push(sample_aircraft());
        assert!(result.is_err(), "push into full batch should return Err");
    }

    #[test]
    fn telemetry_type_roundtrip() {
        let types = [
            TelemetryType::AircraftPosition,
            TelemetryType::AprsPosition,
            TelemetryType::MeshtasticNode,
            TelemetryType::ShipPosition,
            TelemetryType::WeatherObservation,
            TelemetryType::SatellitePass,
            TelemetryType::ServiceAnnouncement,
            TelemetryType::NodeStatus,
        ];
        for t in types {
            let code = t.as_u16();
            assert_eq!(TelemetryType::from_u16(code), Some(t),
                "from_u16({code:#06x}) should return {:?}", t);
        }
    }

    #[test]
    fn unknown_type_code_returns_none() {
        assert_eq!(TelemetryType::from_u16(0xDEAD), None);
        assert_eq!(TelemetryType::from_u16(0xFFFF), None);
        assert_eq!(TelemetryType::from_u16(0x0000), None);
    }

    #[test]
    fn record_type_returns_correct_code() {
        let r = sample_aircraft();
        assert_eq!(r.record_type(), TelemetryType::AircraftPosition as u16);
    }

    #[test]
    fn unknown_record_type_code_preserved() {
        let r = TelemetryRecord::Unknown {
            type_code: 0xBEEF,
            raw_bytes: HVec::new(),
        };
        assert_eq!(r.record_type(), 0xBEEF);
    }

    #[test]
    fn of_type_filters_correctly() {
        let mut b = TelemetryBatch::new([0u8; 16], 0);
        b.push(sample_aircraft()).unwrap();
        b.push(TelemetryRecord::Unknown {
            type_code: 0x9999,
            raw_bytes: HVec::new(),
        })
        .unwrap();
        let aircraft: alloc::vec::Vec<_> = b
            .of_type(TelemetryType::AircraftPosition as u16)
            .collect();
        assert_eq!(aircraft.len(), 1);
    }
}
