use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::{Component, Path};

use ed25519_dalek::{Signature, VerifyingKey};
use thiserror::Error;

use crate::{ExecutorClass, FirmwareOperation, MemoryRegion, Sha256Digest, TargetObservation};

const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_ARCHIVE_MEMBERS: usize = 128;
const MAX_EXPANDED_BYTES: u64 = 64 * 1024 * 1024;

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
    ManifestInvalid,
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
    ManifestInvalid,
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
        if self.layout_findings.contains(&LayoutFinding::ManifestInvalid) {
            return deny(ArtifactDecisionReason::ManifestInvalid);
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

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FirmwareManifest {
    pub schema_version: u16,
    pub manifest_id: String,
    pub firmware_version: String,
    pub operations: Vec<FirmwareOperation>,
    pub target: ManifestTarget,
    pub artifact: ManifestArtifact,
    pub images: Vec<ManifestImage>,
    pub protected_regions: Vec<MemoryRegion>,
    pub recovery: ManifestRecovery,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManifestTarget {
    pub board: String,
    pub radio_variant: String,
    pub hardware_revision: String,
    pub executor: ExecutorClass,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManifestArtifact {
    pub archive_sha256: Sha256Digest,
    pub max_expanded_bytes: u64,
    pub expected_members: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManifestImage {
    pub member: String,
    pub region: MemoryRegion,
    pub sha256: Sha256Digest,
    pub application: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManifestRecovery {
    pub executor: ExecutorClass,
    pub procedure_id: String,
    pub physical_mode: String,
    pub tool_id: String,
    pub power_condition: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedFirmwareManifest {
    /// Exact signed bytes. Parsing occurs only after signature verification.
    pub payload: Vec<u8>,
    pub signature: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
pub struct ArchiveMember<'a> {
    /// The archive parser must report every member without path normalization.
    pub path: &'a str,
    pub bytes: &'a [u8],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedArtifact {
    pub manifest: FirmwareManifest,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("firmware artifact denied: {reason:?}")]
pub struct ArtifactAdmissionError {
    reason: ArtifactDecisionReason,
}

impl ArtifactAdmissionError {
    #[must_use]
    pub const fn reason(&self) -> ArtifactDecisionReason {
        self.reason
    }
}

#[allow(clippy::too_many_arguments)]
pub fn admit_artifact(
    signed: Option<&SignedFirmwareManifest>,
    verifying_key: &[u8; 32],
    archive: &[u8],
    members: &[ArchiveMember<'_>],
    target: &TargetObservation,
    executor: ExecutorClass,
) -> Result<AdmittedArtifact, ArtifactAdmissionError> {
    let signed =
        signed.ok_or_else(|| admission_error(ArtifactDecisionReason::ManifestSignatureRequired))?;
    if signed.payload.len() > MAX_MANIFEST_BYTES {
        return Err(admission_error(ArtifactDecisionReason::ManifestInvalid));
    }
    let key = VerifyingKey::from_bytes(verifying_key)
        .map_err(|_| admission_error(ArtifactDecisionReason::ManifestSignatureInvalid))?;
    let signature = Signature::from_slice(&signed.signature)
        .map_err(|_| admission_error(ArtifactDecisionReason::ManifestSignatureInvalid))?;
    key.verify_strict(&signed.payload, &signature)
        .map_err(|_| admission_error(ArtifactDecisionReason::ManifestSignatureInvalid))?;
    let manifest: FirmwareManifest = serde_json::from_slice(&signed.payload)
        .map_err(|_| admission_error(ArtifactDecisionReason::ManifestInvalid))?;
    validate_manifest(&manifest)?;

    if manifest.target.executor != executor
        || !executor.supports_mcu(target.mcu_family)
        || !manifest.recovery.executor.supports_mcu(target.mcu_family)
        || target.board.as_deref() != Some(manifest.target.board.as_str())
        || target.radio_variant.as_deref() != Some(manifest.target.radio_variant.as_str())
        || target.hardware_revision.as_deref() != Some(manifest.target.hardware_revision.as_str())
    {
        return Err(admission_error(ArtifactDecisionReason::TargetMismatch));
    }
    if archive.len() as u64 > manifest.artifact.max_expanded_bytes {
        return Err(admission_error(ArtifactDecisionReason::UnsafeArchive));
    }
    if Sha256Digest::from_bytes(Sha256::digest(archive).into()) != manifest.artifact.archive_sha256
    {
        return Err(admission_error(ArtifactDecisionReason::ArchiveDigestMismatch));
    }
    validate_members(&manifest, members)?;
    validate_layout(&manifest)?;
    Ok(AdmittedArtifact { manifest })
}

fn validate_manifest(manifest: &FirmwareManifest) -> Result<(), ArtifactAdmissionError> {
    if manifest.schema_version != 1
        || manifest.manifest_id.is_empty()
        || manifest.firmware_version.is_empty()
        || manifest.operations.is_empty()
        || manifest.target.board.is_empty()
        || manifest.target.radio_variant.is_empty()
        || manifest.target.hardware_revision.is_empty()
        || manifest.artifact.max_expanded_bytes == 0
        || manifest.artifact.max_expanded_bytes > MAX_EXPANDED_BYTES
        || manifest.artifact.expected_members.is_empty()
        || manifest.artifact.expected_members.len() > MAX_ARCHIVE_MEMBERS
    {
        return Err(admission_error(ArtifactDecisionReason::ManifestInvalid));
    }
    let mut operations = HashSet::new();
    if manifest.operations.iter().any(|operation| {
        matches!(operation, FirmwareOperation::Inspect | FirmwareOperation::Plan)
            || !operations.insert(*operation)
    }) || manifest.recovery.procedure_id.is_empty()
        || manifest.recovery.physical_mode.is_empty()
        || manifest.recovery.tool_id.is_empty()
        || manifest.recovery.power_condition.is_empty()
    {
        return Err(admission_error(ArtifactDecisionReason::ManifestInvalid));
    }
    let mut expected = HashSet::new();
    for member in &manifest.artifact.expected_members {
        if !safe_relative_path(member) || !expected.insert(member.as_str()) {
            return Err(admission_error(ArtifactDecisionReason::ManifestInvalid));
        }
    }
    let mut image_members = HashSet::new();
    if manifest.images.iter().any(|image| {
        !expected.contains(image.member.as_str())
            || !image_members.insert(image.member.as_str())
            || image.region.length == 0
            || image.region.offset.checked_add(image.region.length).is_none()
            || !safe_relative_path(&image.member)
    }) || manifest
        .protected_regions
        .iter()
        .any(|region| region.length == 0 || region.offset.checked_add(region.length).is_none())
    {
        return Err(admission_error(ArtifactDecisionReason::ManifestInvalid));
    }
    if manifest.images.iter().filter(|image| image.application).count() != 1 {
        return Err(admission_error(ArtifactDecisionReason::ApplicationDigestRequired));
    }
    Ok(())
}

fn validate_members(
    manifest: &FirmwareManifest,
    members: &[ArchiveMember<'_>],
) -> Result<(), ArtifactAdmissionError> {
    if members.len() > MAX_ARCHIVE_MEMBERS {
        return Err(admission_error(ArtifactDecisionReason::UnsafeArchive));
    }
    let expected =
        manifest.artifact.expected_members.iter().map(String::as_str).collect::<HashSet<_>>();
    let mut observed = HashSet::new();
    let mut expanded = 0_u64;
    for member in members {
        if !safe_relative_path(member.path)
            || !observed.insert(member.path)
            || !expected.contains(member.path)
        {
            return Err(admission_error(ArtifactDecisionReason::UnsafeArchive));
        }
        expanded = expanded
            .checked_add(member.bytes.len() as u64)
            .ok_or_else(|| admission_error(ArtifactDecisionReason::UnsafeArchive))?;
        if expanded > manifest.artifact.max_expanded_bytes {
            return Err(admission_error(ArtifactDecisionReason::UnsafeArchive));
        }
    }
    if observed != expected {
        return Err(admission_error(ArtifactDecisionReason::UnsafeArchive));
    }
    for image in &manifest.images {
        let member = members
            .iter()
            .find(|member| member.path == image.member)
            .ok_or_else(|| admission_error(ArtifactDecisionReason::UnsafeArchive))?;
        if member.bytes.len() as u64 != image.region.length
            || Sha256Digest::from_bytes(Sha256::digest(member.bytes).into()) != image.sha256
        {
            return Err(admission_error(ArtifactDecisionReason::UnsafeLayout));
        }
    }
    Ok(())
}

fn validate_layout(manifest: &FirmwareManifest) -> Result<(), ArtifactAdmissionError> {
    for (index, image) in manifest.images.iter().enumerate() {
        if manifest.images[index + 1..]
            .iter()
            .any(|other| regions_overlap(&image.region, &other.region))
            || manifest
                .protected_regions
                .iter()
                .any(|protected| regions_overlap(&image.region, protected))
        {
            return Err(admission_error(ArtifactDecisionReason::UnsafeLayout));
        }
    }
    Ok(())
}

fn regions_overlap(left: &MemoryRegion, right: &MemoryRegion) -> bool {
    let Some(left_end) = left.offset.checked_add(left.length) else { return true };
    let Some(right_end) = right.offset.checked_add(right.length) else { return true };
    left.offset < right_end && right.offset < left_end
}

fn safe_relative_path(value: &str) -> bool {
    !value.is_empty()
        && Path::new(value).components().all(|component| matches!(component, Component::Normal(_)))
}

const fn admission_error(reason: ArtifactDecisionReason) -> ArtifactAdmissionError {
    ArtifactAdmissionError { reason }
}
