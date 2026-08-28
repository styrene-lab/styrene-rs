//! Canonical Identity-issued bindings for epoch-indexed repository-signing keys.

use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use minicbor::{Decoder, Encoder};
use sha2::{Digest, Sha256};

use crate::IdentityId;

const PROFILE_VERSION: u16 = 1;
const PURPOSE: &str = "styrene-repository-signing-v1";
const SIGNING_DOMAIN: &[u8] = b"styrene-repository-signer-binding-v1";
const DIGEST_DOMAIN: &[u8] = b"styrene-repository-signer-binding-id-v1\0";
const MAX_PROTECTED_BYTES: usize = 256;
const MAX_BINDING_BYTES: usize = 384;

/// Canonical signed binding bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositorySignerBinding {
    protected: Vec<u8>,
    signature: [u8; 64],
}

impl RepositorySignerBinding {
    /// Issue a binding from caller-owned Identity seed bytes.
    pub fn issue_from_identity_seed(
        identity_seed: &[u8; 32],
        repository_public_key: [u8; 32],
        epoch: u32,
    ) -> Result<Self, RepositorySignerBindingError> {
        Self::issue(&SigningKey::from_bytes(identity_seed), repository_public_key, epoch)
    }

    /// Derive both software keys from a root and issue their binding at an explicit epoch.
    pub fn issue_derived(
        root_secret: &[u8; 32],
        epoch: u32,
    ) -> Result<Self, RepositorySignerBindingError> {
        let deriver = crate::derive::KeyDeriver::new(root_secret);
        let identity_key = SigningKey::from_bytes(&deriver.signing_seed());
        let repository_key = SigningKey::from_bytes(&deriver.derive_repository_signing_key(epoch));
        Self::issue(&identity_key, repository_key.verifying_key().to_bytes(), epoch)
    }

    /// Issue a profile-v1 binding with an Identity authority signing key.
    pub fn issue(
        identity_signing_key: &SigningKey,
        repository_public_key: [u8; 32],
        epoch: u32,
    ) -> Result<Self, RepositorySignerBindingError> {
        validate_public_key(&repository_public_key)?;
        let identity_public_key = identity_signing_key.verifying_key().to_bytes();
        let identity_id = IdentityId::from_public_key(&identity_public_key);
        let protected =
            encode_protected(identity_id, &identity_public_key, &repository_public_key, epoch)?;
        let signature = identity_signing_key.sign(&signing_frame(&protected)?).to_bytes();
        Ok(Self { protected, signature })
    }

    /// Return canonical protected claims bytes.
    pub fn protected_bytes(&self) -> &[u8] {
        &self.protected
    }

    /// Return the exact Ed25519 signature bytes.
    pub const fn signature(&self) -> &[u8; 64] {
        &self.signature
    }

    /// Return the exact profile signing frame.
    pub fn signing_frame(&self) -> Result<Vec<u8>, RepositorySignerBindingError> {
        signing_frame(&self.protected)
    }

    /// Encode the complete binding canonically.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, RepositorySignerBindingError> {
        encode_outer(&self.protected, &self.signature)
    }

    /// Compute the profile-v1 binding digest.
    pub fn digest(&self) -> Result<[u8; 32], RepositorySignerBindingError> {
        let bytes = self.canonical_bytes()?;
        let mut hasher = Sha256::new();
        hasher.update(DIGEST_DOMAIN);
        hasher.update(bytes);
        Ok(hasher.finalize().into())
    }
}

/// Verified attribution evidence. Consumer authorization and current-epoch policy are excluded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedRepositorySignerBinding {
    identity_id: IdentityId,
    identity_public_key: [u8; 32],
    repository_public_key: [u8; 32],
    epoch: u32,
    digest: [u8; 32],
}

impl VerifiedRepositorySignerBinding {
    pub const fn identity_id(&self) -> IdentityId {
        self.identity_id
    }

    pub const fn identity_public_key(&self) -> &[u8; 32] {
        &self.identity_public_key
    }

    pub const fn repository_public_key(&self) -> &[u8; 32] {
        &self.repository_public_key
    }

    pub const fn epoch(&self) -> u32 {
        self.epoch
    }

    pub const fn digest(&self) -> &[u8; 32] {
        &self.digest
    }
}

