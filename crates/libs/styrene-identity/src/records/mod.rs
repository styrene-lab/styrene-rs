//! Canonical profile-v1 signed identity record primitives.

pub mod lifecycle_transition;
pub mod runtime_certificate;

use minicbor::{Decode, Encode};
use sha2::{Digest, Sha256};

pub const RECORD_PROFILE_VERSION: u16 = 1;
pub const KEY_ID_DOMAIN: &[u8] = b"styrene-key-id-v1\0";
pub const RUNTIME_CERTIFICATE_DOMAIN: &[u8] = b"styrene-runtime-certificate-v1";
pub const LIFECYCLE_TRANSITION_DOMAIN: &[u8] = b"styrene-lifecycle-transition-v1";
pub const MAX_RUNTIME_CERTIFICATE_BYTES: usize = 16_384;
pub const MAX_LIFECYCLE_TRANSITION_BYTES: usize = 65_535;

pub type Digest32 = [u8; 32];
pub type Id16 = [u8; 16];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(index_only)]
pub enum SignatureSuite {
    #[n(0)]
    Ed25519,
    #[n(1)]
    EcdsaP256Sha256,
    #[n(2)]
    RsaPssSha256,
}

impl SignatureSuite {
    pub const fn id(self) -> u16 {
        match self {
            Self::Ed25519 => 0,
            Self::EcdsaP256Sha256 => 1,
            Self::RsaPssSha256 => 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct RsaPublicKey {
    #[n(0)]
    pub modulus: Vec<u8>,
    #[n(1)]
    pub exponent: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PublicKey {
    Ed25519([u8; 32]),
    EcdsaP256([u8; 33]),
    RsaPss(RsaPublicKey),
}

impl PublicKey {
    pub const fn suite(&self) -> SignatureSuite {
        match self {
            Self::Ed25519(_) => SignatureSuite::Ed25519,
            Self::EcdsaP256(_) => SignatureSuite::EcdsaP256Sha256,
            Self::RsaPss(_) => SignatureSuite::RsaPssSha256,
        }
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, RecordError> {
        self.validate()?;
        match self {
            Self::Ed25519(bytes) => Ok(bytes.to_vec()),
            Self::EcdsaP256(bytes) => Ok(bytes.to_vec()),
            Self::RsaPss(key) => minicbor::to_vec(key).map_err(encode_error),
        }
    }

    pub fn validate(&self) -> Result<(), RecordError> {
        match self {
            Self::Ed25519(_) => Ok(()),
            Self::EcdsaP256(bytes) if matches!(bytes[0], 0x02 | 0x03) => Ok(()),
            Self::EcdsaP256(_) => Err(RecordError::InvalidPublicKey),
            Self::RsaPss(key) => {
                let bits = key.modulus.len().checked_mul(8).ok_or(RecordError::InvalidPublicKey)?;
                if !matches!(bits, 2048 | 3072 | 4096)
                    || key.modulus.first().copied().unwrap_or(0) == 0
                    || key.exponent != 65_537
                {
                    return Err(RecordError::InvalidPublicKey);
                }
                Ok(())
            }
        }
    }
}

pub fn derive_key_id(public_key: &PublicKey) -> Result<Digest32, RecordError> {
    let canonical = public_key.canonical_bytes()?;
    let mut hasher = Sha256::new();
    hasher.update(KEY_ID_DOMAIN);
    hasher.update(public_key.suite().id().to_be_bytes());
    hasher.update(canonical);
    Ok(hasher.finalize().into())
}

pub fn signing_frame(domain: &[u8], protected: &[u8]) -> Result<Vec<u8>, RecordError> {
    let domain_len = u16::try_from(domain.len()).map_err(|_| RecordError::LengthOverflow)?;
    let protected_len = u32::try_from(protected.len()).map_err(|_| RecordError::LengthOverflow)?;
    if domain.is_empty() || !domain.is_ascii() {
        return Err(RecordError::InvalidDomain);
    }
    let mut frame = Vec::with_capacity(2 + domain.len() + 2 + 4 + protected.len());
    frame.extend_from_slice(&domain_len.to_be_bytes());
    frame.extend_from_slice(domain);
    frame.extend_from_slice(&RECORD_PROFILE_VERSION.to_be_bytes());
    frame.extend_from_slice(&protected_len.to_be_bytes());
    frame.extend_from_slice(protected);
    Ok(frame)
}

pub fn sha256(bytes: &[u8]) -> Digest32 {
    Sha256::digest(bytes).into()
}

pub(crate) fn validate_ascii_identifier(value: &str, max: usize) -> Result<(), RecordError> {
    let bytes = value.as_bytes();
    if bytes.is_empty()
        || bytes.len() > max
        || !bytes.iter().all(|byte| (0x21..=0x7e).contains(byte))
    {
        return Err(RecordError::InvalidIdentifier);
    }
    Ok(())
}

pub(crate) fn encode_error(
    error: minicbor::encode::Error<std::convert::Infallible>,
) -> RecordError {
    RecordError::Encode(error.to_string())
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RecordError {
    #[error("failed to encode record: {0}")]
    Encode(String),
    #[error("unsupported record profile version {0}")]
    UnsupportedVersion(u16),
    #[error("record exceeds its profile byte limit: {actual} > {max}")]
    TooLarge { actual: usize, max: usize },
    #[error("record length overflows its framing integer")]
    LengthOverflow,
    #[error("invalid signing domain")]
    InvalidDomain,
    #[error("invalid canonical security identifier")]
    InvalidIdentifier,
    #[error("public key is invalid for its declared suite")]
    InvalidPublicKey,
    #[error("public key suite does not match the declared suite")]
    SuiteMismatch,
    #[error("derived key id does not match the public key")]
    KeyIdMismatch,
    #[error("certificate validity or renewal interval is invalid")]
    InvalidValidity,
    #[error("custody fields are inconsistent with the custody class")]
    InvalidCustody,
    #[error("lifecycle domain key is invalid")]
    InvalidDomainKey,
    #[error("lifecycle transition fields are inconsistent")]
    InvalidTransition,
    #[error("lifecycle revision or predecessor is invalid")]
    InvalidRevision,
    #[error("reconciliation payload is invalid")]
    InvalidReconciliation,
}

#[cfg(test)]
mod tests {
    use super::lifecycle_transition::*;
    use super::runtime_certificate::*;
    use super::*;

    fn sample_claims() -> RuntimeCertificateClaims {
        let public_key = PublicKey::Ed25519([7; 32]);
        RuntimeCertificateClaims {
            profile_version: 1,
            certificate_id: [1; 16],
            issuer_key_id: [2; 32],
            issuer_key_version: 0,
            issuer_suite: SignatureSuite::Ed25519,
            subject_kind: SubjectKind::Agent,
            subject_id: "agent-1".into(),
            runtime_id: [3; 16],
            runtime_key_id: derive_key_id(&public_key).unwrap(),
            runtime_key_version: 0,
            runtime_suite: SignatureSuite::Ed25519,
            runtime_public_key: CanonicalPublicKey::Ed25519([7; 32]),
            host_subject_binding: [4; 32],
            not_before_ms: 1_000,
            not_after_ms: 86_401_000,
            renew_after_ms: 60_001_000,
            custody_class: CustodyClass::SoftwareEphemeral,
            evidence_profile_id: None,
            evidence_digest: None,
            attestation_verifier_id: None,
            attested_at_ms: None,
            degraded: false,
            predecessor_certificate_digest: None,
            issuance_revision: 0,
            extensions_digest: None,
        }
    }

    #[test]
    fn runtime_certificate_is_deterministic_and_bounded() {
        let claims = sample_claims();
        let protected = claims.protected_bytes().unwrap();
        assert_eq!(protected, claims.protected_bytes().unwrap());
        let frame = claims.signing_input().unwrap();
        assert_eq!(&frame[2..32], RUNTIME_CERTIFICATE_DOMAIN);
        let certificate = RuntimeCertificate::from_claims(&claims, vec![9; 64]).unwrap();
        assert!(certificate.encode().unwrap().len() < MAX_RUNTIME_CERTIFICATE_BYTES);
        assert_eq!(certificate.digest().unwrap(), certificate.digest().unwrap());
    }

    #[test]
    fn runtime_certificate_rejects_key_id_and_custody_mismatch() {
        let mut claims = sample_claims();
        claims.runtime_key_id = [0; 32];
        assert_eq!(claims.validate(), Err(RecordError::KeyIdMismatch));
        let mut claims = sample_claims();
        claims.custody_class = CustodyClass::HardwareNonexportable;
        assert_eq!(claims.validate(), Err(RecordError::InvalidCustody));
    }

    #[test]
    fn lifecycle_transition_enforces_revision_and_compromise_rules() {
        let mut claims = LifecycleTransitionClaims {
            profile_version: 1,
            transition_id: [1; 16],
            domain_key: LifecycleDomainKey::KeyFamily([2; 32]),
            revision: 0,
            previous_transition_digest: None,
            transition_kind: TransitionKind::Bootstrap,
            target_kind: TargetKind::KeyVersion,
            target_id: [3; 32],
            effective_at_ms: 5,
            reason_code: ReasonCode::Unspecified,
            issuer_key_id: [4; 32],
            issuer_key_version: 0,
            issuer_suite: SignatureSuite::Ed25519,
            replacement_digest: None,
            compromise_not_before_ms: None,
            descendant_scope: DescendantScope::TargetOnly,
            reconciliation: None,
            extensions_digest: None,
        };
        assert!(claims.validate().is_ok());
        claims.revision = 1;
        assert_eq!(claims.validate(), Err(RecordError::InvalidRevision));
        claims.previous_transition_digest = Some([9; 32]);
        claims.transition_kind = TransitionKind::RevokeCompromise;
        claims.descendant_scope = DescendantScope::KeyVersionAndCertificates;
        claims.compromise_not_before_ms = Some(4);
        assert!(claims.validate().is_ok());
    }

    #[test]
    fn reconciliation_requires_sorted_unique_rejected_heads() {
        let value = Reconciliation {
            fork_predecessor_digest: [1; 32],
            winning_head_digest: [2; 32],
            rejected_head_digests: vec![[4; 32], [3; 32]],
            superior_domain_key: LifecycleDomainKey::OwnerState("example.test".into()),
            superior_transition_digest: [5; 32],
        };
        assert_eq!(value.validate(), Err(RecordError::InvalidReconciliation));
    }
}
