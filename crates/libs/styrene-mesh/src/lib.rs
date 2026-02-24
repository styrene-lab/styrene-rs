//! # styrene-mesh
//!
//! Styrene wire protocol envelope format for mesh communications.
//!
//! This crate implements the binary wire format used by the Styrene mesh
//! communications platform. It is the shared contract between the Python
//! (`styrened`) and Rust (`styrened-rs`) implementations — both must produce
//! and consume identical byte sequences.
//!
//! ## Wire Format v2
//!
//! ```text
//! [namespace:10][version:1][type:1][request_id:16][payload:variable]
//!  "styrene.io"   0x01      enum    random bytes    msgpack-encoded
//! ```
//!
//! ## Example
//!
//! ```rust
//! use styrene_mesh::{StyreneMessage, StyreneMessageType};
//!
//! let msg = StyreneMessage::new(
//!     StyreneMessageType::Ping,
//!     &[],
//! );
//! let encoded = msg.encode();
//! let decoded = StyreneMessage::decode(&encoded).unwrap();
//! assert_eq!(decoded.message_type, StyreneMessageType::Ping);
//! ```
//!
//! ## Crate Family
//!
//! This crate is part of the [styrene-rs](https://github.com/styrene-lab/styrene-rs)
//! workspace:
//!
//! - [`styrene-rns`](https://crates.io/crates/styrene-rns) — RNS protocol core
//! - [`styrene-lxmf`](https://crates.io/crates/styrene-lxmf) — LXMF messaging
//! - [`styrene-rns-transport`](https://crates.io/crates/styrene-rns-transport) — Transport interfaces
//! - **`styrene-mesh`** (this crate) — Wire protocol envelope
//! - [`styrene`](https://crates.io/crates/styrene) — Meta-crate re-exporting all

pub mod wire;

pub use wire::{StyreneMessage, StyreneMessageType, WireError};

/// Wire format namespace prefix.
pub const NAMESPACE: &[u8; 10] = b"styrene.io";

/// Current wire format version.
pub const WIRE_VERSION: u8 = 0x01;
