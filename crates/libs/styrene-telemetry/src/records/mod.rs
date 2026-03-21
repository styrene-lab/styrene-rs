//! Per-type observation record structs — Zone 0 (no_std, no alloc).
//!
//! All structs use `f32` for lat/lon (≈1 m precision at equator — sufficient
//! for all RF-intelligence use cases), fixed-point integers for non-float
//! quantities, and [`heapless::String`] for text fields.
//!
//! Serde derives are present on all structs so they round-trip through the
//! CBOR encoding layer transparently.

use heapless::String;
use serde::{Deserialize, Serialize};

/// Maximum length for callsigns, IDs, short names, and status strings.
pub const MAX_STR: usize = 32;
/// Maximum length for comments, descriptions, and longer text fields.
pub const MAX_TEXT: usize = 128;

// ---------------------------------------------------------------------------
// Position and tracking (0x0001–0x00FF)
// ---------------------------------------------------------------------------

/// ADS-B aircraft position (`TelemetryType::AircraftPosition = 0x0001`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AircraftPosition {
    /// ICAO 24-bit hex address (e.g. `"A12345"`).
    pub icao: String<MAX_STR>,
    /// Flight callsign if squitter includes it (e.g. `"UAL123"`).
    pub callsign: Option<String<MAX_STR>>,
    /// Latitude in decimal degrees.
    pub lat: f32,
    /// Longitude in decimal degrees.
    pub lon: f32,
    /// Altitude in feet (barometric or geometric).
    pub alt_ft: Option<i32>,
    /// Ground speed in knots.
    pub ground_speed_kt: Option<u16>,
    /// Track / heading in degrees (0–359).
    pub track_deg: Option<u16>,
    /// Squawk code (4 octal digits as string, e.g. `"7700"`).
    pub squawk: Option<String<MAX_STR>>,
    /// Unix timestamp of this observation (seconds).
    pub timestamp: u64,
}

/// APRS position report (`TelemetryType::AprsPosition = 0x0002`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AprsPosition {
    /// Amateur radio callsign with optional SSID (e.g. `"W1AW-9"`).
    pub callsign: String<MAX_STR>,
    /// Latitude in decimal degrees.
    pub lat: f32,
    /// Longitude in decimal degrees.
    pub lon: f32,
    /// APRS symbol table identifier + symbol code (2 chars, e.g. `"/>"`).
    pub symbol: String<MAX_STR>,
    /// Free-text comment from the APRS packet.
    pub comment: Option<String<MAX_TEXT>>,
    /// Altitude in metres, if available.
    pub alt_m: Option<i32>,
    /// Speed in km/h, if available.
    pub speed_kmh: Option<u16>,
    /// Unix timestamp of this observation (seconds).
    pub timestamp: u64,
}

/// Meshtastic network node (`TelemetryType::MeshtasticNode = 0x0003`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshtasticNode {
    /// Meshtastic node ID as hex string (e.g. `"!abcdef12"`).
    pub node_id: String<MAX_STR>,
    /// Short display name (≤4 chars on device).
    pub short_name: String<MAX_STR>,
    /// Long display name, if set.
    pub long_name: Option<String<MAX_STR>>,
    /// Latitude in decimal degrees, if GPS available.
    pub lat: Option<f32>,
    /// Longitude in decimal degrees, if GPS available.
    pub lon: Option<f32>,
    /// Altitude in metres, if GPS available.
    pub alt_m: Option<i32>,
    /// Battery level 0–100, or `None` if unknown / charging.
    pub battery_pct: Option<u8>,
    /// SNR of last received packet, in dB × 4 (e.g. 24 = 6.0 dB).
    pub snr_db_x4: Option<i16>,
    /// Number of hops from the reporting hub.
    pub hops: Option<u8>,
    /// Unix timestamp of this observation (seconds).
    pub timestamp: u64,
}

