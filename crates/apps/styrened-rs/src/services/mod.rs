//! Daemon services — decomposed from the RpcDaemon god struct.
//!
//! Each service owns a bounded domain of behavior. Services are constructed
//! by `AppContext` and accessed through its accessor methods. Services
//! communicate through `AppContext` accessors, never through direct circular
//! references.
//!
//! Service graph:
//! - Package E: identity, config, status, fleet, auth, auto_reply
//! - Package F: messaging, discovery (+ storage/node_store)
//! - Package G: protocol
//! - Package H: events, tunnel

pub mod auth;
pub mod auto_reply;
pub mod config;
pub mod discovery;
pub mod events;
pub mod fleet;
pub mod identity;
pub mod messaging;
pub mod protocol;
pub mod status;
pub mod tunnel;

// Re-exports for convenience
pub use auth::{AuthService, Capability, Role};
pub use auto_reply::{AutoReplyConfig, AutoReplyMode, AutoReplyService};
pub use config::ConfigService;
pub use discovery::DiscoveryService;
pub use events::EventService;
pub use fleet::FleetService;
pub use identity::IdentityService;
pub use messaging::MessagingService;
pub use protocol::ProtocolService;
pub use status::{InterfaceRecord, PropagationState, StatusService};
pub use tunnel::TunnelService;
