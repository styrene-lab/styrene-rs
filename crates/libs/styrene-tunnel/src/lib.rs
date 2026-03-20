//! PQC tunnel infrastructure for styrene mesh communications.
//!
//! This crate provides:
//!
//! - **PQC crypto** — ML-KEM-768 + X25519 hybrid key exchange, AES-256-GCM
//!   session encryption, key ratcheting (always available)
//! - **Session state machine** — 3-message PQC handshake protocol matching
//!   Python `styrened`'s implementation (always available)
//! - **Tunnel backends** — strongSwan VICI (feature `strongswan`) and
//!   WireGuard netlink (feature `wireguard`) behind [`TunnelBackend`] trait
//! - **Orchestrator** — Tunnel selection, failover, health monitoring
//!   (requires at least one backend feature)
//!
//! # Feature flags
//!
//! - `strongswan` — Enables the strongSwan VICI backend for IPsec + ML-KEM tunnels
//! - `wireguard` — Enables the WireGuard netlink backend for classical tunnels
//! - `tunnel` — Enables both backends + orchestrator

pub mod crypto;
pub mod error;
pub mod session;
pub mod traits;

#[cfg(feature = "strongswan")]
pub mod strongswan;

#[cfg(feature = "wireguard")]
pub mod wireguard;

#[cfg(any(feature = "strongswan", feature = "wireguard"))]
pub mod orchestrator;

pub use error::TunnelError;
pub use traits::TunnelBackend;