/// Strictly verify one bounded canonical profile-v1 binding.
pub fn verify_repository_signer_binding(
    bytes: &[u8],
) -> Result<VerifiedRepositorySignerBinding, RepositorySignerBindingError> {
    if bytes.len() > MAX_BINDING_BYTES {
        return Err(RepositorySignerBindingError::TooLarge);
    }
    let (protected, signature) = decode_outer(bytes)?;
    if protected.len() > MAX_PROTECTED_BYTES {
        return Err(RepositorySignerBindingError::TooLarge);
    }
    let claims = decode_protected(protected)?;
    validate_public_key(&claims.identity_public_key)?;
    validate_public_key(&claims.repository_public_key)?;
    if !claims.identity_id.matches_public_key(&claims.identity_public_key) {
        return Err(RepositorySignerBindingError::IdentityMismatch);
    }
    if encode_protected(
        claims.identity_id,
        &claims.identity_public_key,
        &claims.repository_public_key,
        claims.epoch,
    )? != protected
    {
        return Err(RepositorySignerBindingError::Canonical);
    }
    if encode_outer(protected, &signature)? != bytes {
        return Err(RepositorySignerBindingError::Canonical);
    }
    let verifying_key = VerifyingKey::from_bytes(&claims.identity_public_key)
        .map_err(|_| RepositorySignerBindingError::Signature)?;
    let signature = ed25519_dalek::Signature::from_bytes(&signature);
    verifying_key
        .verify_strict(&signing_frame(protected)?, &signature)
        .map_err(|_| RepositorySignerBindingError::Signature)?;

    let mut hasher = Sha256::new();
    hasher.update(DIGEST_DOMAIN);
    hasher.update(bytes);
    Ok(VerifiedRepositorySignerBinding {
        identity_id: claims.identity_id,
        identity_public_key: claims.identity_public_key,
        repository_public_key: claims.repository_public_key,
        epoch: claims.epoch,
        digest: hasher.finalize().into(),
    })
}

struct Claims {
    identity_id: IdentityId,
    identity_public_key: [u8; 32],
    repository_public_key: [u8; 32],
    epoch: u32,
}

fn encode_protected(
    identity_id: IdentityId,
    identity_public_key: &[u8; 32],
    repository_public_key: &[u8; 32],
    epoch: u32,
) -> Result<Vec<u8>, RepositorySignerBindingError> {
    let mut encoder = Encoder::new(Vec::new());
    encoder
        .map(7)
        .and_then(|encoder| encoder.u8(0))
        .and_then(|encoder| encoder.u16(PROFILE_VERSION))
        .and_then(|encoder| encoder.u8(1))
        .and_then(|encoder| encoder.bytes(identity_id.as_bytes()))
        .and_then(|encoder| encoder.u8(2))
        .and_then(|encoder| encoder.bytes(identity_public_key))
        .and_then(|encoder| encoder.u8(3))
        .and_then(|encoder| encoder.bytes(repository_public_key))
        .and_then(|encoder| encoder.u8(4))
        .and_then(|encoder| encoder.u32(epoch))
        .and_then(|encoder| encoder.u8(5))
        .and_then(|encoder| encoder.str(PURPOSE))
        .and_then(|encoder| encoder.u8(6))
        .and_then(|encoder| encoder.u8(0))
        .map_err(|_| RepositorySignerBindingError::Format)?;
    let bytes = encoder.into_writer();
    if bytes.len() > MAX_PROTECTED_BYTES {
        return Err(RepositorySignerBindingError::TooLarge);
    }
    Ok(bytes)
}

fn encode_outer(
    protected: &[u8],
    signature: &[u8; 64],
) -> Result<Vec<u8>, RepositorySignerBindingError> {
    let mut encoder = Encoder::new(Vec::new());
    encoder
        .map(2)
        .and_then(|encoder| encoder.u8(0))
        .and_then(|encoder| encoder.bytes(protected))
        .and_then(|encoder| encoder.u8(1))
        .and_then(|encoder| encoder.bytes(signature))
        .map_err(|_| RepositorySignerBindingError::Format)?;
    let bytes = encoder.into_writer();
    if bytes.len() > MAX_BINDING_BYTES {
        return Err(RepositorySignerBindingError::TooLarge);
    }
    Ok(bytes)
}

fn decode_outer(bytes: &[u8]) -> Result<(&[u8], [u8; 64]), RepositorySignerBindingError> {
    let mut decoder = Decoder::new(bytes);
    require_map(&mut decoder, 2)?;
    require_key(&mut decoder, 0)?;
    let protected = decoder.bytes().map_err(format_error)?;
    require_key(&mut decoder, 1)?;
    let signature = decoder.bytes().map_err(format_error)?;
    let signature = signature.try_into().map_err(|_| RepositorySignerBindingError::Format)?;
    if decoder.position() != bytes.len() {
        return Err(RepositorySignerBindingError::Format);
    }
    Ok((protected, signature))
}

