//! VICI protocol client for communicating with strongSwan charon.
//!
//! The VICI protocol is a message-based protocol over Unix sockets.
//! See: https://docs.strongswan.org/docs/5.9/plugins/vici.html
//!
//! # Future implementation
//!
//! This module will use the `rustici` crate for VICI protocol handling.
//! Key operations:
//!
//! - `load-shared` — Inject PSK derived from PQC session shared secret
//! - `load-conn` — Configure IKEv2 connection with hybrid proposal:
//!   `x25519-ke1_mlkem768-aes256gcm16-sha384`
//! - `initiate` — Start SA negotiation
//! - `terminate` — Tear down SA
//! - `list-sas` — Query active SAs
//! - Event subscription: `ike-updown`, `child-updown` for state tracking
