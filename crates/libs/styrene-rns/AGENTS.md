# AGENTS.md -- styrene-rns

## What It Does

Rust implementation of the Reticulum Network Stack (RNS) protocol core. Provides identity management (X25519 + Ed25519), destinations, links, resources, ratchets, packet encoding/decoding, and cryptographic primitives. The transport layer (TCP, UDP, Serial/KISS interfaces) is behind the `transport` feature flag.

Library name is `rns_core` (not `styrene_rns`).

## Current Status

**Primary protocol crate -- actively used by styrened and styrene-lxmf.** Has comprehensive interop tests against Python RNS. Transport layer is functional with TCP, UDP, and Serial interfaces.

## Module Map

```
src/
  lib.rs                # Crate root, re-exports
  identity.rs           # X25519+Ed25519 keypair, sign/verify, encrypt/decrypt
  destination.rs        # RNS destinations (Single, Group, Plain)
    destination/
      primitives.rs     # Low-level destination ops
      ratchet.rs        # Destination-level ratchet state
      tests.rs          # Destination unit tests
  destination_hash.rs   # Truncated hash for addressing
  hash.rs               # Hash utilities, lxmf_address_hash
  packet.rs             # Packet encode/decode, LXMF_MAX_PAYLOAD
  buffer.rs             # Buffer utilities
  key_manager.rs        # Key storage and lookup
  ratchets.rs           # Ratchet state management
  error.rs              # RnsError
  serde.rs              # Custom serde helpers
  crypt.rs              # Cryptographic module root
    crypt/
      fernet.rs         # Fernet token encrypt/decrypt (AES-128-CBC + HMAC)
  transport/            # (feature = "transport")
    mod.rs              # Transport root
    config.rs           # Transport configuration
    channel.rs          # Channel abstraction
    channel_buffer.rs   # Buffered channel I/O
    delivery.rs         # Message delivery pipeline
    receipt.rs          # Delivery receipts
    embedded_link.rs    # Link management
    identity_bridge.rs  # Identity <-> transport bridge
    ratchet_store.rs    # Persistent ratchet storage (SQLite)
    time.rs             # Time utilities
    error.rs            # Transport-specific errors
    core_transport/     # Core transport loop
    destination_ext/    # Destination extensions for transport
    identity_ext/       # Identity extensions for transport
    iface/              # Network interfaces (TCP, UDP, Serial/KISS)
    resource/           # Resource transfer
    storage/            # Persistent storage
    utils/              # Transport utilities
```

## Key Types and Traits

- `Identity` -- X25519+Ed25519 keypair; `encrypt()`, `decrypt()`, `sign()`, `verify()`
- `Destination` -- RNS destination with hash, type, direction, app name
- `DestinationHash` -- 16-byte truncated hash used for routing
- `Packet` -- wire-format packet with header, payload, IFAC
- `RnsError` -- crate-level error enum
- `group_encrypt` / `group_decrypt` -- group destination crypto (re-exported)

## Feature Flags

| Feature | What it enables |
|---------|----------------|
| `default` | `std` |
| `std` | Standard library support |
| `alloc` | Alloc-only (no std) |
| `fernet-aes128` | Fernet AES-128-CBC token format |
| `interop-tests` | Cross-language interop test support |
| `transport` | Full transport layer: TCP, UDP, Serial, SQLite storage, tokio runtime |
| `serial` | Serial/KISS interface (implies transport) |

## Test Commands

```bash
# Unit tests (default features)
cargo test -p styrene-rns

# With transport layer
cargo test -p styrene-rns --features transport

# Interop tests (requires Python RNS fixtures in tests/interop/)
cargo test -p styrene-rns --features interop-tests

# All features
cargo test -p styrene-rns --all-features
```

## Gotchas

- Library name is `rns_core`, not `styrene_rns`. Use `use rns_core::...` in code.
- `fernet.rs` line 334 has an `.unwrap()` on a `try_into()` for IV extraction. This is technically safe (slice length is checked) but violates the workspace lint `clippy::unwrap_used = "warn"`.
- The `transport` feature pulls in heavy dependencies: tokio, rusqlite, bzip2, tokio-serial. Only enable if you need the full transport stack.
- Module-level doc comments are missing on most modules. The crate-level docs are also minimal.
- UDP interface has a TODO noting that IFAC wrap/unwrap is not yet implemented for UDP.
- The `serde` module shadows the `serde` crate name -- use `::serde` or the re-export if you hit name resolution issues.

## Known Issues

- UDP IFAC support incomplete (see TODO in `transport/iface/udp.rs`)
- Missing module-level documentation across the crate
- The `.unwrap()` in fernet.rs should be replaced with proper error handling
