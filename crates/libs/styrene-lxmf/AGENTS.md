# AGENTS.md -- styrene-lxmf

## What It Does

Rust implementation of the LXMF (Lightweight Extensible Message Format) messaging protocol. Handles message encoding/decoding, payload field packing, delivery state tracking, announce generation, stamp validation, and inbound message decoding. The `sdk` feature adds higher-level domain types for application development.

Library name is `lxmf_core` (not `styrene_lxmf`).

## Current Status

**Active -- used by styrened for LXMF messaging.** Core message paths work. Has benchmarks but zero integration tests. Missing module-level docs throughout.

## Module Map

```
src/
  lib.rs              # Crate root, re-exports LxmfError, Message, Payload, WireMessage
  constants.rs        # Protocol constants (private)
  error.rs            # LxmfError (private, re-exported)
  errors.rs           # Additional error types (public)
  announce.rs         # LXMF announce generation and parsing
  identity.rs         # LXMF identity helpers (wraps rns_core identity)
  inbound_decode.rs   # Decode inbound LXMF messages from wire format
  payload_fields.rs   # Payload field definitions and packing
  wire_fields.rs      # Wire-level field encoding (requires `std`)
  message/
    mod.rs            # Message module root
    container.rs      # Message container type
    delivery.rs       # Delivery state machine
    payload.rs        # Payload struct and encoding
    state.rs          # Message state tracking
    types.rs          # Message type enums
    wire.rs           # WireMessage -- on-the-wire format
  sdk/                # (feature = "sdk")
    mod.rs            # SDK module root
    capability.rs     # Capability definitions
    domain.rs         # Domain model types
    error.rs          # SDK-specific errors (thiserror)
    event.rs          # Event types
    lifecycle.rs      # Message lifecycle management
    profiles.rs       # Profile types
    types.rs          # SDK type definitions
    types/            # Additional type subdirectory
```

## Key Types and Traits

- `Message` -- core LXMF message type
- `Payload` -- message payload with fields
- `WireMessage` -- on-the-wire serialization format
- `LxmfError` -- crate-level error enum

## Feature Flags

| Feature | What it enables |
|---------|----------------|
| `default` | `std` |
| `std` | Standard library, serde_json, wire_fields module |
| `alloc` | Alloc-only (no std) |
| `sdk` | Higher-level domain types, thiserror errors (implies std) |

## Test Commands

```bash
# Unit tests
cargo test -p styrene-lxmf

# With SDK types
cargo test -p styrene-lxmf --features sdk

# Benchmarks
cargo bench -p styrene-lxmf

# All features
cargo test -p styrene-lxmf --all-features
```

## Gotchas

- Library name is `lxmf_core`, not `styrene_lxmf`. Use `use lxmf_core::...` in code.
- Depends on `rns_core` (styrene-rns) via path dependency. Changes to rns_core identity or crypto can break this crate.
- There are two error modules: `error.rs` (private, re-exported as `LxmfError`) and `errors.rs` (public, additional error types). This is confusing -- be aware of which one you are working with.
- `wire_fields.rs` is only available with the `std` feature.
- Serialization uses MessagePack (`rmp-serde` / `rmpv`), not CBOR. This crate predates the CBOR migration decision.

## Known Issues

- Zero integration tests. Only unit tests and benchmarks exist.
- No module-level documentation on any module.
- Dual error modules (`error.rs` + `errors.rs`) need consolidation.
- No interop tests against Python LXMF (unlike styrene-rns which has several).
