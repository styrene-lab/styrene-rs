use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FirmwareOperation {
    Inspect,
    Plan,
    Upgrade,
    FreshInstall,
    Provision,
    Recovery,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HostClass {
    Desktop,
    IosMobile,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorClass {
    ReadOnlySerial,
    BleNusInspect,
    HostSerialEsp,
    HostSerialAvr,
    HostSerialNrfDfu,
    IosNrfBleDfu,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McuFamily {
    Esp32,
    Esp32s3,
    Avr1284p,
    AvrMega2560,
    Nrf52840,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConfigurationState {
    Yes,
    No,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct TargetObservation {
    pub generation: u64,
    pub platform_code: Option<u8>,
    pub mcu_code: Option<u8>,
    pub board_code: Option<u8>,
    pub product_code: Option<u8>,
    pub model_code: Option<u8>,
    pub hardware_revision_code: Option<u8>,
    pub mcu_family: McuFamily,
    pub board: Option<String>,
    pub radio_variant: Option<String>,
    pub hardware_revision: Option<String>,
    pub bootloader: Option<String>,
    pub bootloader_revision: Option<String>,
    pub configuration: ConfigurationState,
    pub firmware_version: Option<String>,
    pub running_application_hash: Option<Sha256Digest>,
    pub target_application_hash: Option<Sha256Digest>,
}

impl TargetObservation {
    #[must_use]
    pub fn new(mcu_family: McuFamily, configuration: ConfigurationState) -> Self {
        Self { mcu_family, configuration, ..Self::default() }
    }

    #[must_use]
    pub fn with_hardware(
        mut self,
        board: Option<String>,
        radio_variant: Option<String>,
        hardware_revision: Option<String>,
        bootloader: Option<String>,
    ) -> Self {
        self.board = board;
        self.radio_variant = radio_variant;
        self.hardware_revision = hardware_revision;
        self.bootloader = bootloader;
        self
    }

    #[must_use]
    pub fn has_exact_hardware(&self) -> bool {
        self.board.is_some()
            && self.radio_variant.is_some()
            && self.hardware_revision.is_some()
            && self.bootloader.is_some()
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum DigestError {
    #[error("SHA-256 digest must contain exactly 64 lowercase hexadecimal characters")]
    InvalidSha256,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct Sha256Digest(String);

impl Sha256Digest {
    pub fn new(value: impl Into<String>) -> Result<Self, DigestError> {
        let value = value.into();
        if value.len() != 64
            || !value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(DigestError::InvalidSha256);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        let value = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        Self(value)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Sha256Digest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for Sha256Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ArtifactIdentity {
    pub manifest_entry: String,
    pub archive_sha256: Sha256Digest,
    pub application_sha256: Sha256Digest,
    pub firmware_version: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct MemoryRegion {
    pub offset: u64,
    pub length: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ImageRegion {
    pub name: String,
    pub region: MemoryRegion,
    pub sha256: Sha256Digest,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ExpectedDeviceState {
    pub board: String,
    pub radio_variant: String,
    pub hardware_revision: String,
    pub firmware_version: String,
    pub running_application_hash: Sha256Digest,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RecoveryPolicy {
    pub executor: ExecutorClass,
    pub procedure_id: String,
    pub requires_new_confirmation: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct FirmwarePlan {
    pub schema_version: u16,
    pub operation: FirmwareOperation,
    pub target_generation: u64,
    pub target: TargetObservation,
    pub artifact: ArtifactIdentity,
    pub executor: ExecutorClass,
    pub image_regions: Vec<ImageRegion>,
    pub preserved_regions: Vec<MemoryRegion>,
    pub recovery: RecoveryPolicy,
    pub expected: ExpectedDeviceState,
}

impl FirmwarePlan {
    pub fn digest(&self) -> Result<PlanDigest, PlanError> {
        let encoded = serde_json::to_vec(self).map_err(PlanError::Serialize)?;
        let digest = Sha256::digest(encoded);
        let value = digest.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
        Ok(PlanDigest(Sha256Digest::new(value).map_err(PlanError::Digest)?))
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct PlanDigest(Sha256Digest);

impl PlanDigest {
    pub fn new(value: impl Into<String>) -> Result<Self, DigestError> {
        Sha256Digest::new(value).map(Self)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl<'de> Deserialize<'de> for PlanDigest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Sha256Digest::deserialize(deserializer).map(Self)
    }
}

impl fmt::Display for PlanDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Error)]
pub enum PlanError {
    #[error("failed to serialize firmware plan: {0}")]
    Serialize(serde_json::Error),
    #[error(transparent)]
    Digest(DigestError),
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FirmwarePhase {
    Inspecting,
    Planned,
    Confirmed,
    Preparing,
    Writing,
    Restarting,
    Verifying,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ProgressError {
    #[error("completed bytes exceed total bytes")]
    CompletedExceedsTotal,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct FirmwareProgress {
    pub phase: FirmwarePhase,
    pub completed_bytes: u64,
    pub total_bytes: u64,
}

impl FirmwareProgress {
    pub fn new(
        phase: FirmwarePhase,
        completed_bytes: u64,
        total_bytes: u64,
    ) -> Result<Self, ProgressError> {
        if completed_bytes > total_bytes {
            return Err(ProgressError::CompletedExceedsTotal);
        }
        Ok(Self { phase, completed_bytes, total_bytes })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceScope {
    SyntheticContract,
    PhysicalHardware,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct FirmwareEvidence {
    pub scope: EvidenceScope,
    pub application_revision: String,
    pub manifest_revision: String,
    pub upstream_revision: String,
    pub artifact_sha256: Sha256Digest,
    pub executor_version: String,
    pub target_class: String,
    pub bootloader_revision: String,
    pub final_application_hash: Sha256Digest,
}
