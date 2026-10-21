//! Borrowed-provider and legacy API regression tests. All keys are synthetic.

#[path = "support/rng.rs"]
mod rng;

use ed25519_dalek::SigningKey;
use rand_core::{CryptoRng, CryptoRngCore, RngCore};
use rng::RecordingRng;
use rns_core::RnsError;
use rns_core::crypt::fernet::{CachedFernet, Fernet, Token};
use rns_core::destination::{DestinationAnnounce, DestinationName, SingleInputDestination};
use rns_core::hash::AddressHash;
use rns_core::identity::{
    DERIVED_KEY_LENGTH, DecryptIdentity, DerivedKey, EmptyIdentity, EncryptIdentity,
    PrivateIdentity,
};
use rns_core::ratchets::{decrypt_with_identity, decrypt_with_private_key, encrypt_for_public_key};
use rns_core::{group_decrypt, group_encrypt_with_rng};
use std::cell::Cell;
use x25519_dalek::{PublicKey, StaticSecret};

fn secret(bytes: &[u8]) -> StaticSecret {
    StaticSecret::from(<[u8; 32]>::try_from(bytes).expect("32-byte RNG request"))
}

#[test]
fn identity_and_ephemeral_generation_advance_one_noncopy_provider() {
    let mut rng = RecordingRng::default();
    let first = PrivateIdentity::new_from_rand(&mut rng);
    let second = PrivateIdentity::new_from_rand(&mut rng);
    assert_eq!(rng.lengths(), [32, 32, 32, 32]);
    assert_ne!(first.to_private_key_bytes(), second.to_private_key_bytes());
    let expected = PrivateIdentity::new(
        secret(&rng.requests[1]),
        SigningKey::from_bytes(rng.requests[0].as_slice().try_into().expect("signing seed")),
    );
    assert_eq!(first.to_private_key_bytes(), expected.to_private_key_bytes());

    let identity = first.as_identity();
    let first_key = identity.derive_key(&mut rng, Some(b"salt"));
    let second_key =
        DerivedKey::new_from_ephemeral_key(&mut rng, &identity.public_key, Some(b"salt"));
    assert_eq!(rng.lengths(), [32, 32, 32, 32, 32, 32]);
    for (key, request) in [(&first_key, &rng.requests[4]), (&second_key, &rng.requests[5])] {
        let public = PublicKey::from(&secret(request));
        assert_eq!(key.ephemeral_public_bytes(), Some(public.as_bytes()));
        assert_eq!(key.as_bytes(), first.derive_key(&public, Some(b"salt")).as_bytes());
    }
    assert_ne!(first_key.ephemeral_public_bytes(), second_key.ephemeral_public_bytes());
    assert_ne!(first_key.as_bytes(), second_key.as_bytes());

    let first_hash = AddressHash::new_from_rand(&mut rng);
    let second_hash = AddressHash::new_from_rand(&mut rng);
    assert_ne!(first_hash, second_hash);
}

