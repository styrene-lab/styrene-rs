use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManifestSignatureState {
    Valid,
    Invalid,
    Absent,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveDigestMatch {
    Match,
    Mismatch,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TargetMatch {
    Exact,
    ModelMismatch,
    RadioMismatch,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveFinding {
    PathTraversal,
    DuplicateMember,
    UnexpectedMember,
    ExpandedSizeExceeded,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LayoutFinding {
    ImageOverlap,
    ProtectedRegionOverlap,
    ApplicationDigestMissing,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactDecision {
    Allow,
    Deny,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactDecisionReason {
    ArtifactAdmitted,
    ManifestSignatureRequired,
    ManifestSignatureInvalid,
    ArchiveDigestMismatch,
    TargetMismatch,
    UnsafeArchive,
    UnsafeLayout,
    ApplicationDigestRequired,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactAdmissionFacts {
    pub manifest_signature: ManifestSignatureState,
    pub archive_digest: ArchiveDigestMatch,
    pub target_match: TargetMatch,
    pub archive_findings: Vec<ArchiveFinding>,
    pub layout_findings: Vec<LayoutFinding>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArtifactAdmissionResult {
    pub decision: ArtifactDecision,
    pub reason: ArtifactDecisionReason,
}

impl ArtifactAdmissionFacts {
    #[must_use]
    pub fn evaluate(&self) -> ArtifactAdmissionResult {
        if self.manifest_signature == ManifestSignatureState::Absent {
            return deny(ArtifactDecisionReason::ManifestSignatureRequired);
        }
        if self.manifest_signature == ManifestSignatureState::Invalid {
            return deny(ArtifactDecisionReason::ManifestSignatureInvalid);
        }
        if self.archive_digest == ArchiveDigestMatch::Mismatch {
            return deny(ArtifactDecisionReason::ArchiveDigestMismatch);
        }
        if self.target_match != TargetMatch::Exact {
            return deny(ArtifactDecisionReason::TargetMismatch);
        }
        if !self.archive_findings.is_empty() {
            return deny(ArtifactDecisionReason::UnsafeArchive);
        }
        if self.layout_findings.contains(&LayoutFinding::ApplicationDigestMissing) {
            return deny(ArtifactDecisionReason::ApplicationDigestRequired);
        }
        if !self.layout_findings.is_empty() {
            return deny(ArtifactDecisionReason::UnsafeLayout);
        }
        ArtifactAdmissionResult {
            decision: ArtifactDecision::Allow,
            reason: ArtifactDecisionReason::ArtifactAdmitted,
        }
    }
}

const fn deny(reason: ArtifactDecisionReason) -> ArtifactAdmissionResult {
    ArtifactAdmissionResult { decision: ArtifactDecision::Deny, reason }
}