fn decode_protected(bytes: &[u8]) -> Result<Claims, RepositorySignerBindingError> {
    let mut decoder = Decoder::new(bytes);
    require_map(&mut decoder, 7)?;
    require_key(&mut decoder, 0)?;
    let version = decoder.u16().map_err(format_error)?;
    if version != PROFILE_VERSION {
        return Err(RepositorySignerBindingError::Semantic);
    }
    require_key(&mut decoder, 1)?;
    let identity_id = IdentityId::from_bytes(
        decoder
            .bytes()
            .map_err(format_error)?
            .try_into()
            .map_err(|_| RepositorySignerBindingError::Format)?,
    );
    require_key(&mut decoder, 2)?;
    let identity_public_key = decoder
        .bytes()
        .map_err(format_error)?
        .try_into()
        .map_err(|_| RepositorySignerBindingError::Format)?;
    require_key(&mut decoder, 3)?;
    let repository_public_key = decoder
        .bytes()
        .map_err(format_error)?
        .try_into()
        .map_err(|_| RepositorySignerBindingError::Format)?;
    require_key(&mut decoder, 4)?;
    let epoch = decoder.u32().map_err(format_error)?;
    require_key(&mut decoder, 5)?;
    if decoder.str().map_err(format_error)? != PURPOSE {
        return Err(RepositorySignerBindingError::Semantic);
    }
    require_key(&mut decoder, 6)?;
    if decoder.u8().map_err(format_error)? != 0 {
        return Err(RepositorySignerBindingError::Semantic);
    }
    if decoder.position() != bytes.len() {
        return Err(RepositorySignerBindingError::Format);
    }
    Ok(Claims { identity_id, identity_public_key, repository_public_key, epoch })
}

fn require_map(
    decoder: &mut Decoder<'_>,
    expected: u64,
) -> Result<(), RepositorySignerBindingError> {
    match decoder.map().map_err(format_error)? {
        Some(actual) if actual == expected => Ok(()),
        Some(_) => Err(RepositorySignerBindingError::Format),
        None => Err(RepositorySignerBindingError::Canonical),
    }
}

fn require_key(
    decoder: &mut Decoder<'_>,
    expected: u8,
) -> Result<(), RepositorySignerBindingError> {
    if decoder.u8().map_err(format_error)? == expected {
        Ok(())
    } else {
        Err(RepositorySignerBindingError::Format)
    }
}

fn signing_frame(protected: &[u8]) -> Result<Vec<u8>, RepositorySignerBindingError> {
    let domain_len =
        u16::try_from(SIGNING_DOMAIN.len()).map_err(|_| RepositorySignerBindingError::Format)?;
    let protected_len =
        u32::try_from(protected.len()).map_err(|_| RepositorySignerBindingError::Format)?;
    let mut frame = Vec::with_capacity(2 + SIGNING_DOMAIN.len() + 2 + 4 + protected.len());
    frame.extend_from_slice(&domain_len.to_be_bytes());
    frame.extend_from_slice(SIGNING_DOMAIN);
    frame.extend_from_slice(&PROFILE_VERSION.to_be_bytes());
    frame.extend_from_slice(&protected_len.to_be_bytes());
    frame.extend_from_slice(protected);
    Ok(frame)
}

fn validate_public_key(bytes: &[u8; 32]) -> Result<(), RepositorySignerBindingError> {
    let key =
        VerifyingKey::from_bytes(bytes).map_err(|_| RepositorySignerBindingError::Signature)?;
    if key.is_weak() { Err(RepositorySignerBindingError::Signature) } else { Ok(()) }
}

fn format_error(_error: minicbor::decode::Error) -> RepositorySignerBindingError {
    RepositorySignerBindingError::Format
}

/// Stable rejection class for profile vectors and downstream adapters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepositorySignerBindingErrorClass {
    Format,
    TooLarge,
    Canonical,
    Semantic,
    IdentityMismatch,
    Signature,
}

/// Strict repository signer binding verification failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RepositorySignerBindingError {
    #[error("repository signer binding has invalid structure or field shape")]
    Format,
    #[error("repository signer binding exceeds its profile byte limit")]
    TooLarge,
    #[error("repository signer binding is not canonically encoded")]
    Canonical,
    #[error("repository signer binding has invalid profile semantics")]
    Semantic,
    #[error("repository signer binding Identity ID does not match its public key")]
    IdentityMismatch,
    #[error("repository signer binding public key or signature is invalid")]
    Signature,
}

