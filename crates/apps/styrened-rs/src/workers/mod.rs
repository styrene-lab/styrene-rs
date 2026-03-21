//! Background worker tasks for the daemon.
//!
//! Workers are spawned tokio tasks that bridge transport events to the
//! service layer. They subscribe to transport broadcast channels and
//! feed decoded data into services.

pub mod inbound;
pub mod announce;
