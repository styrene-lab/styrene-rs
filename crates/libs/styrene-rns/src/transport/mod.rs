//! Transport layer — TCP, UDP, and future Serial/KISS transports.
//!
//! This module is gated behind the `transport` feature.

pub mod channel;
pub mod config;
pub mod core_transport;
pub mod delivery;
pub mod destination_ext;
pub mod embedded_link;
pub mod error;
pub mod iface;
pub mod identity_bridge;
pub mod identity_ext;
pub(crate) mod ratchet_store;
pub mod receipt;
pub mod resource;
pub mod storage;
pub mod time;
pub mod utils;