#[test]
fn identity_encryption_preserves_framing_and_the_existing_key_splits() {
    let identity = PrivateIdentity::new_from_name("portable RNG test");
    let mut rng = RecordingRng::default();
    let key = identity.as_identity().derive_key(&mut rng, None);
    let mut buffer = [0; 256];
    let first = identity
        .as_identity()
        .encrypt_with_rng(&mut rng, b"public", &key, &mut buffer)
        .expect("public encryption")
        .to_vec();
    // Exercise ?Sized on the additive APIs as well as concrete non-Copy state.
    let second = identity
        .as_identity()
        .encrypt_with_rng(&mut rng as &mut dyn CryptoRngCore, b"public", &key, &mut buffer)
        .expect("second public encryption")
        .to_vec();
    assert_eq!(rng.lengths(), [32, 16, 16]);
    assert_eq!(&first[..32], key.ephemeral_public_bytes().expect("ephemeral public"));
    assert_eq!(&first[32..48], rng.requests[1]);
    assert_eq!(&second[32..48], rng.requests[2]);
    assert_ne!(first, second);
    // Public Identity historically splits at 16 even under AES-256. Preserve it.
    let public_fernet = CachedFernet::new_from_slices(&key.as_bytes()[..16], &key.as_bytes()[16..]);
    let verified = public_fernet.verify(Token::from(&first[32..])).expect("public split tag");
    assert_eq!(
        public_fernet.decrypt(verified, &mut buffer).expect("public plaintext").as_bytes(),
        b"public"
    );

    let first = identity
        .encrypt_with_rng(&mut rng, b"private", &key, &mut buffer)
        .expect("private encryption")
        .to_vec();
    let second = identity
        .encrypt_with_rng(&mut rng as &mut dyn CryptoRngCore, b"private", &key, &mut buffer)
        .expect("second private encryption")
        .to_vec();
    assert_eq!(rng.lengths(), [32, 16, 16, 16, 16]);
    assert_eq!(&first[..16], rng.requests[3]);
    assert_eq!(&second[..16], rng.requests[4]);
    assert_ne!(first, second);
    let half = DERIVED_KEY_LENGTH / 2;
    let private_fernet =
        CachedFernet::new_from_slices(&key.as_bytes()[..half], &key.as_bytes()[half..]);
    let verified = private_fernet.verify(Token::from(first.as_slice())).expect("private split tag");
    assert_eq!(
        private_fernet.decrypt(verified, &mut buffer).expect("private plaintext").as_bytes(),
        b"private"
    );
    assert_eq!(
        identity.decrypt_with_derived_key(&first, &key, &mut buffer),
        Ok(b"private".as_slice())
    );
}

#[test]
fn cached_and_group_encryption_advance_borrowed_state() {
    let mut rng = RecordingRng::default();
    let fernet = CachedFernet::new_from_slices(&[42; 32], &[43; 32]);
    let mut buffer = [0; 128];
    let first =
        fernet.encrypt(&mut rng, "cached".into(), &mut buffer).expect("token").as_bytes().to_vec();
    let second =
        fernet.encrypt(&mut rng, "cached".into(), &mut buffer).expect("token").as_bytes().to_vec();
    assert_ne!(first, second);
    assert_eq!(&first[..16], rng.requests[0]);
    assert_eq!(&second[..16], rng.requests[1]);
    let first = group_encrypt_with_rng(&[42; 16], b"group", &mut rng).expect("group token");
    let second = rns_core::destination::group_encrypt_with_rng(
        &[42; 16],
        b"group",
        &mut rng as &mut dyn CryptoRngCore,
    )
    .expect("second group token");
    assert_ne!(first, second);
    assert_eq!(&first[..16], rng.requests[2]);
    assert_eq!(&second[..16], rng.requests[3]);
    assert_eq!(rng.lengths(), [16, 16, 16, 16]);
    assert_eq!(group_decrypt(&[42; 16], &first), Ok(b"group".to_vec()));
    assert_eq!(rng.lengths(), [16, 16, 16, 16]);
}

#[cfg(feature = "std")]
#[test]
fn hosted_group_wrappers_remain_available_at_both_exports() {
    let key = [42; 16];
    let first = rns_core::group_encrypt(&key, b"hosted").expect("root export");
    let second = rns_core::destination::group_encrypt(&key, b"hosted").expect("destination export");
    assert_eq!(group_decrypt(&key, &first), Ok(b"hosted".to_vec()));
    assert_eq!(group_decrypt(&key, &second), Ok(b"hosted".to_vec()));
}

#[test]
fn ratchet_encryption_reborrows_for_ephemeral_key_then_iv() {
    let mut rng = RecordingRng::default();
    let identity = PrivateIdentity::new_from_name("ratchet provider test");
    let public = &identity.as_identity().public_key;
    let first = encrypt_for_public_key(public, b"salt", b"ratchet", &mut rng).expect("ciphertext");
    let second = encrypt_for_public_key(public, b"salt", b"ratchet", &mut rng).expect("ciphertext");
    assert_eq!(rng.lengths(), [32, 16, 32, 16]);
    for (ciphertext, index) in [(&first, 0), (&second, 2)] {
        assert_eq!(&ciphertext[..32], PublicKey::from(&secret(&rng.requests[index])).as_bytes());
        assert_eq!(&ciphertext[32..48], rng.requests[index + 1]);
        assert_eq!(decrypt_with_identity(&identity, b"salt", ciphertext), Ok(b"ratchet".to_vec()));
        let private = secret(&identity.to_private_key_bytes()[..32]);
        assert_eq!(
            decrypt_with_private_key(&private, b"salt", ciphertext),
            Ok(b"ratchet".to_vec())
        );
    }
    assert_ne!(&first[..32], &second[..32]);
    assert_ne!(&first[32..48], &second[32..48]);
    assert_eq!(rng.lengths(), [32, 16, 32, 16]);
}

