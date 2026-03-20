//! Security Association lifecycle management.
//!
//! Tracks the state of IPsec SAs managed by this daemon and maps them
//! to the `TunnelInfo` abstraction.
//!
//! # Future implementation
//!
//! - SA state tracking from VICI events
//! - DPD (Dead Peer Detection) monitoring
//! - Rekey scheduling integration with PQC session ratcheting
//! - Credential bridge: derive IKEv2 PSK from PQC shared secret
