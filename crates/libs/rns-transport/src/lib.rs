//! Transport boundary APIs for runtime crates and daemon entrypoints.

use core::fmt;

pub mod delivery;
pub mod iface;
pub mod receipt;
pub mod resource;
pub mod transport;

pub use transport::{DeliveryReceipt, ReceiptHandler, Transport, TransportConfig};

pub mod storage {
    pub mod messages {
        /// Placeholder retained for API parity during phased migration.
        #[derive(Clone, Debug, Default)]
        pub struct MessagesStore;
    }
}
