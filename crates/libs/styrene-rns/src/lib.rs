#![cfg_attr(not(feature = "std"), no_std)]

//! RNS protocol core with caller-supplied cryptographic randomness.
//!
//! Pass `&mut rng` to portable encryption, key-generation, and announce APIs to
//! advance one provider state. The provider must be initialized and healthy.
//! Infallible RNG requests may panic on entropy loss, even inside an operation
//! that returns `Result`; these APIs do not promise typed entropy recovery.
//! Legacy `Copy`-bounded APIs require handles to advancing providers, not copied
//! PRNG state snapshots. Hosted OS-backed conveniences require `std`.

extern crate alloc;

pub mod buffer;
pub mod crypt;
pub mod destination;
pub mod destination_hash;
mod error;
pub mod hash;
pub mod identity;
pub mod key_manager;
pub mod packet;
pub mod ratchets;
pub mod time_source;

pub mod serde;

#[cfg(feature = "std")]
pub mod diagnostics;
#[cfg(feature = "std")]
pub mod private_file;

#[cfg(feature = "transport")]
pub mod transport;

#[cfg(feature = "std")]
pub use destination::group_encrypt;
pub use destination::{group_decrypt, group_encrypt_with_rng};
pub use error::RnsError;
pub use hash::lxmf_address_hash;
pub use identity::{lxmf_sign, lxmf_verify};
pub use packet::{LXMF_MAX_PAYLOAD, Packet};
