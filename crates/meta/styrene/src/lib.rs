//! # styrene
//!
//! Meta-crate re-exporting all Styrene library crates for convenient
//! dependency declaration.
//!
//! Instead of depending on each crate individually:
//!
//! ```toml
//! [dependencies]
//! styrene-rns = "0.1"
//! styrene-lxmf = "0.1"
//! styrene-mesh = "0.1"
//! styrene-rns-transport = "0.1"
//! ```
//!
//! You can depend on the meta-crate:
//!
//! ```toml
//! [dependencies]
//! styrene = "0.1"
//! ```
//!
//! ## Crate Family
//!
//! - [`styrene-rns`](https://crates.io/crates/styrene-rns) — RNS protocol core
//! - [`styrene-lxmf`](https://crates.io/crates/styrene-lxmf) — LXMF messaging
//! - [`styrene-rns-transport`](https://crates.io/crates/styrene-rns-transport) — Transport interfaces
//! - [`styrene-mesh`](https://crates.io/crates/styrene-mesh) — Wire protocol envelope
//!
//! Source: <https://github.com/styrene-lab/styrene-rs>

/// RNS protocol core — identity, destinations, links, resources, ratchets.
pub use rns_core as rns;

/// RNS transport interfaces — TCP, UDP, future Serial/KISS.
pub use rns_transport as transport;

/// LXMF messaging — router, propagation, stamps, delivery pipeline.
pub use lxmf_core as lxmf;

/// Styrene wire protocol envelope format.
pub use styrene_mesh as mesh;
