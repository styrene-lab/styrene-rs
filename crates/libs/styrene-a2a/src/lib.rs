//! Transport-independent Styrene facade for the Linux Foundation A2A protocol.
//!
//! Applications depend on this crate rather than directly on `a2a-lf`. The
//! facade deliberately keeps transport routes out of signed/domain payloads.

mod envelope;
mod extension;

pub use a2a::agent_card::{AgentCapabilities, AgentCard, AgentSkill};
pub use a2a::event::{
    StreamResponse as AgentEvent, TaskArtifactUpdateEvent, TaskStatusUpdateEvent,
};
pub use a2a::types::{Artifact, Message, Part, Role, Task, TaskState, TaskStatus};
pub use envelope::{AgentEnvelope, AgentEnvelopeKind, EnvelopeError};
pub use extension::{
    AgentId, AgentRuntimeRef, ControlClass, DelegationRelationship, RootOperationId, RuntimeId,
    StyreneDelegationExtension, STYRENE_DELEGATION_EXTENSION_URI,
};
