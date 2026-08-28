use minicbor::{Decode, Encode};

use super::runtime_certificate::SubjectKind;
use super::{
    Digest32, Id16, LIFECYCLE_TRANSITION_DOMAIN, MAX_LIFECYCLE_TRANSITION_BYTES,
    RECORD_PROFILE_VERSION, RecordError, SignatureSuite, encode_error, sha256, signing_frame,
    validate_ascii_identifier,
};

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum LifecycleDomainKey {
    #[n(0)]
    OwnerState(#[n(0)] String),
    #[n(1)]
    Authority(#[n(0)] String, #[n(1)] Digest32),
    #[n(2)]
    RuntimeCertificate(#[n(0)] Digest32, #[n(1)] SubjectKind, #[n(2)] String),
    #[n(3)]
    Revocation(#[n(0)] Digest32),
    #[n(4)]
    EnrollmentNonce(#[n(0)] Digest32),
    #[n(5)]
    ReplicaQuota(#[n(0)] Digest32, #[n(1)] Digest32),
    #[n(6)]
    RecoveryPolicy(#[n(0)] String, #[n(1)] u64),
    #[n(7)]
    ApiGrant(#[n(0)] Digest32, #[n(1)] Digest32),
    #[n(8)]
    AuditPurge(#[n(0)] String),
    #[n(9)]
    KeyFamily(#[n(0)] Digest32),
}

impl LifecycleDomainKey {
    fn validate(&self) -> Result<(), RecordError> {
        match self {
            Self::OwnerState(domain) | Self::AuditPurge(domain) => {
                validate_ascii_identifier(domain, 255)
            }
            Self::Authority(domain, _) | Self::RecoveryPolicy(domain, _) => {
                validate_ascii_identifier(domain, 255)
            }
            Self::RuntimeCertificate(_, _, subject) => validate_ascii_identifier(subject, 255),
            _ => Ok(()),
        }
        .map_err(|_| RecordError::InvalidDomainKey)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(index_only)]
pub enum TransitionKind {
    #[n(0)]
    Bootstrap,
    #[n(1)]
    Issue,
    #[n(2)]
    Activate,
    #[n(3)]
    Retire,
    #[n(4)]
    Suspend,
    #[n(5)]
    Reinstate,
    #[n(6)]
    RevokeAdministrative,
    #[n(7)]
    RevokeCompromise,
    #[n(8)]
    Consume,
    #[n(9)]
    Tombstone,
    #[n(10)]
    Reconcile,
    #[n(11)]
    Grant,
    #[n(12)]
    PurgeAuthorize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(index_only)]
pub enum TargetKind {
    #[n(0)]
    RuntimeCertificate,
    #[n(1)]
    KeyVersion,
    #[n(2)]
    Authority,
    #[n(3)]
    OwnerState,
    #[n(4)]
    EnrollmentChallenge,
    #[n(5)]
    ReplicaLease,
    #[n(6)]
    RecoveryPolicy,
    #[n(7)]
    ApiGrant,
    #[n(8)]
    AuditPurgeProposal,
    #[n(9)]
    LifecycleTransition,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(index_only)]
pub enum ReasonCode {
    #[n(0)]
    Unspecified,
    #[n(1)]
    RoutineRenewal,
    #[n(2)]
    OperatorRequest,
    #[n(3)]
    PolicyViolation,
    #[n(4)]
    KeyCompromise,
    #[n(5)]
    ProviderOrHostCompromise,
    #[n(6)]
    AuthorityCompromise,
    #[n(7)]
    AttestationInvalidation,
    #[n(8)]
    LostCustody,
    #[n(9)]
    Superseded,
    #[n(10)]
    ProviderOperationFailed,
    #[n(11)]
    RecoveryAction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(index_only)]
pub enum DescendantScope {
    #[n(0)]
    TargetOnly,
    #[n(1)]
    KeyVersionAndCertificates,
    #[n(2)]
    AuthorityDescendants,
    #[n(3)]
    ProviderOrHostBoundary,
    #[n(4)]
    TrustDomain,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct Reconciliation {
    #[n(0)]
    pub fork_predecessor_digest: Digest32,
    #[n(1)]
    pub winning_head_digest: Digest32,
    #[n(2)]
    pub rejected_head_digests: Vec<Digest32>,
    #[n(3)]
    pub superior_domain_key: LifecycleDomainKey,
    #[n(4)]
    pub superior_transition_digest: Digest32,
}

impl Reconciliation {
    pub(crate) fn validate(&self) -> Result<(), RecordError> {
        if self.rejected_head_digests.is_empty() || self.rejected_head_digests.len() > 64 {
            return Err(RecordError::InvalidReconciliation);
        }
        if self.rejected_head_digests.iter().any(|digest| digest == &self.winning_head_digest)
            || self.rejected_head_digests.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(RecordError::InvalidReconciliation);
        }
        self.superior_domain_key.validate()?;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct LifecycleTransitionClaims {
    #[n(0)]
    pub profile_version: u16,
    #[n(1)]
    pub transition_id: Id16,
    #[n(2)]
    pub domain_key: LifecycleDomainKey,
    #[n(3)]
    pub revision: u64,
    #[n(4)]
    pub previous_transition_digest: Option<Digest32>,
    #[n(5)]
    pub transition_kind: TransitionKind,
    #[n(6)]
    pub target_kind: TargetKind,
    #[n(7)]
    pub target_id: Digest32,
    #[n(8)]
    pub effective_at_ms: u64,
    #[n(9)]
    pub reason_code: ReasonCode,
    #[n(10)]
    pub issuer_key_id: Digest32,
    #[n(11)]
    pub issuer_key_version: u64,
    #[n(12)]
    pub issuer_suite: SignatureSuite,
    #[n(13)]
    pub replacement_digest: Option<Digest32>,
    #[n(14)]
    pub compromise_not_before_ms: Option<u64>,
    #[n(15)]
    pub descendant_scope: DescendantScope,
    #[n(16)]
    pub reconciliation: Option<Reconciliation>,
    #[n(17)]
    pub extensions_digest: Option<Digest32>,
}

impl LifecycleTransitionClaims {
    pub fn validate(&self) -> Result<(), RecordError> {
        if self.profile_version != RECORD_PROFILE_VERSION {
            return Err(RecordError::UnsupportedVersion(self.profile_version));
        }
        self.domain_key.validate()?;
        let bootstrap = self.transition_kind == TransitionKind::Bootstrap;
        if bootstrap != (self.revision == 0 && self.previous_transition_digest.is_none()) {
            return Err(RecordError::InvalidRevision);
        }
        if !bootstrap && (self.revision == 0 || self.previous_transition_digest.is_none()) {
            return Err(RecordError::InvalidRevision);
        }
        let compromise = self.transition_kind == TransitionKind::RevokeCompromise;
        if !compromise && self.compromise_not_before_ms.is_some() {
            return Err(RecordError::InvalidTransition);
        }
        if let Some(start) = self.compromise_not_before_ms
            && start > self.effective_at_ms
        {
            return Err(RecordError::InvalidTransition);
        }
        if compromise && self.descendant_scope == DescendantScope::TargetOnly {
            return Err(RecordError::InvalidTransition);
        }
        if !compromise && self.descendant_scope != DescendantScope::TargetOnly {
            return Err(RecordError::InvalidTransition);
        }
        match (&self.transition_kind, &self.reconciliation) {
            (TransitionKind::Reconcile, Some(value)) => value.validate()?,
            (TransitionKind::Reconcile, None) | (_, Some(_)) => {
                return Err(RecordError::InvalidReconciliation);
            }
            _ => {}
        }
        Ok(())
    }

    pub fn protected_bytes(&self) -> Result<Vec<u8>, RecordError> {
        self.validate()?;
        minicbor::to_vec(self).map_err(encode_error)
    }

    pub fn signing_input(&self) -> Result<Vec<u8>, RecordError> {
        signing_frame(LIFECYCLE_TRANSITION_DOMAIN, &self.protected_bytes()?)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub struct LifecycleTransition {
    #[n(0)]
    pub protected: Vec<u8>,
    #[n(1)]
    pub signature: Vec<u8>,
}

impl LifecycleTransition {
    pub fn from_claims(
        claims: &LifecycleTransitionClaims,
        signature: Vec<u8>,
    ) -> Result<Self, RecordError> {
        Ok(Self { protected: claims.protected_bytes()?, signature })
    }

    pub fn encode(&self) -> Result<Vec<u8>, RecordError> {
        let bytes = minicbor::to_vec(self).map_err(encode_error)?;
        if bytes.len() > MAX_LIFECYCLE_TRANSITION_BYTES {
            return Err(RecordError::TooLarge {
                actual: bytes.len(),
                max: MAX_LIFECYCLE_TRANSITION_BYTES,
            });
        }
        Ok(bytes)
    }

    pub fn digest(&self) -> Result<Digest32, RecordError> {
        Ok(sha256(&self.encode()?))
    }
}
