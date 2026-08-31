use thiserror::Error;

use crate::{
    AdmittedArtifact, ArtifactIdentity, ExpectedDeviceState, FirmwareOperation, FirmwarePlan,
    ImageRegion, RecoveryPolicy, TargetObservation,
};

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum PlanConstructionError {
    #[error("firmware operation is not admitted by the manifest")]
    OperationNotAdmitted,
    #[error("firmware target no longer matches the admitted manifest")]
    TargetMismatch,
    #[error("firmware manifest has no unambiguous application image")]
    ApplicationImageUnavailable,
    #[error("derived firmware plan is unsafe: {0}")]
    UnsafePlan(crate::PlanValidationError),
}

impl AdmittedArtifact {
    pub fn dry_run_plan(
        &self,
        operation: FirmwareOperation,
        target: TargetObservation,
    ) -> Result<FirmwarePlan, PlanConstructionError> {
        if !self.manifest.operations.contains(&operation) {
            return Err(PlanConstructionError::OperationNotAdmitted);
        }
        if target.board.as_deref() != Some(self.manifest.target.board.as_str())
            || target.radio_variant.as_deref() != Some(self.manifest.target.radio_variant.as_str())
            || target.hardware_revision.as_deref()
                != Some(self.manifest.target.hardware_revision.as_str())
            || !self.manifest.target.executor.supports_mcu(target.mcu_family)
        {
            return Err(PlanConstructionError::TargetMismatch);
        }
        let mut applications = self.manifest.images.iter().filter(|image| image.application);
        let application = applications
            .next()
            .filter(|_| applications.next().is_none())
            .ok_or(PlanConstructionError::ApplicationImageUnavailable)?;
        let plan = FirmwarePlan {
            schema_version: 1,
            operation,
            target_generation: target.generation,
            target,
            artifact: ArtifactIdentity {
                manifest_entry: self.manifest.manifest_id.clone(),
                archive_sha256: self.manifest.artifact.archive_sha256.clone(),
                application_sha256: application.sha256.clone(),
                firmware_version: self.manifest.firmware_version.clone(),
            },
            executor: self.manifest.target.executor,
            image_regions: self
                .manifest
                .images
                .iter()
                .map(|image| ImageRegion {
                    name: image.member.clone(),
                    region: image.region.clone(),
                    sha256: image.sha256.clone(),
                    application: image.application,
                })
                .collect(),
            preserved_regions: self.manifest.protected_regions.clone(),
            recovery: RecoveryPolicy {
                executor: self.manifest.recovery.executor,
                procedure_id: self.manifest.recovery.procedure_id.clone(),
                physical_mode: self.manifest.recovery.physical_mode.clone(),
                tool_id: self.manifest.recovery.tool_id.clone(),
                power_condition: self.manifest.recovery.power_condition.clone(),
                requires_new_confirmation: true,
            },
            expected: ExpectedDeviceState {
                board: self.manifest.target.board.clone(),
                radio_variant: self.manifest.target.radio_variant.clone(),
                hardware_revision: self.manifest.target.hardware_revision.clone(),
                firmware_version: self.manifest.firmware_version.clone(),
                running_application_hash: application.sha256.clone(),
            },
        };
        plan.validate().map_err(PlanConstructionError::UnsafePlan)?;
        Ok(plan)
    }
}
