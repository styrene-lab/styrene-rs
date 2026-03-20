//! PQC cryptographic primitives.
//!
//! Hybrid key exchange combining ML-KEM-768 (FIPS 203) with X25519,
//! AES-256-GCM session encryption, and key ratcheting.

pub(crate) mod aead;
pub(crate) mod kdf;
pub(crate) mod kem;

pub use aead::{SessionCipher, CONFIRM_ROLE_INITIATOR, CONFIRM_ROLE_RESPONDER};
pub use kdf::HybridKdf;
pub use kem::{MlKemEncapsulated, MlKemKeyPair};
