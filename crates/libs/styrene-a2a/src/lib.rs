//! Transport-independent Styrene facade for the Linux Foundation A2A protocol.
//!
//! Applications depend on this crate rather than directly on `a2a-lf`. The
//! facade deliberately keeps transport routes out of signed/domain payloads.

mod envelope;
mod extension;
mod protocol;
mod snapshot;

pub use a2a::agent_card::{AgentCapabilities, AgentCard, AgentSkill};
pub use a2a::event::{
    StreamResponse as AgentEvent, TaskArtifactUpdateEvent, TaskStatusUpdateEvent,
};
pub use a2a::types::{Artifact, Message, Part, Role, Task, TaskState, TaskStatus};
pub use envelope::{
    AgentEnvelope, AgentEnvelopeKind, EnvelopeError, SignatureAlgorithm, A2A_JSON_CONTENT_TYPE,
    AGENT_ENVELOPE_PROFILE_VERSION, MAX_A2A_PAYLOAD_SIZE, MAX_AGENT_ENVELOPE_SIZE,
};
pub use extension::{
    AgentId, AgentRuntimeRef, ControlClass, DelegationRelationship, RootOperationId, RuntimeId,
    StyreneDelegationExtension, STYRENE_DELEGATION_EXTENSION_URI,
};
pub use protocol::{AcceptanceDisposition, AcceptanceReceipt, ProtocolError, ProtocolErrorCode};
pub use snapshot::{
    AgentGraphEdge, AgentSnapshot, AgentSnapshotRequest, AgentTaskSnapshot, CancellationState,
    GraphEdgeRelationship, SequenceWatermark, SnapshotValidationError,
};
