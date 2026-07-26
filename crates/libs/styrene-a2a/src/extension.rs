use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const STYRENE_DELEGATION_EXTENSION_URI: &str =
    "https://styrene.io/a2a/extensions/delegation/v1";

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, ExtensionValidationError> {
                let value = value.into();
                if value.trim().is_empty() || value.len() > 256 {
                    return Err(ExtensionValidationError::InvalidIdentity);
                }
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

string_id!(AgentId);
string_id!(RootOperationId);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RuntimeId(Uuid);

impl RuntimeId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
    pub fn into_bytes(self) -> [u8; 16] {
        self.0.into_bytes()
    }
}

impl Default for RuntimeId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRuntimeRef {
    pub agent_id: AgentId,
    pub runtime_id: RuntimeId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationRelationship {
    Delegate,
    CleaveChild,
    Handoff,
    OperatorAttach,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlClass {
    Owned,
    Attached,
    Observed,
    Delegated,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StyreneDelegationExtension {
    pub root_operation_id: RootOperationId,
    pub parent_task_id: Option<String>,
    pub source: AgentRuntimeRef,
    pub relationship: DelegationRelationship,
    pub control_class: ControlClass,
    pub remaining_depth: u16,
    pub grant_reference: Option<String>,
    pub traceparent: Option<String>,
}

impl StyreneDelegationExtension {
    pub fn validate_child_of(&self, parent: &Self) -> Result<(), ExtensionValidationError> {
        if self.root_operation_id != parent.root_operation_id {
            return Err(ExtensionValidationError::RootMismatch);
        }
        if self.remaining_depth >= parent.remaining_depth {
            return Err(ExtensionValidationError::DepthEscalation);
        }
        if self.parent_task_id.as_deref().is_none_or(str::is_empty) {
            return Err(ExtensionValidationError::MissingParentTask);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ExtensionValidationError {
    #[error("agent/root identity is empty or exceeds 256 bytes")]
    InvalidIdentity,
    #[error("child root operation does not match parent")]
    RootMismatch,
    #[error("child did not attenuate remaining delegation depth")]
    DepthEscalation,
    #[error("nested child is missing parent task id")]
    MissingParentTask,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extension(depth: u16, parent: Option<&str>) -> StyreneDelegationExtension {
        StyreneDelegationExtension {
            root_operation_id: RootOperationId::new("root-1").unwrap(),
            parent_task_id: parent.map(str::to_owned),
            source: AgentRuntimeRef {
                agent_id: AgentId::new("agent-a").unwrap(),
                runtime_id: RuntimeId::new(),
            },
            relationship: DelegationRelationship::Delegate,
            control_class: ControlClass::Delegated,
            remaining_depth: depth,
            grant_reference: None,
            traceparent: None,
        }
    }

    #[test]
    fn child_must_attenuate_depth_and_keep_root() {
        let parent = extension(3, None);
        assert!(extension(2, Some("task-parent")).validate_child_of(&parent).is_ok());
        assert_eq!(
            extension(3, Some("task-parent")).validate_child_of(&parent),
            Err(ExtensionValidationError::DepthEscalation)
        );
    }
}
