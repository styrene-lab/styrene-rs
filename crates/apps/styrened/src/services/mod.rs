//! Daemon services — decomposed from the RpcDaemon god struct.
//!
//! Each service owns a bounded domain of behavior. Services are constructed
//! by `AppContext` and accessed through its accessor methods. Services
//! communicate through `AppContext` accessors, never through direct circular
//! references.
//!
//! Service graph:
//! - Package E: identity, config, status, fleet, policy (RBAC), auto_reply
//! - Package F: messaging, discovery (+ shared MessagesStore)
//! - Package G: protocol
//! - Package H: events, tunnel

pub mod auto_reply;
pub mod config;
pub mod discovery;
pub mod events;
pub mod fleet;
#[cfg(feature = "i2p-proxy")]
pub mod i2p_proxy;
pub mod identity;
pub mod messaging;
pub mod pages;
pub mod propagation;
pub mod policy;
pub mod protocol;
pub mod status;
pub mod tunnel;

// Re-exports for convenience
pub use auto_reply::{AutoReplyConfig, AutoReplyMode, AutoReplyService};
pub use policy::PolicyService;
pub use config::ConfigService;
pub use discovery::DiscoveryService;
pub use events::EventService;
pub use fleet::FleetService;
pub use identity::IdentityService;
pub use messaging::MessagingService;
pub use pages::PageService;
pub use propagation::PropagationService;
pub use protocol::ProtocolService;
pub use status::{InterfaceRecord, PropagationState, StatusService};
#[cfg(feature = "i2p-proxy")]
pub use i2p_proxy::I2pProxyService;
pub use tunnel::TunnelService;
