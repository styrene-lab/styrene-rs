# AGENTS.md -- styrene-content

## What It Does

P2P content distribution protocol for the Styrene mesh. Splits files into BLAKE3-addressed chunks, tracks availability via bitsets, and distributes content over RNS/Yggdrasil transports.

Designed with a three-zone `no_std` architecture so the same types compile on bare-metal (RP2040), embedded async runtimes (embassy), and full tokio.

## Current Status

**Design-complete but not wired into the daemon.** The entire crate is behind `#![allow(dead_code)]`. It compiles, tests pass, but nothing in `styrened` imports it yet. Treat it as a library waiting for integration.

Recent history: had compile errors that were fixed. Cfg-conditional warnings may still appear depending on feature combination.

## Module Map

```
src/
  lib.rs              # Crate root, re-exports, zone documentation
  content_id.rs       # BLAKE3-based content identifier
  manifest.rs         # StyreneManifest -- file metadata + chunk list
  chunk_bitset.rs     # Fixed-size bitset tracking which chunks a peer has
  chunk_profile.rs    # Per-peer chunk availability profile
  announce.rs         # ResourceAvailableAnnounce -- mesh announce for content
  store.rs            # ChunkStore trait (async, no_std compatible via AFIT)
  transport.rs        # ContentTransport trait + ContentEvent enum
  distributor.rs      # ContentDistributor state machine (orchestrates transfers)
  error.rs            # DistributorError, ManifestError
  impls/
    mod.rs            # Feature-gated impl selection
    ram.rs            # RamChunkStore (requires `alloc`)
    tokio_fs.rs       # TokioFsChunkStore (requires `tokio`)
    flash.rs          # FlashChunkStore (requires `embedded-storage`)
```

## Key Types and Traits

- `ContentId` -- BLAKE3 hash identifying a piece of content
- `StyreneManifest` -- metadata envelope: content ID, chunk count, size, CBOR-serialized
- `ChunkBitset` -- heapless fixed-size bitset for chunk availability
- `ChunkStore` (trait) -- async chunk read/write, AFIT (no boxing)
- `ContentTransport` (trait) -- async send/receive of chunks over mesh
- `ContentDistributor` -- state machine driving chunk exchange

## Feature Flags

| Feature | What it enables |
|---------|----------------|
| *(default)* | Zone 0 + Zone 1 only -- no alloc, no std |
| `alloc` | `RamChunkStore`, dynamic collections |
| `std` | Filesystem access (implies alloc) |
| `tokio` | `TokioFsChunkStore` (implies std) |
| `embedded-storage` | `FlashChunkStore` for RP2040/ESP32 |

## Test Commands

```bash
# Unit tests (default features, no_std zone 0+1)
cargo test -p styrene-content

# Distributor integration test (requires alloc)
cargo test -p styrene-content --test distributor --features alloc

# Manifest roundtrip test
cargo test -p styrene-content --test manifest_roundtrip

# no_std compile check
cargo test -p styrene-content --test no_std_compile
```

## Gotchas

- Uses `#![no_std]` at crate level. If you add a dependency, it must be `no_std`-compatible or feature-gated behind `std`/`alloc`.
- Uses async-fn-in-trait (AFIT) directly -- no `#[async_trait]`, no boxing. Requires Rust 1.75+.
- Serialization is CBOR via `ciborium`, not MessagePack. This crate was written after the CBOR migration decision.
- `heapless` collections have fixed capacity. Exceeding capacity is a runtime error, not a resize.
- The `#![allow(dead_code)]` suppresses warnings crate-wide -- do not remove until it is integrated into `styrened`.

## Known Issues

- Not integrated into any binary yet. All code is effectively unused.
- Cfg-conditional warnings may surface depending on which features are active.
- No benchmarks.