#[test]
fn announce_and_path_responses_use_one_provider_and_cache_without_entropy() {
    #[cfg(not(feature = "std"))]
    rns_core::time_source::set_unix_time(1_800_000_000);
    let mut rng = RecordingRng::default();
    let mut destination = SingleInputDestination::new(
        PrivateIdentity::new_from_name("announce provider test"),
        DestinationName::new("test", "portable-rng"),
    );
    let first = destination.announce(&mut rng, None).expect("announce");
    let info = DestinationAnnounce::validate(&first).expect("signed announce");
    assert_eq!(&info.random_blob[..5], rng.requests[0]);
    assert!(info.ratchet.is_none());
    assert_eq!(rng.lengths(), [5]);
    destination.enable_ratchets_in_memory();
    let second = destination.path_response(&mut rng, None).expect("path response");
    let info = DestinationAnnounce::validate(&second).expect("signed response");
    assert_eq!(&info.random_blob[..5], rng.requests[1]);
    assert_eq!(info.ratchet, Some(*PublicKey::from(&secret(&rng.requests[2])).as_bytes()));
    assert_eq!(rng.lengths(), [5, 5, 32]);
    let third =
        destination.path_response_with_tag(&mut rng, None, Some(b"tag")).expect("tag response");
    let cached =
        destination.path_response_with_tag(&mut rng, None, Some(b"tag")).expect("cached response");
    assert_eq!(third.data.as_slice(), cached.data.as_slice());
    assert_eq!(rng.lengths(), [5, 5, 32, 5]);
    assert_ne!(
        &DestinationAnnounce::validate(&third).expect("tag signature").random_blob[..5],
        &info.random_blob[..5]
    );
}

/// Copyable provider handle, not a copied state snapshot. Test-only entropy.
#[derive(Clone, Copy)]
struct LegacyProvider<'a>(&'a Cell<u8>);

impl RngCore for LegacyProvider<'_> {
    fn next_u32(&mut self) -> u32 {
        panic!("unexpected word request")
    }
    fn next_u64(&mut self) -> u64 {
        panic!("unexpected word request")
    }
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        self.0.set(self.0.get() + 1);
        dest.fill(self.0.get());
    }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}
impl CryptoRng for LegacyProvider<'_> {}

// Integration tests are downstream crates: these implementations must continue
// to compile without new required methods or relaxed trait method Copy bounds.
struct ExternalIdentity;
impl EncryptIdentity for ExternalIdentity {
    fn encrypt<'a, R: CryptoRngCore + Copy>(
        &self,
        rng: R,
        text: &[u8],
        key: &DerivedKey,
        out: &'a mut [u8],
    ) -> Result<&'a [u8], RnsError> {
        EmptyIdentity.encrypt(rng, text, key, out)
    }
}
impl DecryptIdentity for ExternalIdentity {
    fn decrypt<'a, R: CryptoRngCore + Copy>(
        &self,
        rng: R,
        text: &[u8],
        key: &DerivedKey,
        out: &'a mut [u8],
    ) -> Result<&'a [u8], RnsError> {
        EmptyIdentity.decrypt(rng, text, key, out)
    }
}

