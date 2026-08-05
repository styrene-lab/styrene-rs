use minicbor::{Decode, Encode};

use super::{
    derive_key_id, encode_error, sha256, signing_frame, validate_ascii_identifier, Digest32, Id16,
    PublicKey, RecordError, SignatureSuite, MAX_RUNTIME_CERTIFICATE_BYTES, RECORD_PROFILE_VERSION,
    RUNTIME_CERTIFICATE_DOMAIN,
};

pub const MAX_RUNTIME_LIFETIME_MS: u64 = 86_400_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(index_only)]
pub enum SubjectKind {
    #[n(0)]
    Agent,
    #[n(1)]
    DurableWorkload,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(index_only)]
pub enum CustodyClass {
    #[n(0)]
    SoftwareEphemeral,
    #[n(1)]
    OsProtected,
    #[n(2)]
    HardwareNonexportable,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct RuntimeCertificateClaims {
    #[n(0)]
    pub profile_version: u16,
    #[n(1)]
    pub certificate_id: Id16,
    #[n(2)]
    pub issuer_key_id: Digest32,
    #[n(3)]
    pub issuer_key_version: u64,
    #[n(4)]
    pub issuer_suite: SignatureSuite,
    #[n(5)]
    pub subject_kind: SubjectKind,
    #[n(6)]
    pub subject_id: String,
    #[n(7)]
    pub runtime_id: Id16,
    #[n(8)]
    pub runtime_key_id: Digest32,
    #[n(9)]
    pub runtime_key_version: u64,
    #[n(10)]
    pub runtime_suite: SignatureSuite,
    #[n(11)]
    pub runtime_public_key: CanonicalPublicKey,
    #[n(12)]
    pub host_subject_binding: Digest32,
    #[n(13)]
    pub not_before_ms: u64,
    #[n(14)]
    pub not_after_ms: u64,
    #[n(15)]
    pub renew_after_ms: u64,
    #[n(16)]
    pub custody_class: CustodyClass,
    #[n(17)]
    pub evidence_profile_id: Option<String>,
    #[n(18)]
    pub evidence_digest: Option<Digest32>,
    #[n(19)]
    pub attestation_verifier_id: Option<Digest32>,
    #[n(20)]
    pub attested_at_ms: Option<u64>,
    #[n(21)]
    pub degraded: bool,
    #[n(22)]
    pub predecessor_certificate_digest: Option<Digest32>,
    #[n(23)]
    pub issuance_revision: u64,
    #[n(24)]
    pub extensions_digest: Option<Digest32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum CanonicalPublicKey {
    #[n(0)]
    Ed25519(#[n(0)] [u8; 32]),
    #[n(1)]
    EcdsaP256(#[n(0)] Vec<u8>),
    #[n(2)]
    RsaPss(#[n(0)] super::RsaPublicKey),
}

impl CanonicalPublicKey {
    pub fn to_public_key(&self) -> Result<PublicKey, RecordError> {
        match self {
            Self::Ed25519(value) => Ok(PublicKey::Ed25519(*value)),
            Self::EcdsaP256(value) => {
                let value: [u8; 33] =
                    value.as_slice().try_into().map_err(|_| RecordError::InvalidPublicKey)?;
                Ok(PublicKey::EcdsaP256(value))
            }
            Self::RsaPss(value) => Ok(PublicKey::RsaPss(value.clone())),
        }
    }
}

impl RuntimeCertificateClaims {
    pub fn validate(&self) -> Result<(), RecordError> {
        if self.profile_version != RECORD_PROFILE_VERSION {
            return Err(RecordError::UnsupportedVersion(self.profile_version));
        }
        validate_ascii_identifier(&self.subject_id, 255)?;
        if let Some(profile) = &self.evidence_profile_id {
            validate_ascii_identifier(profile, 127)?;
        }
        let public_key = self.runtime_public_key.to_public_key()?;
        if public_key.suite() != self.runtime_suite {
            return Err(RecordError::SuiteMismatch);
        }
        if derive_key_id(&public_key)? != self.runtime_key_id {
            return Err(RecordError::KeyIdMismatch);
        }
        self.validate_times()?;
        self.validate_custody()?;
        Ok(())
    }

    fn validate_times(&self) -> Result<(), RecordError> {
        let lifetime = self
            .not_after_ms
            .checked_sub(self.not_before_ms)
            .ok_or(RecordError::InvalidValidity)?;
        let renewal = self
            .renew_after_ms
            .checked_sub(self.not_before_ms)
            .ok_or(RecordError::InvalidValidity)?;
        if lifetime == 0
            || lifetime > MAX_RUNTIME_LIFETIME_MS
            || self.renew_after_ms >= self.not_after_ms
        {
            return Err(RecordError::InvalidValidity);
        }
        let min = lifetime / 2;
        let max = lifetime.checked_mul(4).ok_or(RecordError::InvalidValidity)? / 5;
        if renewal < min || renewal > max {
            return Err(RecordError::InvalidValidity);
        }
        Ok(())
    }

    fn validate_custody(&self) -> Result<(), RecordError> {
        let evidence = self.evidence_profile_id.is_some()
            && self.evidence_digest.is_some()
            && self.attestation_verifier_id.is_some()
            && self.attested_at_ms.is_some();
        let absent = self.evidence_profile_id.is_none()
            && self.evidence_digest.is_none()
            && self.attestation_verifier_id.is_none()
            && self.attested_at_ms.is_none();
        match self.custody_class {
            CustodyClass::SoftwareEphemeral if absent => Ok(()),
            CustodyClass::OsProtected if evidence => Ok(()),
            CustodyClass::HardwareNonexportable if evidence && !self.degraded => Ok(()),
            _ => Err(RecordError::InvalidCustody),
        }
    }

    pub fn protected_bytes(&self) -> Result<Vec<u8>, RecordError> {
        self.validate()?;
        minicbor::to_vec(self).map_err(encode_error)
    }

    pub fn signing_input(&self) -> Result<Vec<u8>, RecordError> {
        signing_frame(RUNTIME_CERTIFICATE_DOMAIN, &self.protected_bytes()?)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub struct RuntimeCertificate {
    #[n(0)]
    pub protected: Vec<u8>,
    #[n(1)]
    pub signature: Vec<u8>,
}

impl RuntimeCertificate {
    pub fn from_claims(
        claims: &RuntimeCertificateClaims,
        signature: Vec<u8>,
    ) -> Result<Self, RecordError> {
        Ok(Self { protected: claims.protected_bytes()?, signature })
    }

    pub fn encode(&self) -> Result<Vec<u8>, RecordError> {
        let bytes = minicbor::to_vec(self).map_err(encode_error)?;
        if bytes.len() > MAX_RUNTIME_CERTIFICATE_BYTES {
            return Err(RecordError::TooLarge {
                actual: bytes.len(),
                max: MAX_RUNTIME_CERTIFICATE_BYTES,
            });
        }
        Ok(bytes)
    }

    pub fn digest(&self) -> Result<Digest32, RecordError> {
        Ok(sha256(&self.encode()?))
    }
}
