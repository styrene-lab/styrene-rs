//! Application state — bridges protocol crate types into UI-friendly data.

use rns_core::identity::PrivateIdentity;

/// UI-friendly identity summary (no raw crypto in the view layer).
#[derive(Clone, Debug, PartialEq)]
pub struct IdentityInfo {
    pub hash_hex: String,
    pub public_key_hex: String,
    pub signing_key_hex: String,
}

/// Generate a fresh RNS identity and extract display info.
///
/// This proves styrene-rns crypto compiles and runs in the Dioxus context.
pub fn generate_identity_info() -> IdentityInfo {
    let mut rng = rand_core::OsRng;
    let identity = PrivateIdentity::new_from_rand(&mut rng);
    let public = identity.as_identity();

    IdentityInfo {
        hash_hex: hex::encode(public.address_hash.as_slice()),
        public_key_hex: hex::encode(public.public_key_bytes()),
        signing_key_hex: hex::encode(public.verifying_key_bytes()),
    }
}