#[test]
fn immutable_fernet_and_external_legacy_traits_remain_usable() {
    let count = Cell::new(0);
    let rng = LegacyProvider(&count);
    let fernet = Fernet::new_from_slices(&[1; 32], &[2; 32], rng);
    let mut buffer = [0; 128];
    let first = fernet
        .encrypt("legacy".into(), &mut buffer)
        .expect("immutable encryption")
        .as_bytes()
        .to_vec();
    let second = fernet
        .encrypt("legacy".into(), &mut buffer)
        .expect("immutable encryption")
        .as_bytes()
        .to_vec();
    assert_ne!(first, second);
    assert_eq!(count.get(), 2);
    let verified = fernet.verify(Token::from(first.as_slice())).expect("legacy tag");
    assert_eq!(
        fernet.decrypt(verified, &mut buffer).expect("legacy plaintext").as_bytes(),
        b"legacy"
    );
    let key = DerivedKey::new_empty();
    assert_eq!(ExternalIdentity.encrypt(rng, b"plain", &key, &mut buffer), Ok(b"plain".as_slice()));
    assert_eq!(ExternalIdentity.decrypt(rng, b"plain", &key, &mut buffer), Ok(b"plain".as_slice()));
    let private = PrivateIdentity::new_from_name("legacy encryptor");
    let key = private.as_identity().derive_key(rng, None);
    private
        .as_identity()
        .encrypt(rng, b"public", &key, &mut buffer)
        .expect("legacy public encryption");
    let token = private
        .encrypt(rng, b"private", &key, &mut buffer)
        .expect("legacy private encryption")
        .to_vec();
    let before = count.get();
    assert_eq!(private.decrypt(rng, &token, &key, &mut buffer), Ok(b"private".as_slice()));
    assert_eq!(count.get(), before, "legacy decrypt never requests entropy");
}

#[test]
fn empty_identity_and_failed_encryption_do_not_request_entropy() {
    let mut rng = RecordingRng::default();
    let mut buffer = [0xa5; 128];
    let key = DerivedKey::new_empty();
    assert_eq!(
        EmptyIdentity.encrypt_with_rng(
            &mut rng as &mut dyn CryptoRngCore,
            b"plain",
            &key,
            &mut buffer
        ),
        Ok(b"plain".as_slice())
    );
    assert_eq!(
        EmptyIdentity.decrypt_with_derived_key(b"plain", &key, &mut buffer),
        Ok(b"plain".as_slice())
    );
    assert_eq!(
        EmptyIdentity.encrypt_with_rng(&mut rng, b"plain", &key, &mut buffer[..1]),
        Err(RnsError::OutOfMemory)
    );
    assert_eq!(
        EmptyIdentity.decrypt_with_derived_key(b"plain", &key, &mut buffer[..1]),
        Err(RnsError::OutOfMemory)
    );
    let private = PrivateIdentity::new_from_name("buffer test");
    assert_eq!(
        private.as_identity().encrypt_with_rng(&mut rng, b"plain", &key, &mut buffer),
        Err(RnsError::InvalidArgument)
    );
    assert_eq!(
        private.encrypt_with_rng(&mut rng, b"plain", &key, &mut buffer[..1]),
        Err(RnsError::InvalidArgument)
    );
    assert!(rng.requests.is_empty());
    let key = private.as_identity().derive_key(&mut rng, None);
    assert_eq!(
        private.as_identity().encrypt_with_rng(&mut rng, b"plain", &key, &mut buffer[..33]),
        Err(RnsError::InvalidArgument)
    );
    // The public API historically writes the prefix before detecting a short
    // token buffer. Do not promise an untouched output on every failure.
    assert_eq!(&buffer[..32], key.ephemeral_public_bytes().expect("prefix"));
    assert_eq!(rng.lengths(), [32]);
}

struct FailedProvider;
impl RngCore for FailedProvider {
    fn next_u32(&mut self) -> u32 {
        panic!("entropy unavailable")
    }
    fn next_u64(&mut self) -> u64 {
        panic!("entropy unavailable")
    }
    fn fill_bytes(&mut self, _: &mut [u8]) {
        panic!("entropy unavailable")
    }
    fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), rand_core::Error> {
        panic!("entropy unavailable")
    }
}
impl CryptoRng for FailedProvider {}

#[test]
#[should_panic(expected = "entropy unavailable")]
fn result_returning_encryption_does_not_promise_entropy_recovery() {
    let _ = group_encrypt_with_rng(&[0; 16], b"test", &mut FailedProvider);
}
