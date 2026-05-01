# styrene-telemetry

Typed telemetry schema for Styrene mesh distribution. Encodes/decodes batched observations (ADS-B, APRS, AIS, weather, satellite passes, mesh coordination) as CBOR over LXMF `FIELD_TELEMETRY` (0x02).

## Three-zone `no_std` architecture

| Zone | Feature flag | What it unlocks |
|------|-------------|-----------------|
| 0 | *(default)* | Types, decode, heapless collections. No heap. |
| 1 | `alloc` | `encode()` returning `Vec<u8>` |
| 2 | `std` / `tokio` | Implies alloc. Future: async publish helpers. |

The crate is `#![no_std]` at root. Zones are additive: `std` implies `alloc`.

## Module map

| Module | Purpose |
|--------|---------|
| `types` | `TelemetryType` (u16 registry), `TelemetryRecord` (enum), `TelemetryBatch` (container) |
| `records` | Per-type observation structs: `AircraftPosition`, `AprsPosition`, `MeshtasticNode`, `ShipPosition`, `WeatherObservation`, `SatellitePass`, `ServiceAnnouncement`, `NodeStatus` |
| `encode` | `decode(&[u8])` (all zones), `encode(&TelemetryBatch)` (alloc), `encode_to_heapless()` (alloc, returns fixed-size buffer) |

## Key types

- **`TelemetryType`** -- u16 enum, append-only registry. Ranges: `0x0001-0x00FF` position, `0x0100-0x01FF` environmental, `0x0200-0x02FF` RF intel, `0x0300-0x03FF` coordination, `0x0400-0x0FFF` fleet, `0xF000-0xFFFE` vendor.
- **`TelemetryRecord`** -- enum over typed record structs + `Unknown { type_code, raw_bytes }` for forward-compatible forwarding.
- **`TelemetryBatch`** -- `version: u8`, `timestamp: u64`, `origin: [u8; 16]`, `records: heapless::Vec<TelemetryRecord, 128>`. Published to well-known channel destination `("styrene", "telemetry", type_hex)`.

## Constants

- `MAX_BATCH_RECORDS = 128` -- max records per batch
- `MAX_UNKNOWN_BYTES = 512` -- max raw CBOR preserved for unknown record types
- `MAX_ENCODED_BYTES = 16_384` -- heapless encode buffer cap (16 KiB)
- `MAX_STR = 32`, `MAX_TEXT = 128` -- heapless string caps in record structs

## Feature flags

```toml
default = []       # Zone 0 only
alloc = []         # encode()
std = ["alloc", "ciborium/std"]
tokio = ["std"]    # placeholder for future async
```

## Dependencies

- `heapless` 0.8 (serde) -- fixed-size collections for no_std
- `serde` 1 (derive, no default features)
- `ciborium` 0.2 (no default features) -- CBOR codec
- `ciborium-io` 0.2

## Test commands

```bash
cargo test -p styrene-telemetry --features alloc   # encode/decode roundtrip tests need alloc
cargo test -p styrene-telemetry                     # Zone 0 tests only (types, records)
```

## Gotchas

- All string fields use `heapless::String<N>` -- construction requires `String::try_from()` and can fail if input exceeds N.
- `encode_to_heapless()` requires the `alloc` feature despite returning a heapless buffer (it encodes via `Vec` internally then copies). Without `alloc` it returns `Err(NotSupported)`.
- Unknown type codes produce `TelemetryRecord::Unknown` with raw CBOR bytes preserved -- never an error. This is by design for transparent forwarding.
- `TelemetryBatch.version` must be `1` or `decode()` returns `Err(InvalidVersion)`.
- Records use `f32` for lat/lon (approx 1m precision at equator).
- Several `TelemetryType` variants (VehiclePosition, AirQuality, SkyQuality, SatelliteTelemetry, SpectrumObservation, SignalReport, ServiceQuery, ServiceResponse, ContentInventory) are defined in the type registry but have no corresponding record struct or `TelemetryRecord` variant yet.

## Status

Implemented and tested. Core schema is stable -- type codes are append-only by contract. Zone 2 async helpers are planned but not yet written.