impl RepositorySignerBindingError {
    pub const fn class(self) -> RepositorySignerBindingErrorClass {
        match self {
            Self::Format => RepositorySignerBindingErrorClass::Format,
            Self::TooLarge => RepositorySignerBindingErrorClass::TooLarge,
            Self::Canonical => RepositorySignerBindingErrorClass::Canonical,
            Self::Semantic => RepositorySignerBindingErrorClass::Semantic,
            Self::IdentityMismatch => RepositorySignerBindingErrorClass::IdentityMismatch,
            Self::Signature => RepositorySignerBindingErrorClass::Signature,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::derive::{KeyDeriver, KeyPurpose};
    use proptest::prelude::*;

    struct FixtureActor {
        deriver: KeyDeriver,
        identity_key: SigningKey,
    }

    impl FixtureActor {
        fn new(public_test_fill: u8) -> Self {
            let deriver = KeyDeriver::new(&[public_test_fill; 32]);
            let identity_key = SigningKey::from_bytes(&deriver.signing_seed());
            Self { deriver, identity_key }
        }

        fn repository_key(&self, epoch: u32) -> SigningKey {
            SigningKey::from_bytes(&self.deriver.derive_repository_signing_key(epoch))
        }

        fn binding(&self, epoch: u32) -> RepositorySignerBinding {
            RepositorySignerBinding::issue(
                &self.identity_key,
                self.repository_key(epoch).verifying_key().to_bytes(),
                epoch,
            )
            .expect("fixture binding")
        }
    }

    #[test]
    fn identity_id_is_strict_and_matches_existing_vector() {
        let deriver = KeyDeriver::new(&[0x42; 32]);
        let identity_key = SigningKey::from_bytes(&deriver.signing_seed());
        let id = IdentityId::from_public_key(&identity_key.verifying_key().to_bytes());
        assert_eq!(id.to_string(), "6279e31aff9bc151638ac305d88ab6bc");
        assert_eq!(id.to_string().parse::<IdentityId>(), Ok(id));
        assert!(id.to_string().to_uppercase().parse::<IdentityId>().is_err());
        assert!("6279".parse::<IdentityId>().is_err());
        assert!("g279e31aff9bc151638ac305d88ab6bc".parse::<IdentityId>().is_err());
    }

    #[test]
    fn repository_derivation_is_epoch_indexed_and_separated() {
        let deriver = KeyDeriver::new(&[0x42; 32]);
        let zero = deriver.derive_repository_signing_key(0);
        let one = deriver.derive_repository_signing_key(1);
        let maximum = deriver.derive_repository_signing_key(u32::MAX);
        assert_ne!(zero, one);
        assert_ne!(one, maximum);
        assert_eq!(maximum, deriver.derive_repository_signing_key(u32::MAX));
        for purpose in KeyPurpose::all() {
            assert_ne!(zero, deriver.derive(*purpose));
        }
    }

    #[test]
    fn repository_derivation_has_fixed_epoch_vectors() {
        let deriver = KeyDeriver::new(&[0x42; 32]);
        assert_eq!(
            hex::encode(deriver.derive_repository_signing_key(0)),
            "9140cc2ce009c3e9bb7f9d4c73cf4457ef530ea9f5c2d5bea8efba0cd082b504"
        );
        assert_eq!(
            hex::encode(deriver.derive_repository_signing_key(1)),
            "4d832d6783216b864631fcf2493b1f09e5f6010abc3a7d8e011851bf14a28d3f"
        );
        assert_eq!(
            hex::encode(deriver.derive_repository_signing_key(u32::MAX)),
            "f92e32c7ae44cd692d3a8115e0b175c099e5a2e82daa51240181b61136c06ccc"
        );
    }

    proptest! {
        #[test]
        fn identity_ids_round_trip_and_match_public_keys(
            identity_seed in prop::array::uniform32(any::<u8>()),
            bytes in prop::array::uniform16(any::<u8>()),
        ) {
            let public_key = SigningKey::from_bytes(&identity_seed).verifying_key().to_bytes();
            let derived = IdentityId::from_public_key(&public_key);
            prop_assert!(derived.matches_public_key(&public_key));
            prop_assert_eq!(derived.to_string().parse::<IdentityId>(), Ok(derived));

            let id = IdentityId::from_bytes(bytes);
            prop_assert_eq!(id.to_string().parse::<IdentityId>(), Ok(id));
        }

        #[test]
        fn repository_family_is_deterministic_separate_and_does_not_wrap(
            root in prop::array::uniform32(any::<u8>()),
            epoch in any::<u32>(),
        ) {
            let deriver = KeyDeriver::new(&root);
            let repository = deriver.derive_repository_signing_key(epoch);
            let repository_public = SigningKey::from_bytes(&repository).verifying_key().to_bytes();
            prop_assert_eq!(repository, deriver.derive_repository_signing_key(epoch));
            for purpose in KeyPurpose::all() {
                let other = deriver.derive(*purpose);
                prop_assert_ne!(repository, other);
                if matches!(
                    purpose,
                    KeyPurpose::Signing
                        | KeyPurpose::SshHost
                        | KeyPurpose::Yggdrasil
                        | KeyPurpose::I2pSigning
                        | KeyPurpose::Tor
                ) {
                    prop_assert_ne!(
                        repository_public,
                        SigningKey::from_bytes(&other).verifying_key().to_bytes()
                    );
                }
            }
            for other in [
                deriver.derive_agent_key("repository").expect("label"),
                deriver.derive_ssh_user_key("repository").expect("label"),
                deriver.derive_tls_certificate_key("repository").expect("label"),
            ] {
                prop_assert_ne!(repository, other);
                prop_assert_ne!(
                    repository_public,
                    SigningKey::from_bytes(&other).verifying_key().to_bytes()
                );
            }

            if epoch < u32::MAX {
                prop_assert_ne!(repository, deriver.derive_repository_signing_key(epoch + 1));
            }
        }
    }

    #[test]
    fn binding_round_trips_and_detects_mutation() {
        let deriver = KeyDeriver::new(&[0x42; 32]);
        let identity_key = SigningKey::from_bytes(&deriver.signing_seed());
        let repository_key = SigningKey::from_bytes(&deriver.derive_repository_signing_key(1));
        let binding = RepositorySignerBinding::issue(
            &identity_key,
            repository_key.verifying_key().to_bytes(),
            1,
        )
        .expect("issue binding");
        let bytes = binding.canonical_bytes().expect("binding bytes");
        let verified = verify_repository_signer_binding(&bytes).expect("verify binding");
        assert_eq!(verified.epoch(), 1);
        assert_eq!(
            verified.identity_id(),
            IdentityId::from_public_key(&identity_key.verifying_key().to_bytes())
        );
        assert_eq!(verified.repository_public_key(), &repository_key.verifying_key().to_bytes());
        assert_eq!(verified.digest(), &binding.digest().expect("binding digest"));

        let mut mutated = bytes;
        let last = mutated.last_mut().expect("binding is nonempty");
        *last ^= 1;
        assert_eq!(
            verify_repository_signer_binding(&mutated).expect_err("mutation must fail").class(),
            RepositorySignerBindingErrorClass::Signature
        );
    }

    #[test]
    fn committed_positive_corpus_freezes_every_signed_layer() {
        let corpus: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/vectors/repository-signing-v1/positive.json"
        ))
        .expect("positive corpus JSON");
        for vector in corpus["vectors"].as_array().expect("vector array") {
            let root: [u8; 32] =
                decode_hex_field(vector, "root_secret_hex").try_into().expect("root length");
            let epoch = u32::try_from(vector["epoch"].as_u64().expect("epoch")).expect("u32 epoch");
            let repository_public_key: [u8; 32] =
                decode_hex_field(vector, "repository_public_key_hex")
                    .try_into()
                    .expect("repository key length");
            let deriver = KeyDeriver::new(&root);
            let identity_key = SigningKey::from_bytes(&deriver.signing_seed());
            let binding =
                RepositorySignerBinding::issue(&identity_key, repository_public_key, epoch)
                    .expect("issue vector binding");
            assert_eq!(binding.protected_bytes(), decode_hex_field(vector, "protected_hex"));
            assert_eq!(
                binding.signing_frame().expect("frame"),
                decode_hex_field(vector, "signing_frame_hex")
            );
            assert_eq!(binding.signature(), decode_hex_field(vector, "signature_hex").as_slice());
            assert_eq!(
                binding.canonical_bytes().expect("outer"),
                decode_hex_field(vector, "binding_hex")
            );
            assert_eq!(
                binding.digest().expect("digest"),
                decode_hex_field(vector, "binding_digest_hex").as_slice()
            );
            verify_repository_signer_binding(&decode_hex_field(vector, "binding_hex"))
                .expect("verify committed vector");
        }
    }

