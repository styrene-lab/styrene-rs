mod events;
mod fleet;
mod identity;
mod messaging;
mod status;

pub use events::DaemonEvents;
pub use fleet::DaemonFleet;
pub use identity::DaemonIdentity;
pub use messaging::DaemonMessaging;
pub use status::DaemonStatus;

/// Composite trait encompassing all daemon IPC capabilities.
///
/// Automatically implemented for any type that implements all five
/// sub-traits. Use `Arc<dyn Daemon>` as the primary handle type.
pub trait Daemon: DaemonMessaging + DaemonIdentity + DaemonStatus + DaemonFleet + DaemonEvents {}

impl<T> Daemon for T where T: DaemonMessaging + DaemonIdentity + DaemonStatus + DaemonFleet + DaemonEvents {}
