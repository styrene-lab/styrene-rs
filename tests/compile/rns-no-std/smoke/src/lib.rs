#![no_std]
#![forbid(unsafe_code)]

//! Finite synthetic API exercise. Seeds, keys and packets must never be used live.

use rand_chacha::ChaCha20Rng;
use rand_core::{CryptoRng, RngCore, SeedableRng};
use rns_core::RnsError;
use rns_core::crypt::fernet::{CachedFernet, Fernet, PlainText};
use rns_core::destination::{
    DestinationAnnounce, DestinationName, SingleInputDestination, group_decrypt,
    group_encrypt_with_rng,
};
use rns_core::identity::{
    DecryptIdentity, DerivedKey, EmptyIdentity, EncryptIdentity, PrivateIdentity,
};
use rns_core::packet::Packet;
use rns_core::ratchets::encrypt_for_public_key;
use rns_core::time_source::{advance_unix_time, clear_unix_time, set_unix_time, unix_now};

// Deliberately neither Copy nor Clone: each portable entry point must borrow
// this one advancing state. ChaCha is seeded only for reproducible fixture data.
struct SyntheticRng {
    inner: ChaCha20Rng,
    bytes: usize,
}

impl CryptoRng for SyntheticRng {}

impl RngCore for SyntheticRng {
    fn next_u32(&mut self) -> u32 {
        self.bytes += 4;
        self.inner.next_u32()
    }