    fn decode_hex_field(vector: &serde_json::Value, field: &str) -> Vec<u8> {
        hex::decode(vector[field].as_str().expect("hex field")).expect("valid hex")
    }

    #[test]
    fn field_mutation_vectors_have_stable_rejection_classes() {
        let deriver = KeyDeriver::new(&[0x42; 32]);
        let identity_key = SigningKey::from_bytes(&deriver.signing_seed());
        let identity_public_key = identity_key.verifying_key().to_bytes();
        let identity_id = IdentityId::from_public_key(&identity_public_key);
        let repository_public_key =
            SigningKey::from_bytes(&deriver.derive_repository_signing_key(1))
                .verifying_key()
                .to_bytes();
        let other_identity_key = SigningKey::from_bytes(&[0x24; 32]);
        let other_repository_key = SigningKey::from_bytes(&[0x25; 32]).verifying_key().to_bytes();
        let canonical = test_protected(TestClaims {
            version: 1,
            identity_id,
            identity_public_key,
            repository_public_key,
            epoch: 1,
            purpose: PURPOSE,
            suite: 0,
        });
        let canonical_signature =
            identity_key.sign(&signing_frame(&canonical).expect("frame")).to_bytes();

        let vectors = [
            (
                "version",
                signed_test_outer(
                    &identity_key,
                    test_protected(TestClaims {
                        version: 2,
                        ..TestClaims::canonical(
                            identity_id,
                            identity_public_key,
                            repository_public_key,
                        )
                    }),
                ),
                RepositorySignerBindingErrorClass::Semantic,
            ),
            (
                "identity-id",
                signed_test_outer(
                    &identity_key,
                    test_protected(TestClaims {
                        identity_id: IdentityId::from_bytes([0; 16]),
                        ..TestClaims::canonical(
                            identity_id,
                            identity_public_key,
                            repository_public_key,
                        )
                    }),
                ),
                RepositorySignerBindingErrorClass::IdentityMismatch,
            ),
            (
                "identity-key",
                signed_test_outer(
                    &identity_key,
                    test_protected(TestClaims {
                        identity_public_key: other_identity_key.verifying_key().to_bytes(),
                        ..TestClaims::canonical(
                            identity_id,
                            identity_public_key,
                            repository_public_key,
                        )
                    }),
                ),
                RepositorySignerBindingErrorClass::IdentityMismatch,
            ),
            (
                "repository-key",
                encode_outer(
                    &test_protected(TestClaims {
                        repository_public_key: other_repository_key,
                        ..TestClaims::canonical(
                            identity_id,
                            identity_public_key,
                            repository_public_key,
                        )
                    }),
                    &canonical_signature,
                )
                .expect("outer"),
                RepositorySignerBindingErrorClass::Signature,
            ),
            (
                "epoch",
                encode_outer(
                    &test_protected(TestClaims {
                        epoch: 2,
                        ..TestClaims::canonical(
                            identity_id,
                            identity_public_key,
                            repository_public_key,
                        )
                    }),
                    &canonical_signature,
                )
                .expect("outer"),
                RepositorySignerBindingErrorClass::Signature,
            ),
            (
                "purpose",
                signed_test_outer(
                    &identity_key,
                    test_protected(TestClaims {
                        purpose: "ordinary-git-signing",
                        ..TestClaims::canonical(
                            identity_id,
                            identity_public_key,
                            repository_public_key,
                        )
                    }),
                ),
                RepositorySignerBindingErrorClass::Semantic,
            ),
            (
                "suite",
                signed_test_outer(
                    &identity_key,
                    test_protected(TestClaims {
                        suite: 1,
                        ..TestClaims::canonical(
                            identity_id,
                            identity_public_key,
                            repository_public_key,
                        )
                    }),
                ),
                RepositorySignerBindingErrorClass::Semantic,
            ),
        ];
        for (name, bytes, class) in vectors {
            let error = match verify_repository_signer_binding(&bytes) {
                Ok(_) => panic!("{name} unexpectedly verified"),
                Err(error) => error,
            };
            assert_eq!(error.class(), class, "{name} produced unexpected error: {error}");
        }
    }

