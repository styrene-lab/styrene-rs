use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{ExecutorClass, FirmwareOperation, HostClass};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FirmwareEvent {
    Inspected,
    ArtifactAdmitted,
    PlanCreated,
    Confirmed,
    ConfirmationRejected,
    TargetChanged,
    Preparing,
    WriteStarted,
    WriteCompleted,
    Reopened,
    Verified,
    VerificationFailed,
    Cancelled,
    Interrupted,
    GenerationReplaced(u64),
    StaleEvent(u64),
    ProvisioningIncomplete,
    RecoveryRequired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorkflowStatus {
    New,
    Inspected,
    ArtifactAdmitted,
    Planned,
    Confirmed,
    Preparing,
    Writing,
    Written,
    Verifying,
    Rejected,
    Cancelled,
    Failed,
    VerificationFailed,
    Succeeded,
    ProvisioningIncomplete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EventDisposition {
    Applied,
    StaleRejected,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum WorkflowError {
    #[error("verification cannot succeed before a destructive write and reopen")]
    PrematureVerification,
    #[error("event generation is current and cannot be rejected as stale")]
    CurrentEventMarkedStale,
}

#[derive(Clone, Debug)]
pub struct FirmwareWorkflow {
    host: HostClass,
    operation: FirmwareOperation,
    executor: ExecutorClass,
    generation: u64,
    status: WorkflowStatus,
    last_event: EventDisposition,
    destructive_started: bool,
    write_completed: bool,
    recovery_required: bool,
}

impl FirmwareWorkflow {
    #[must_use]
    pub fn new(
        host: HostClass,
        operation: FirmwareOperation,
        executor: ExecutorClass,
        generation: u64,
    ) -> Self {
        Self {
            host,
            operation,
            executor,
            generation,
            status: WorkflowStatus::New,
            last_event: EventDisposition::Applied,
            destructive_started: false,
            write_completed: false,
            recovery_required: false,
        }
    }

    pub fn apply(&mut self, event: FirmwareEvent) -> Result<(), WorkflowError> {
        self.last_event = EventDisposition::Applied;
        match event {
            FirmwareEvent::Inspected => self.status = WorkflowStatus::Inspected,
            FirmwareEvent::ArtifactAdmitted => self.status = WorkflowStatus::ArtifactAdmitted,
            FirmwareEvent::PlanCreated => self.status = WorkflowStatus::Planned,
            FirmwareEvent::Confirmed => self.status = WorkflowStatus::Confirmed,
            FirmwareEvent::ConfirmationRejected | FirmwareEvent::TargetChanged => {
                self.status = WorkflowStatus::Rejected;
            }
            FirmwareEvent::Preparing => self.status = WorkflowStatus::Preparing,
            FirmwareEvent::WriteStarted => {
                self.destructive_started = true;
                self.status = WorkflowStatus::Writing;
            }
            FirmwareEvent::WriteCompleted => {
                self.write_completed = true;
                self.status = WorkflowStatus::Written;
            }
            FirmwareEvent::Reopened => {
                if self.status != WorkflowStatus::ProvisioningIncomplete {
                    self.status = WorkflowStatus::Verifying;
                }
            }
            FirmwareEvent::Verified => {
                if !self.destructive_started
                    || !self.write_completed
                    || self.status != WorkflowStatus::Verifying
                {
                    return Err(WorkflowError::PrematureVerification);
                }
                self.status = WorkflowStatus::Succeeded;
                self.recovery_required = false;
            }
            FirmwareEvent::VerificationFailed => {
                self.status = WorkflowStatus::VerificationFailed;
                self.recovery_required = self.destructive_started;
            }
            FirmwareEvent::Cancelled => {
                self.status = WorkflowStatus::Cancelled;
                self.recovery_required = self.destructive_started;
            }
            FirmwareEvent::Interrupted => {
                self.status = WorkflowStatus::Failed;
                self.recovery_required = self.destructive_started;
            }
            FirmwareEvent::GenerationReplaced(generation) => self.generation = generation,
            FirmwareEvent::StaleEvent(generation) => {
                if generation == self.generation {
                    return Err(WorkflowError::CurrentEventMarkedStale);
                }
                self.last_event = EventDisposition::StaleRejected;
            }
            FirmwareEvent::ProvisioningIncomplete => {
                self.status = WorkflowStatus::ProvisioningIncomplete;
                self.recovery_required = self.destructive_started;
            }
            FirmwareEvent::RecoveryRequired => {
                self.status = WorkflowStatus::Failed;
                self.recovery_required = true;
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn terminal_name(&self) -> &'static str {
        if self.last_event == EventDisposition::StaleRejected {
            return "stale_event_rejected";
        }
        match self.status {
            WorkflowStatus::New => "new",
            WorkflowStatus::Inspected => "inspected",
            WorkflowStatus::ArtifactAdmitted => "artifact_admitted",
            WorkflowStatus::Planned => "planned",
            WorkflowStatus::Confirmed => "confirmed",
            WorkflowStatus::Preparing => "preparing",
            WorkflowStatus::Writing => "writing",
            WorkflowStatus::Written => "written",
            WorkflowStatus::Verifying => "verifying",
            WorkflowStatus::Rejected => "rejected",
            WorkflowStatus::Cancelled => "cancelled",
            WorkflowStatus::Failed => "failed",
            WorkflowStatus::VerificationFailed => "verification_failed",
            WorkflowStatus::Succeeded => "succeeded",
            WorkflowStatus::ProvisioningIncomplete => "provisioning_incomplete",
        }
    }

    #[must_use]
    pub const fn recovery_required(&self) -> bool {
        self.recovery_required
    }

    #[must_use]
    pub const fn host(&self) -> HostClass {
        self.host
    }

    #[must_use]
    pub const fn operation(&self) -> FirmwareOperation {
        self.operation
    }

    #[must_use]
    pub const fn executor(&self) -> ExecutorClass {
        self.executor
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
}