    fn next_u64(&mut self) -> u64 {
        self.bytes += 8;
        self.inner.next_u64()
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        self.bytes += dest.len();
        self.inner.fill_bytes(dest);
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}

/// Runs once, on one CPU with no concurrent clock updater.
pub fn run() {
    let mut rng = SyntheticRng { inner: ChaCha20Rng::from_seed([0x78; 32]), bytes: 0 };
    let identity = PrivateIdentity::new_from_rand(&mut rng);
    let after_keygen = rng.bytes;
    assert!(after_keygen >= 64);
    let other = PrivateIdentity::new_from_rand(&mut rng);
    assert!(rng.bytes > after_keygen);
    assert_ne!(identity.address_hash(), other.address_hash());
    let text = b"synthetic no-std fixture";
    let signature = identity.sign(text);
    assert!(identity.verify(text, &signature).is_ok());
    assert!(identity.verify(b"tampered", &signature).is_err());

    let public = identity.as_identity();
    let key = public.derive_key(&mut rng, None);
    let next_key = public.derive_key(&mut rng, None);
    assert_ne!(key.ephemeral_public_bytes(), next_key.ephemeral_public_bytes());
    let mut ciphertext = [0u8; 256];
    let mut plaintext = [0u8; 256];
    let encrypted = identity.encrypt_with_rng(&mut rng, text, &key, &mut ciphertext).unwrap();
    let before_decrypt = rng.bytes;
    assert_eq!(identity.decrypt_with_derived_key(encrypted, &key, &mut plaintext).unwrap(), text);
    assert_eq!(rng.bytes, before_decrypt);

    // Preserve the public-identity API's existing framing and key slicing.
    // Its first 16 bytes form the signing key (also in the AES-256 profile).
    let framed = public.encrypt_with_rng(&mut rng, text, &key, &mut ciphertext).unwrap();
    assert_eq!(&framed[..32], key.ephemeral_public_bytes().unwrap());
    let cached = CachedFernet::new_from_slices(&key.as_bytes()[..16], &key.as_bytes()[16..]);
    let verified = cached.verify((&framed[32..]).into()).unwrap();
    assert_eq!(cached.decrypt(verified, &mut plaintext).unwrap().as_slice(), text);
    let token =
        cached.encrypt(&mut rng, PlainText::from(text.as_slice()), &mut ciphertext).unwrap();
    assert_eq!(
        cached.decrypt(cached.verify(token).unwrap(), &mut plaintext).unwrap().as_slice(),
        text
    );

    let empty = EmptyIdentity;
    let copied = empty.encrypt_with_rng(&mut rng, text, &key, &mut ciphertext).unwrap();
    assert_eq!(empty.decrypt_with_derived_key(copied, &key, &mut plaintext).unwrap(), text);
    let group_key = [0x42; 16];
    let first = group_encrypt_with_rng(&group_key, text, &mut rng).unwrap();
    let second = group_encrypt_with_rng(&group_key, text, &mut rng).unwrap();
    assert_ne!(first, second);
    let before_decrypt = rng.bytes;
    assert_eq!(group_decrypt(&group_key, &first).unwrap(), text);
    let mut tampered = first;
    tampered[16] ^= 1;
    assert!(group_decrypt(&group_key, &tampered).is_err());
    assert_eq!(rng.bytes, before_decrypt);

    clear_unix_time();
    assert_eq!(unix_now(), Err(RnsError::TimeUnavailable));
    assert_eq!(advance_unix_time(1), None);
    let mut destination =
        SingleInputDestination::new(identity, DestinationName::new("synthetic", "no-std"));
    assert_eq!(destination.announce(&mut rng, None).map(|_| ()), Err(RnsError::TimeUnavailable));
    set_unix_time(1_800_000_000);
    destination.enable_ratchets_in_memory();
    destination.set_retained_ratchets(2).unwrap();
    destination.set_ratchet_interval_secs(10).unwrap();
    let announce = destination.announce(&mut rng, Some(text)).unwrap();
    let encoded = announce.to_bytes().unwrap();
    let decoded = Packet::from_bytes(&encoded).unwrap();
    assert_eq!(decoded.to_bytes().unwrap(), encoded);
    let info = DestinationAnnounce::validate(&decoded).unwrap();
    assert_eq!(info.app_data, text);
    assert_eq!(&info.random_blob[5..], &1_800_000_000u64.to_be_bytes()[3..]);
    let ratchet = info.ratchet.unwrap();
    let encrypted = encrypt_for_public_key(
        &ratchet.into(),
        destination.identity.address_hash().as_slice(),
        text,
        &mut rng,
    )
    .unwrap();
    let before_decrypt = rng.bytes;
    let (decrypted, used_ratchet) = destination.decrypt_with_ratchets(&encrypted).unwrap();
    assert!(used_ratchet);
    assert_eq!(decrypted, text);
    assert_eq!(rng.bytes, before_decrypt);
    assert_eq!(advance_unix_time(4), Some(1_800_000_004));
    let same = destination.announce(&mut rng, None).unwrap();
    assert_eq!(DestinationAnnounce::validate(&same).unwrap().ratchet, Some(ratchet));
    assert_eq!(advance_unix_time(7), Some(1_800_000_011));
    let before_rotate = rng.bytes;
    let rotated = destination.announce(&mut rng, None).unwrap();
    assert!(rng.bytes >= before_rotate + 37, "announce and rotation consume caller state");
    assert_ne!(DestinationAnnounce::validate(&rotated).unwrap().ratchet, Some(ratchet));
    assert_eq!(destination.decrypt_with_ratchets(&encrypted).unwrap().0, text);
    destination.path_response_with_tag(&mut rng, None, Some(&[1; 16])).unwrap();
    clear_unix_time();
    assert_eq!(destination.announce(&mut rng, None).map(|_| ()), Err(RnsError::TimeUnavailable));
}

// External implementations retain the exact legacy trait surface. These also
// type-check on the real target, independently of RNS's dev-dependencies.
pub struct LegacyIdentity;

impl EncryptIdentity for LegacyIdentity {
    fn encrypt<'a, R: rand_core::CryptoRngCore + Copy>(
        &self,
        mut rng: R,
        text: &[u8],
        key: &DerivedKey,
        out: &'a mut [u8],
    ) -> Result<&'a [u8], RnsError> {
        EmptyIdentity.encrypt_with_rng(&mut rng, text, key, out)
    }
}

impl DecryptIdentity for LegacyIdentity {
    fn decrypt<'a, R: rand_core::CryptoRngCore + Copy>(
        &self,
        _rng: R,
        data: &[u8],
        key: &DerivedKey,
        out: &'a mut [u8],
    ) -> Result<&'a [u8], RnsError> {
        EmptyIdentity.decrypt_with_derived_key(data, key, out)
    }
}

pub fn legacy_immutable_fernet<R: rand_core::CryptoRngCore + Copy>(fernet: &Fernet<R>) {
    let mut out = [0u8; 128];
    assert!(fernet.encrypt(PlainText::from(b"legacy".as_slice()), &mut out).is_ok());
}