    #[derive(Clone, Copy)]
    struct TestClaims<'a> {
        version: u16,
        identity_id: IdentityId,
        identity_public_key: [u8; 32],
        repository_public_key: [u8; 32],
        epoch: u32,
        purpose: &'a str,
        suite: u8,
    }

    impl TestClaims<'static> {
        fn canonical(
            identity_id: IdentityId,
            identity_public_key: [u8; 32],
            repository_public_key: [u8; 32],
        ) -> Self {
            Self {
                version: 1,
                identity_id,
                identity_public_key,
                repository_public_key,
                epoch: 1,
                purpose: PURPOSE,
                suite: 0,
            }
        }
    }

    fn test_protected(claims: TestClaims<'_>) -> Vec<u8> {
        let mut encoder = Encoder::new(Vec::new());
        encoder
            .map(7)
            .and_then(|encoder| encoder.u8(0))
            .and_then(|encoder| encoder.u16(claims.version))
            .and_then(|encoder| encoder.u8(1))
            .and_then(|encoder| encoder.bytes(claims.identity_id.as_bytes()))
            .and_then(|encoder| encoder.u8(2))
            .and_then(|encoder| encoder.bytes(&claims.identity_public_key))
            .and_then(|encoder| encoder.u8(3))
            .and_then(|encoder| encoder.bytes(&claims.repository_public_key))
            .and_then(|encoder| encoder.u8(4))
            .and_then(|encoder| encoder.u32(claims.epoch))
            .and_then(|encoder| encoder.u8(5))
            .and_then(|encoder| encoder.str(claims.purpose))
            .and_then(|encoder| encoder.u8(6))
            .and_then(|encoder| encoder.u8(claims.suite))
            .expect("test encoding");
        encoder.into_writer()
    }

    fn signed_test_outer(identity_key: &SigningKey, protected: Vec<u8>) -> Vec<u8> {
        let signature = identity_key.sign(&signing_frame(&protected).expect("frame")).to_bytes();
        encode_outer(&protected, &signature).expect("outer")
    }

    #[test]
    fn malformed_cbor_vectors_have_stable_rejection_classes() {
        let actor = FixtureActor::new(0x42);
        let identity_key = actor.identity_key.clone();
        let repository_key = actor.repository_key(1);
        let binding = actor.binding(1);
        let valid = binding.canonical_bytes().expect("canonical binding");
        let protected = binding.protected_bytes();
        let signature = binding.signature();

        let mut non_shortest_outer = vec![0xb8, 0x02];
        non_shortest_outer.extend_from_slice(&valid[1..]);
        let mut indefinite_outer = vec![0xbf];
        indefinite_outer.extend_from_slice(&valid[1..]);
        indefinite_outer.push(0xff);
        let mut tagged = vec![0xc0];
        tagged.extend_from_slice(&valid);
        let mut non_shortest_protected = vec![0xb8, 0x07];
        non_shortest_protected.extend_from_slice(&protected[1..]);
        let mut indefinite_protected = vec![0xbf];
        indefinite_protected.extend_from_slice(&protected[1..]);
        indefinite_protected.push(0xff);
        let mut unknown_protected_key = protected.to_vec();
        unknown_protected_key[protected.len() - 2] = 7;
        let mut duplicate_protected_key = protected.to_vec();
        duplicate_protected_key[protected.len() - 2] = 5;
        let mut tagged_protected = vec![0xc0];
        tagged_protected.extend_from_slice(protected);

        let vectors = [
            (
                "truncated",
                valid[..valid.len() - 1].to_vec(),
                RepositorySignerBindingErrorClass::Format,
            ),
            (
                "wrong-outer-arity",
                vec![0xa1, 0x00, 0x40],
                RepositorySignerBindingErrorClass::Format,
            ),
            ("wrong-outer-type", vec![0x82, 0x40, 0x40], RepositorySignerBindingErrorClass::Format),
            (
                "trailing-data",
                [valid.as_slice(), &[0]].concat(),
                RepositorySignerBindingErrorClass::Format,
            ),
            (
                "short-signature",
                test_outer(protected, &[0; 63]),
                RepositorySignerBindingErrorClass::Format,
            ),
            (
                "long-signature",
                test_outer(protected, &[0; 65]),
                RepositorySignerBindingErrorClass::Format,
            ),
            (
                "unknown-outer-key",
                outer_with_keys(2, 1, protected, signature),
                RepositorySignerBindingErrorClass::Format,
            ),
            (
                "duplicate-outer-key",
                outer_with_keys(0, 0, protected, signature),
                RepositorySignerBindingErrorClass::Format,
            ),
            (
                "reordered-outer-keys",
                outer_with_keys(1, 0, protected, signature),
                RepositorySignerBindingErrorClass::Format,
            ),
            (
                "non-shortest-outer",
                non_shortest_outer,
                RepositorySignerBindingErrorClass::Canonical,
            ),
            ("indefinite-outer", indefinite_outer, RepositorySignerBindingErrorClass::Canonical),
            ("tagged-outer", tagged, RepositorySignerBindingErrorClass::Format),
            ("floating-point-outer", vec![0xf9, 0, 0], RepositorySignerBindingErrorClass::Format),
            (
                "non-shortest-protected",
                signed_test_outer(&identity_key, non_shortest_protected),
                RepositorySignerBindingErrorClass::Canonical,
            ),
            (
                "indefinite-protected",
                signed_test_outer(&identity_key, indefinite_protected),
                RepositorySignerBindingErrorClass::Canonical,
            ),
            (
                "indefinite-byte-string",
                signed_test_outer(&identity_key, vec![0x5f, 0xff]),
                RepositorySignerBindingErrorClass::Format,
            ),
            (
                "truncated-protected",
                signed_test_outer(&identity_key, protected[..protected.len() - 1].to_vec()),
                RepositorySignerBindingErrorClass::Format,
            ),
            (
                "wrong-protected-arity",
                signed_test_outer(&identity_key, vec![0xa6]),
                RepositorySignerBindingErrorClass::Format,
            ),
            (
                "wrong-protected-type",
                signed_test_outer(&identity_key, vec![0x87]),
                RepositorySignerBindingErrorClass::Format,
            ),
            (
                "unknown-protected-key",
                signed_test_outer(&identity_key, unknown_protected_key),
                RepositorySignerBindingErrorClass::Format,
            ),
            (
                "duplicate-protected-key",
                signed_test_outer(&identity_key, duplicate_protected_key),
                RepositorySignerBindingErrorClass::Format,
            ),
            (
                "reordered-protected-keys",
                signed_test_outer(
                    &identity_key,
                    reordered_test_protected(&identity_key, &repository_key),
                ),
                RepositorySignerBindingErrorClass::Format,
            ),
            (
                "tagged-protected",
                signed_test_outer(&identity_key, tagged_protected),
                RepositorySignerBindingErrorClass::Format,
            ),
            (
                "floating-point-protected",
                signed_test_outer(&identity_key, vec![0xf9, 0, 0]),
                RepositorySignerBindingErrorClass::Format,
            ),
        ];
        for (name, bytes, class) in vectors {
            assert_rejection_class(name, &bytes, class);
        }

        for length in [MAX_PROTECTED_BYTES - 1, MAX_PROTECTED_BYTES] {
            let mut at_or_below_limit = vec![0; length];
            at_or_below_limit[0] = 0xa0;
            assert_rejection_class(
                "protected-at-or-below-limit",
                &test_outer(&at_or_below_limit, signature),
                RepositorySignerBindingErrorClass::Format,
            );
        }
        let mut over_protected_limit = vec![0; MAX_PROTECTED_BYTES + 1];
        over_protected_limit[0] = 0xa0;
        assert_rejection_class(
            "protected-over-limit",
            &test_outer(&over_protected_limit, signature),
            RepositorySignerBindingErrorClass::TooLarge,
        );

        for length in [MAX_BINDING_BYTES - 1, MAX_BINDING_BYTES] {
            let mut at_or_below_limit = vec![0; length];
            at_or_below_limit[0] = 0xa0;
            assert_rejection_class(
                "binding-at-or-below-limit",
                &at_or_below_limit,
                RepositorySignerBindingErrorClass::Format,
            );
        }
        assert_rejection_class(
            "binding-over-limit",
            &vec![0; MAX_BINDING_BYTES + 1],
            RepositorySignerBindingErrorClass::TooLarge,
        );
    }

    fn test_outer(protected: &[u8], signature: &[u8]) -> Vec<u8> {
        let mut encoder = Encoder::new(Vec::new());
        encoder
            .map(2)
            .and_then(|encoder| encoder.u8(0))
            .and_then(|encoder| encoder.bytes(protected))
            .and_then(|encoder| encoder.u8(1))
            .and_then(|encoder| encoder.bytes(signature))
            .expect("test outer encoding");
        encoder.into_writer()
    }

    fn reordered_test_protected(identity_key: &SigningKey, repository_key: &SigningKey) -> Vec<u8> {
        let identity_public_key = identity_key.verifying_key().to_bytes();
        let identity_id = IdentityId::from_public_key(&identity_public_key);
        let mut encoder = Encoder::new(Vec::new());
        encoder
            .map(7)
            .and_then(|encoder| encoder.u8(1))
            .and_then(|encoder| encoder.bytes(identity_id.as_bytes()))
            .and_then(|encoder| encoder.u8(0))
            .and_then(|encoder| encoder.u16(1))
            .and_then(|encoder| encoder.u8(2))
            .and_then(|encoder| encoder.bytes(&identity_public_key))
            .and_then(|encoder| encoder.u8(3))
            .and_then(|encoder| encoder.bytes(&repository_key.verifying_key().to_bytes()))
            .and_then(|encoder| encoder.u8(4))
            .and_then(|encoder| encoder.u32(1))
            .and_then(|encoder| encoder.u8(5))
            .and_then(|encoder| encoder.str(PURPOSE))
            .and_then(|encoder| encoder.u8(6))
            .and_then(|encoder| encoder.u8(0))
            .expect("test protected encoding");
        encoder.into_writer()
    }

    fn outer_with_keys(
        first_key: u8,
        second_key: u8,
        protected: &[u8],
        signature: &[u8],
    ) -> Vec<u8> {
        let mut encoder = Encoder::new(Vec::new());
        encoder
            .map(2)
            .and_then(|encoder| encoder.u8(first_key))
            .and_then(|encoder| encoder.bytes(protected))
            .and_then(|encoder| encoder.u8(second_key))
            .and_then(|encoder| encoder.bytes(signature))
            .expect("test outer encoding");
        encoder.into_writer()
    }

    fn assert_rejection_class(
        name: &str,
        bytes: &[u8],
        expected: RepositorySignerBindingErrorClass,
    ) {
        let error = match verify_repository_signer_binding(bytes) {
            Ok(_) => panic!("{name} unexpectedly verified"),
            Err(error) => error,
        };
        assert_eq!(error.class(), expected, "{name}: {error}");
    }

    #[test]
    fn verifier_rejects_trailing_indefinite_and_oversized_input() {
        assert_eq!(
            verify_repository_signer_binding(&[0xbf, 0xff])
                .expect_err("indefinite map must fail")
                .class(),
            RepositorySignerBindingErrorClass::Canonical
        );
        assert_eq!(
            verify_repository_signer_binding(&vec![0; MAX_BINDING_BYTES + 1])
                .expect_err("oversize must fail")
                .class(),
            RepositorySignerBindingErrorClass::TooLarge
        );
    }

    #[test]
    fn externally_supplied_key_and_maximum_epoch_verify() {
        let deriver = KeyDeriver::new(&[0x42; 32]);
        let identity_key = SigningKey::from_bytes(&deriver.signing_seed());
        let external_key = SigningKey::from_bytes(&[0xa5; 32]).verifying_key().to_bytes();
        assert_ne!(
            external_key,
            SigningKey::from_bytes(&deriver.derive_repository_signing_key(u32::MAX))
                .verifying_key()
                .to_bytes()
        );
        let binding = RepositorySignerBinding::issue(&identity_key, external_key, u32::MAX)
            .expect("issue external binding");
        let verified = verify_repository_signer_binding(
            &binding.canonical_bytes().expect("external binding bytes"),
        )
        .expect("verify external binding");
        assert_eq!(verified.repository_public_key(), &external_key);
        assert_eq!(verified.epoch(), u32::MAX);
    }
}
