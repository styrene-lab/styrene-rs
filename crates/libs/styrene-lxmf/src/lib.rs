#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod constants;
mod error;

pub mod announce;
pub mod attachments;
pub mod errors;
pub mod identity;
pub mod inbound_decode;
pub mod message;
pub mod payload_fields;
#[cfg(feature = "std")]
pub mod propagation;
pub mod propagation_announce;
pub mod stamps;
#[cfg(feature = "std")]
pub mod wire_fields;

#[cfg(feature = "sdk")]
pub mod sdk;

pub use constants::{MIN_PROPAGATED_LXMF_BYTES, PAPER_MDU};
pub use error::LxmfError;
pub use message::{Message, Payload, WireMessage};
