use super::config::SdkConfig;
use crate::capability::EffectiveLimits;
use crate::error::{code, ErrorCategory, SdkError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct StartRequest {
    pub supported_contract_versions: Vec<u16>,
    pub requested_capabilities: Vec<String>,
    pub config: SdkConfig,
}

impl StartRequest {
    pub fn validate(&self) -> Result<(), SdkError> {
        if self.supported_contract_versions.is_empty() {
            return Err(SdkError::new(
                code::CAPABILITY_CONTRACT_INCOMPATIBLE,
                ErrorCategory::Capability,
                "supported_contract_versions must not be empty",
            )
            .with_user_actionable(true));
        }

        let mut seen_versions = BTreeSet::new();
        for version in &self.supported_contract_versions {
            if !seen_versions.insert(*version) {
                return Err(SdkError::new(
                    code::VALIDATION_INVALID_ARGUMENT,
                    ErrorCategory::Validation,
                    "supported_contract_versions must be unique",
                )
                .with_user_actionable(true));
            }
        }

        let mut seen_caps = BTreeSet::new();
        for capability in &self.requested_capabilities {
            let trimmed = capability.trim();
            if trimmed.is_empty() {
                return Err(SdkError::new(
                    code::VALIDATION_INVALID_ARGUMENT,
                    ErrorCategory::Validation,
                    "requested capability IDs must not be empty",
                )
                .with_user_actionable(true));
            }
            if !seen_caps.insert(trimmed.to_owned()) {
                return Err(SdkError::new(
                    code::VALIDATION_INVALID_ARGUMENT,
                    ErrorCategory::Validation,
                    "requested capability IDs must be unique",
                )
                .with_user_actionable(true));
            }
        }

        self.config.validate()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct ClientHandle {
    pub runtime_id: String,
    pub active_contract_version: u16,
    pub effective_capabilities: Vec<String>,
    pub effective_limits: EffectiveLimits,
}
