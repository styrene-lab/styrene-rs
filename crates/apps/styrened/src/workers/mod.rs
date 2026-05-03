//! Background worker tasks for the daemon.
//!
//! Workers are spawned tokio tasks that bridge transport events to the
//! service layer. They subscribe to transport broadcast channels and
//! feed decoded data into services.

pub mod announce;
pub mod inbound;
pub mod link;
pub mod page_handler;
pub mod propagation_handler;
pub mod rpc_request;
pub mod rpc_response;