/// AIS ship position (`TelemetryType::ShipPosition = 0x0004`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShipPosition {
    /// MMSI number (up to 9 digits).
    pub mmsi: u32,
    /// Vessel name from AIS, if available.
    pub vessel_name: Option<String<MAX_STR>>,
    /// Latitude in decimal degrees.
    pub lat: f32,
    /// Longitude in decimal degrees.
    pub lon: f32,
    /// Course over ground in degrees × 10 (e.g. 1800 = 180.0°).
    pub cog_x10: Option<u16>,
    /// Speed over ground in knots × 10 (e.g. 75 = 7.5 kt).
    pub sog_x10: Option<u16>,
    /// AIS navigational status code (0 = under way using engine, etc.).
    pub nav_status: Option<u8>,
    /// Unix timestamp of this observation (seconds).
    pub timestamp: u64,
}

// ---------------------------------------------------------------------------
// Environmental / sensor (0x0100–0x01FF)
// ---------------------------------------------------------------------------

/// Weather observation (`TelemetryType::WeatherObservation = 0x0100`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeatherObservation {
    /// Temperature in °C × 10 (e.g. 215 = 21.5 °C; −50 = −5.0 °C).
    pub temp_c_x10: Option<i16>,
    /// Relative humidity in % (0–100).
    pub humidity_pct: Option<u8>,
    /// Barometric pressure in hPa × 10 (e.g. 10132 = 1013.2 hPa).
    pub pressure_hpa_x10: Option<u16>,
    /// Wind speed in km/h.
    pub wind_speed_kmh: Option<u16>,
    /// Wind direction in degrees (0–359, 0 = from north).
    pub wind_dir_deg: Option<u16>,
    /// Rainfall in mm × 10 accumulated in the last hour (e.g. 25 = 2.5 mm).
    pub rain_mm_x10_1h: Option<u16>,
    /// Unix timestamp of this observation (seconds).
    pub timestamp: u64,
}

// ---------------------------------------------------------------------------
// RF intelligence (0x0200–0x02FF)
// ---------------------------------------------------------------------------

/// Upcoming or completed satellite pass (`TelemetryType::SatellitePass = 0x0200`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SatellitePass {
    /// NORAD catalog number.
    pub norad_id: u32,
    /// Satellite name from TLE catalog.
    pub name: String<MAX_STR>,
    /// Unix timestamp of Acquisition of Signal (AOS), seconds.
    pub aos_unix: u64,
    /// Unix timestamp of Loss of Signal (LOS), seconds.
    pub los_unix: u64,
    /// Maximum elevation above horizon in degrees (0–90).
    pub max_elevation_deg: u8,
    /// Azimuth at AOS in degrees (0–359).
    pub aos_azimuth_deg: u16,
    /// Downlink frequency in Hz, if known (e.g. 137_500_000 for NOAA APT).
    pub frequency_hz: Option<u32>,
    /// Whether this hub intends to attempt a decode during this pass.
    pub decode_planned: bool,
}

// ---------------------------------------------------------------------------
// Mesh service coordination (0x0300–0x03FF)
// ---------------------------------------------------------------------------

/// Mesh service announcement (`TelemetryType::ServiceAnnouncement = 0x0300`).
///
/// Used by the Styrene Service Directory protocol. Hubs broadcast these
/// to let peers discover available services and their current state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceAnnouncement {
    /// Well-known service type identifier (e.g. `"adsb"`, `"kiwix"`, `"satellite"`).
    pub service_type: String<MAX_STR>,
    /// Announcing node identity hash (16 bytes).
    pub node_identity: [u8; 16],
    /// Human-readable service status (`"active"`, `"degraded"`, `"offline"`).
    pub status: String<MAX_STR>,
    /// Geographic coverage bounding box \[west, south, east, north\] in
    /// degrees × 100 (e.g. −7400 = −74.00°). `None` = no geographic constraint.
    pub bbox_x100: Option<[i32; 4]>,
    /// Opaque service-specific metadata (CBOR bytes, max 256 bytes).
    /// Callers decode this according to `service_type`.
    pub metadata: Option<heapless::Vec<u8, 256>>,
    /// Unix timestamp of this announcement (seconds).
    pub timestamp: u64,
}

// ---------------------------------------------------------------------------
// Fleet / infrastructure (0x0400–0x0FFF)
// ---------------------------------------------------------------------------

