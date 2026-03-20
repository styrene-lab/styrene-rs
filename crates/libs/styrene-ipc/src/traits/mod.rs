mod events;
mod fleet;
mod identity;
mod messaging;
mod status;
mod tunnel;

pub use events::DaemonEvents;
pub use fleet::DaemonFleet;
pub use identity::DaemonIdentity;
pub use messaging::DaemonMessaging;
pub use status::DaemonStatus;
pub use tunnel::DaemonTunnel;

/// Composite trait encompassing all daemon IPC capabilities.
///
/// Automatically implemented for any type that implements all six
/// sub-traits. Use `Arc<dyn Daemon>` as the primary handle type.
pub trait Daemon:
    DaemonMessaging + DaemonIdentity + DaemonStatus + DaemonFleet + DaemonEvents + DaemonTunnel
{
}

impl<T> Daemon for T where
    T: DaemonMessaging + DaemonIdentity + DaemonStatus + DaemonFleet + DaemonEvents + DaemonTunnel
{
}
