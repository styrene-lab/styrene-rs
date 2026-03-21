//! `styrene-telemetry` — typed observation schema for Styrene mesh distribution.
//!
//! Carries RF intelligence, position reports, sensor data, and coordination
//! records over the Styrene mesh via LXMF `FIELD_TELEMETRY` (0x02).
//!
//! # Transport model
//!
//! Batches are published to a well-known channel destination derived from
//! `("styrene", "telemetry", type_hex)` and stored in the hub's LXMF
//! propagation node. Peers pull on their normal sync cycle — no subscription
//! tables, offline peers catch up on reconnect.
//!
//! Time-sensitive records (satellite pass imminent, emergency) also go
//! direct LXMF to peers known online via the announce/discovery table.
//!
//! # Type registry
//!
//! Types are a flat u16 namespace, append-only — existing codes are never
//! reassigned. Unknown codes produce [`TelemetryRecord::Unknown`] with the
//! raw CBOR bytes preserved for transparent forwarding.
//!
//! | Range | Category |
//! |---|---|
//! | `0x0001–0x00FF` | Position and tracking |
//! | `0x0100–0x01FF` | Environmental / sensor |
//! | `0x0200–0x02FF` | RF intelligence |
//! | `0x0300–0x03FF` | Mesh service coordination |
//! | `0x0400–0x0FFF` | Fleet / infrastructure |
//! | `0xF000–0xFFFE` | Vendor / experimental |
//! | `0xFFFF` | Invalid |
//!
//! # Three-zone architecture
//!
//! Mirrors [`styrene-content`](../styrene_content):
//!
//! | Zone | Requires | What it provides |
//! |---|---|---|
//! | 0 | nothing (`no_std`, no `alloc`) | Types, decode |
//! | 1 | `alloc` | `encode()` returning `Vec<u8>` |
//! | 2 | `std` / `tokio` | (future: async publish helpers) |
//!
//! # Feature flags
//!
//! | Feature | Enables |
//! |---|---|
//! | *(default)* | Zone 0 + decode (no heap) |
//! | `alloc` | `encode()` returning `alloc::vec::Vec<u8>` |
//! | `std` | implies `alloc` |
//! | `tokio` | implies `std` (future async helpers) |

#![no_std]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

pub mod encode;
pub mod records;
pub mod types;

// ---------------------------------------------------------------------------
// Flat re-exports
// ---------------------------------------------------------------------------

pub use encode::{decode, DecodeError, EncodeError, MAX_ENCODED_BYTES};

#[cfg(feature = "alloc")]
pub use encode::encode;

pub use records::{
    AircraftPosition, AprsPosition, MeshtasticNode, NodeStatus,
    SatellitePass, ServiceAnnouncement, ShipPosition, WeatherObservation,
    MAX_STR, MAX_TEXT,
};

#[allow(unused_imports)]
pub use types::{TelemetryBatch, TelemetryRecord, TelemetryType, MAX_BATCH_RECORDS, MAX_UNKNOWN_BYTES};
pub use encode::encode_to_heapless;