/// Styrene node status report (`TelemetryType::NodeStatus = 0x0400`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeStatus {
    /// Node identity hash (16 bytes).
    pub node_identity: [u8; 16],
    /// Hub capability flags — same bit layout as `HubAnnounce::capabilities`.
    pub capability_flags: u16,
    /// Process uptime in seconds.
    pub uptime_secs: u64,
    /// 1-minute load average × 100 (e.g. 150 = 1.50). `None` on MCU targets.
    pub load_x100: Option<u16>,
    /// Used storage in MiB. `None` if unavailable.
    pub storage_used_mib: Option<u32>,
    /// Free storage in MiB. `None` if unavailable.
    pub storage_free_mib: Option<u32>,
    /// Unix timestamp of this report (seconds).
    pub timestamp: u64,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip a struct through serde_json as a sanity check on derives.
    /// (Wire encoding uses CBOR; JSON is simpler to construct in tests.)
    macro_rules! json_roundtrip {
        ($name:ident, $val:expr) => {
            #[test]
            fn $name() {
                let original = $val;
                // Verify serde derives work: serialize then parse as a generic Value.
                let json = serde_json::to_string(&original)
                    .expect("serialize failed");
                let _value: serde_json::Value = serde_json::from_str(&json)
                    .expect("deserialize to Value failed");
                // If we got here, serde round-trip is valid.
            }
        };
    }

    json_roundtrip!(aircraft_position_roundtrip, AircraftPosition {
        icao: String::try_from("A1B2C3").unwrap(),
        callsign: Some(String::try_from("UAL123").unwrap()),
        lat: 40.712_8,
        lon: -74.006,
        alt_ft: Some(35_000),
        ground_speed_kt: Some(480),
        track_deg: Some(270),
        squawk: Some(String::try_from("1200").unwrap()),
        timestamp: 1_700_000_000,
    });

    json_roundtrip!(aprs_position_roundtrip, AprsPosition {
        callsign: String::try_from("W1AW-9").unwrap(),
        lat: 41.714_7,
        lon: -72.727_3,
        symbol: String::try_from("/>").unwrap(),
        comment: Some(String::try_from("Mobile").unwrap()),
        alt_m: Some(45),
        speed_kmh: Some(60),
        timestamp: 1_700_000_000,
    });

    json_roundtrip!(meshtastic_node_roundtrip, MeshtasticNode {
        node_id: String::try_from("!abcdef12").unwrap(),
        short_name: String::try_from("NODE").unwrap(),
        long_name: Some(String::try_from("Field Node 1").unwrap()),
        lat: Some(40.712_8),
        lon: Some(-74.006),
        alt_m: Some(10),
        battery_pct: Some(87),
        snr_db_x4: Some(24),
        hops: Some(1),
        timestamp: 1_700_000_000,
    });

    json_roundtrip!(ship_position_roundtrip, ShipPosition {
        mmsi: 123_456_789,
        vessel_name: Some(String::try_from("MV Styrene").unwrap()),
        lat: 40.6_f32,
        lon: -74.1_f32,
        cog_x10: Some(900),
        sog_x10: Some(120),
        nav_status: Some(0),
        timestamp: 1_700_000_000,
    });

    json_roundtrip!(weather_observation_roundtrip, WeatherObservation {
        temp_c_x10: Some(215),
        humidity_pct: Some(68),
        pressure_hpa_x10: Some(10132),
        wind_speed_kmh: Some(15),
        wind_dir_deg: Some(270),
        rain_mm_x10_1h: Some(0),
        timestamp: 1_700_000_000,
    });

    json_roundtrip!(satellite_pass_roundtrip, SatellitePass {
        norad_id: 28_654,
        name: String::try_from("NOAA 18").unwrap(),
        aos_unix: 1_700_001_000,
        los_unix: 1_700_001_900,
        max_elevation_deg: 72,
        aos_azimuth_deg: 320,
        frequency_hz: Some(137_912_500),
        decode_planned: true,
    });

    json_roundtrip!(node_status_roundtrip, NodeStatus {
        node_identity: [0xAAu8; 16],
        capability_flags: 0x00C3,
        uptime_secs: 86_400,
        load_x100: Some(42),
        storage_used_mib: Some(220),
        storage_free_mib: Some(780),
        timestamp: 1_700_000_000,
    });
}
