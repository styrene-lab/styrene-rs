use thiserror::Error;

use crate::{
    CapabilityDecision, CapabilityReason, CapabilityRequest, ConfirmedFirmwarePlan, ExecutorClass,
    FirmwareEvent, FirmwareOperation, FirmwareWorkflow, HostClass, PostWriteVerificationError,
    TargetObservation, WorkflowError,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MobileDfuPhase {
    ClosingNus,
    DiscoveringDfu,
    ReadyToWrite,
    Writing,
    ReconnectingNus,
    Verifying,
    Cancelled,
    Failed,
    VerificationFailed,
    Superseded,
    Succeeded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MobileDfuEffect {
    CloseNus,
    DiscoverDfu,
    BeginWrite,
    ReconnectNus,
    VerifyModelVersionHash,
    RecoveryRequired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MobileDfuApply {
    Applied(Option<MobileDfuEffect>),
    IgnoredStale,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum MobileDfuError {
    #[error("confirmed plan is not an iOS nRF52 application upgrade")]
    UnsupportedPlan,
    #[error("mobile firmware capability is not physically accepted for the confirmed target")]
    CapabilityDenied,
    #[error("mobile DFU event is invalid for the current phase")]
    InvalidPhase,
    #[error("mobile DFU progress exceeds or regresses within the admitted application")]
    InvalidProgress,
    #[error("mobile DFU cannot be cancelled after writing starts")]
    WriteAlreadyStarted,
    #[error("post-write verification failed: {0}")]
    Verification(#[from] PostWriteVerificationError),
    #[error("firmware workflow rejected the event: {0}")]
    Workflow(#[from] WorkflowError),
}

/// Policy-owned mobile lifecycle for one confirmed application-upgrade plan.
///
/// Transport implementations consume effects and report observations. They do
/// not select artifacts, decide recovery, or declare post-write success.
#[derive(Clone, Debug)]
pub struct MobileDfuWorkflow {
    confirmed: ConfirmedFirmwarePlan,
    workflow: FirmwareWorkflow,
    phase: MobileDfuPhase,
    expected_bytes: u64,
    completed_bytes: u64,
    write_started: bool,
}

impl MobileDfuWorkflow {
    /// Create a mobile workflow from an immutable confirmed plan.
    ///
    /// # Errors
    ///
    /// Returns an error unless the plan is an iOS nRF52 application upgrade
    /// with one nonempty application image.
    pub fn new(
        confirmed: ConfirmedFirmwarePlan,
        capability: &CapabilityRequest,
    ) -> Result<Self, MobileDfuError> {
        let plan = confirmed.plan();
        let mut applications = plan.image_regions.iter().filter(|image| image.application);
        let application = applications.next().filter(|image| image.region.length != 0);
        if plan.operation != FirmwareOperation::Upgrade
            || plan.executor != ExecutorClass::IosNrfBleDfu
            || !plan.executor.supports_mcu(plan.target.mcu_family)
            || plan.image_regions.len() != 1
            || applications.next().is_some()
            || application.is_none()
        {
            return Err(MobileDfuError::UnsupportedPlan);
        }
        let capability_result = capability.evaluate();
        if capability.host != HostClass::IosMobile
            || capability.operation != plan.operation
            || capability.executor != Some(plan.executor)
            || capability.target != plan.target
            || plan.target.bootloader_revision.is_none()
            || capability_result.decision != CapabilityDecision::Allow
            || capability_result.reason != CapabilityReason::AcceptedExactTarget
        {
            return Err(MobileDfuError::CapabilityDenied);
        }
        let expected_bytes = application.expect("checked application image").region.length;
        let generation = plan.target_generation;
        let mut workflow = FirmwareWorkflow::new(
            HostClass::IosMobile,
            FirmwareOperation::Upgrade,
            ExecutorClass::IosNrfBleDfu,
            generation,
        );
        workflow.apply(FirmwareEvent::Confirmed)?;
        workflow.apply(FirmwareEvent::Preparing)?;
        Ok(Self {
            confirmed,
            workflow,
            phase: MobileDfuPhase::ClosingNus,
            expected_bytes,
            completed_bytes: 0,
            write_started: false,
        })
    }

    #[must_use]
    pub const fn phase(&self) -> MobileDfuPhase {
        self.phase
    }

    #[must_use]
    pub const fn expected_bytes(&self) -> u64 {
        self.expected_bytes
    }

    #[must_use]
    pub const fn required_effect(&self) -> Option<MobileDfuEffect> {
        match self.phase {
            MobileDfuPhase::ClosingNus => Some(MobileDfuEffect::CloseNus),
            MobileDfuPhase::DiscoveringDfu => Some(MobileDfuEffect::DiscoverDfu),
            MobileDfuPhase::ReadyToWrite => Some(MobileDfuEffect::BeginWrite),
            MobileDfuPhase::ReconnectingNus => Some(MobileDfuEffect::ReconnectNus),
            MobileDfuPhase::Verifying => Some(MobileDfuEffect::VerifyModelVersionHash),
            MobileDfuPhase::Writing
            | MobileDfuPhase::Cancelled
            | MobileDfuPhase::Failed
            | MobileDfuPhase::VerificationFailed
            | MobileDfuPhase::Superseded
            | MobileDfuPhase::Succeeded => None,
        }
    }

    #[must_use]
    pub fn terminal_name(&self) -> &'static str {
        self.workflow.terminal_name()
    }

    #[must_use]
    pub const fn recovery_required(&self) -> bool {
        self.workflow.recovery_required()
    }

    #[must_use]
    pub const fn destructive_started(&self) -> bool {
        self.write_started
    }

    /// Replace the active generation without accepting callbacks from the old one.
    ///
    /// # Errors
    ///
    /// Returns an error if the shared workflow rejects the generation change.
    pub fn replace_generation(&mut self, generation: u64) -> Result<(), MobileDfuError> {
        self.workflow.apply(FirmwareEvent::GenerationReplaced(generation))?;
        if self.write_started {
            self.workflow.apply(FirmwareEvent::Interrupted)?;
            self.phase = MobileDfuPhase::Failed;
        } else {
            self.workflow.apply(FirmwareEvent::TargetChanged)?;
            self.phase = MobileDfuPhase::Superseded;
        }
        Ok(())
    }

    /// Record closure of the normal NUS session.
    ///
    /// # Errors
    ///
    /// Returns an error unless the workflow is waiting for NUS closure.
    pub fn nus_closed(&mut self, generation: u64) -> Result<MobileDfuApply, MobileDfuError> {
        if self.reject_stale(generation)? {
            return Ok(MobileDfuApply::IgnoredStale);
        }
        if self.phase != MobileDfuPhase::ClosingNus {
            return Err(MobileDfuError::InvalidPhase);
        }
        self.phase = MobileDfuPhase::DiscoveringDfu;
        Ok(MobileDfuApply::Applied(Some(MobileDfuEffect::DiscoverDfu)))
    }

    /// Record discovery of the separate DFU session.
    ///
    /// # Errors
    ///
    /// Returns an error unless NUS closed first.
    pub fn dfu_discovered(&mut self, generation: u64) -> Result<MobileDfuApply, MobileDfuError> {
        if self.reject_stale(generation)? {
            return Ok(MobileDfuApply::IgnoredStale);
        }
        if self.phase != MobileDfuPhase::DiscoveringDfu {
            return Err(MobileDfuError::InvalidPhase);
        }
        self.phase = MobileDfuPhase::ReadyToWrite;
        Ok(MobileDfuApply::Applied(Some(MobileDfuEffect::BeginWrite)))
    }

    /// Record the destructive write boundary.
    ///
    /// # Errors
    ///
    /// Returns an error unless DFU discovery completed successfully.
    pub fn write_started(&mut self, generation: u64) -> Result<MobileDfuApply, MobileDfuError> {
        if self.reject_stale(generation)? {
            return Ok(MobileDfuApply::IgnoredStale);
        }
        if self.phase != MobileDfuPhase::ReadyToWrite {
            return Err(MobileDfuError::InvalidPhase);
        }
        self.workflow.apply(FirmwareEvent::WriteStarted)?;
        self.phase = MobileDfuPhase::Writing;
        self.write_started = true;
        Ok(MobileDfuApply::Applied(None))
    }

    /// Record monotonic progress bounded by the confirmed application image.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid phase, overflow, or regression.
    pub fn progress_changed(
        &mut self,
        generation: u64,
        completed_bytes: u64,
    ) -> Result<MobileDfuApply, MobileDfuError> {
        if self.reject_stale(generation)? {
            return Ok(MobileDfuApply::IgnoredStale);
        }
        if self.phase != MobileDfuPhase::Writing
            || completed_bytes > self.expected_bytes
            || completed_bytes < self.completed_bytes
        {
            return Err(MobileDfuError::InvalidProgress);
        }
        self.completed_bytes = completed_bytes;
        Ok(MobileDfuApply::Applied(None))
    }

    /// Record transfer completion after the admitted byte count is reached.
    ///
    /// # Errors
    ///
    /// Returns an error unless the active write reached the expected byte count.
    pub fn write_completed(&mut self, generation: u64) -> Result<MobileDfuApply, MobileDfuError> {
        if self.reject_stale(generation)? {
            return Ok(MobileDfuApply::IgnoredStale);
        }
        if self.phase != MobileDfuPhase::Writing || self.completed_bytes != self.expected_bytes {
            return Err(MobileDfuError::InvalidPhase);
        }
        self.workflow.apply(FirmwareEvent::WriteCompleted)?;
        self.phase = MobileDfuPhase::ReconnectingNus;
        Ok(MobileDfuApply::Applied(Some(MobileDfuEffect::ReconnectNus)))
    }

    /// Record reconnection of NUS for authoritative verification.
    ///
    /// # Errors
    ///
    /// Returns an error unless transfer completed first.
    pub fn nus_reopened(&mut self, generation: u64) -> Result<MobileDfuApply, MobileDfuError> {
        if self.reject_stale(generation)? {
            return Ok(MobileDfuApply::IgnoredStale);
        }
        if self.phase != MobileDfuPhase::ReconnectingNus {
            return Err(MobileDfuError::InvalidPhase);
        }
        self.workflow.apply(FirmwareEvent::Reopened)?;
        self.phase = MobileDfuPhase::Verifying;
        Ok(MobileDfuApply::Applied(Some(MobileDfuEffect::VerifyModelVersionHash)))
    }

    /// Compare a reopened RNode observation with the immutable confirmed plan.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid phase or any model, version, or hash mismatch.
    pub fn verify_reopened(
        &mut self,
        generation: u64,
        observation: &TargetObservation,
    ) -> Result<MobileDfuApply, MobileDfuError> {
        if self.reject_stale(generation)? {
            return Ok(MobileDfuApply::IgnoredStale);
        }
        if self.phase != MobileDfuPhase::Verifying {
            return Err(MobileDfuError::InvalidPhase);
        }
        if let Err(error) = self.confirmed.verify_reopened(observation) {
            self.workflow.apply(FirmwareEvent::VerificationFailed)?;
            self.phase = MobileDfuPhase::VerificationFailed;
            return Err(MobileDfuError::Verification(error));
        }
        self.workflow.apply(FirmwareEvent::Verified)?;
        self.phase = MobileDfuPhase::Succeeded;
        Ok(MobileDfuApply::Applied(None))
    }

    /// Cancel before the destructive write boundary.
    ///
    /// # Errors
    ///
    /// Returns an error after writing starts or when the workflow is terminal.
    pub fn cancel(&mut self, generation: u64) -> Result<MobileDfuApply, MobileDfuError> {
        if self.reject_stale(generation)? {
            return Ok(MobileDfuApply::IgnoredStale);
        }
        if self.write_started {
            return Err(MobileDfuError::WriteAlreadyStarted);
        }
        if matches!(
            self.phase,
            MobileDfuPhase::Cancelled
                | MobileDfuPhase::Failed
                | MobileDfuPhase::VerificationFailed
                | MobileDfuPhase::Superseded
                | MobileDfuPhase::Succeeded
        ) {
            return Err(MobileDfuError::InvalidPhase);
        }
        self.workflow.apply(FirmwareEvent::Cancelled)?;
        self.phase = MobileDfuPhase::Cancelled;
        Ok(MobileDfuApply::Applied(None))
    }

    /// Record a nonterminal transport interruption.
    ///
    /// # Errors
    ///
    /// Returns an error when the workflow is already terminal.
    pub fn interrupted(&mut self, generation: u64) -> Result<MobileDfuApply, MobileDfuError> {
        if self.reject_stale(generation)? {
            return Ok(MobileDfuApply::IgnoredStale);
        }
        if matches!(
            self.phase,
            MobileDfuPhase::Cancelled
                | MobileDfuPhase::Failed
                | MobileDfuPhase::VerificationFailed
                | MobileDfuPhase::Superseded
                | MobileDfuPhase::Succeeded
        ) {
            return Err(MobileDfuError::InvalidPhase);
        }
        self.workflow.apply(FirmwareEvent::Interrupted)?;
        self.phase = MobileDfuPhase::Failed;
        Ok(MobileDfuApply::Applied(
            self.recovery_required().then_some(MobileDfuEffect::RecoveryRequired),
        ))
    }

    fn reject_stale(&mut self, generation: u64) -> Result<bool, WorkflowError> {
        if generation == self.workflow.generation() {
            return Ok(false);
        }
        self.workflow.apply(FirmwareEvent::StaleEvent(generation))?;
        Ok(true)
    }
}
