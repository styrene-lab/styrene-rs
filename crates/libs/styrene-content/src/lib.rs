//! Styrene mesh content distribution — P2P file sharing over RNS/Yggdrasil.
//!
//! # Three-zone architecture
//!
//! **Zone 0 — Pure types** (`no_std`, no `alloc`, no `async`)
//! All types compile on RP2040 bare-metal with no heap. Uses `heapless` for
//! fixed-size collections.
//!
//! **Zone 1 — Async traits** (`no_std`, no `alloc`, AFIT)
//! `ChunkStore` and `ContentTransport` traits use async-fn-in-trait (stable
//! since Rust 1.75). No boxing — works with embassy, FreeRTOS, and tokio.
//!
//! **Zone 2 — Implementations** (feature-gated)
//! `RamChunkStore` (alloc), `TokioFsChunkStore` (tokio), `FlashChunkStore`
//! (embedded-storage). Only compiled when the corresponding feature is active.
//!
//! # Feature flags
//!
//! | Feature | Enables |
//! |---------|---------|
//! | `default` | Zone 0 + Zone 1 only — no alloc, no std |
//! | `alloc` | `RamChunkStore`, dynamic collections |
//! | `std` | filesystem access (implies alloc) |
//! | `tokio` | `TokioFsChunkStore` (implies std) |
//! | `embedded-storage` | `FlashChunkStore` for RP2040/ESP32 |

#![no_std]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

// Zone 0 — pure types
pub mod announce;
pub mod chunk_bitset;
pub mod chunk_profile;
pub mod content_id;
pub mod manifest;

// Zone 1 — async traits + state machine
pub mod distributor;
pub mod error;
pub mod store;
pub mod transport;

// Zone 2 — feature-gated implementations
pub mod impls;

// Flat re-exports for convenience
pub use announce::ResourceAvailableAnnounce;
pub use chunk_bitset::ChunkBitset;
pub use chunk_profile::ChunkProfile;
pub use content_id::ContentId;
pub use distributor::ContentDistributor;
pub use error::{DistributorError, ManifestError};
pub use manifest::StyreneManifest;
pub use store::ChunkStore;
pub use transport::{ContentEvent, ContentTransport};
