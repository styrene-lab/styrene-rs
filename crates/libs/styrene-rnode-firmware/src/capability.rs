use serde::{Deserialize, Serialize};

use crate::{
    ConfigurationState, ExecutorClass, FirmwareOperation, HostClass, McuFamily, TargetObservation,
};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityDecision {
    Allow,
    Deny,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityReason {
    AcceptedExactTarget,
    ReadOnlyInspection,
    ExactTargetUnknown,
    MobileExecutorUnavailable,
    OperationNotMobileSupported,
    PhysicalEvidenceMissing,
    ExecutorMismatch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityRequest {
    pub host: HostClass,
    pub operation: FirmwareOperation,
    pub executor: Option<ExecutorClass>,
    pub target: TargetObservation,
    pub physical_acceptance: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapabilityResult {
    pub decision: CapabilityDecision,
    pub reason: CapabilityReason,
}

impl CapabilityRequest {
    #[must_use]
    pub fn evaluate(&self) -> CapabilityResult {
        if self.operation == FirmwareOperation::Inspect {
            let expected = match self.host {
                HostClass::Desktop => ExecutorClass::ReadOnlySerial,
                HostClass::IosMobile => ExecutorClass::BleNusInspect,
            };
            return if self.executor == Some(expected) {
                allow(CapabilityReason::ReadOnlyInspection)
            } else {
                deny(CapabilityReason::ExecutorMismatch)
            };
        }
        if !self.target.has_exact_hardware() {
            return deny(CapabilityReason::ExactTargetUnknown);
        }
        if self.host == HostClass::IosMobile {
            if self.target.mcu_family != McuFamily::Nrf52840 {
                return deny(CapabilityReason::MobileExecutorUnavailable);
            }
            if self.operation == FirmwareOperation::FreshInstall && !self.physical_acceptance {
                return deny(CapabilityReason::PhysicalEvidenceMissing);
            }
            if self.operation != FirmwareOperation::Upgrade {
                return deny(CapabilityReason::OperationNotMobileSupported);
            }
            if !self.physical_acceptance {
                return deny(CapabilityReason::PhysicalEvidenceMissing);
            }
            if self.executor != Some(ExecutorClass::IosNrfBleDfu)
                || self.target.configuration != ConfigurationState::Yes
            {
                return deny(CapabilityReason::MobileExecutorUnavailable);
            }
            return allow(CapabilityReason::AcceptedExactTarget);
        }
        if !self.physical_acceptance {
            if !self.executor.is_some_and(|executor| executor.supports_mcu(self.target.mcu_family))
            {
                return deny(CapabilityReason::ExecutorMismatch);
            }
            return deny(CapabilityReason::PhysicalEvidenceMissing);
        }
        if !self.executor.is_some_and(|executor| executor.supports_mcu(self.target.mcu_family)) {
            return deny(CapabilityReason::ExecutorMismatch);
        }
        allow(CapabilityReason::AcceptedExactTarget)
    }
}

const fn allow(reason: CapabilityReason) -> CapabilityResult {
    CapabilityResult { decision: CapabilityDecision::Allow, reason }
}

const fn deny(reason: CapabilityReason) -> CapabilityResult {
    CapabilityResult { decision: CapabilityDecision::Deny, reason }
}
