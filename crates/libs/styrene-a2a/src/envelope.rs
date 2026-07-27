use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{AgentId, RootOperationId, RuntimeId};

/// Draft profile version. Field numbers remain unstable until signing and golden vectors land.
pub const AGENT_ENVELOPE_PROFILE_VERSION: u16 = 1;
pub const A2A_JSON_CONTENT_TYPE: &str = "application/a2a+json";
pub const MAX_AGENT_ENVELOPE_SIZE: usize = 4 * 1024 * 1024;
pub const MAX_A2A_PAYLOAD_SIZE: usize = MAX_AGENT_ENVELOPE_SIZE - 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode, Serialize, Deserialize)]
#[cbor(index_only)]
pub enum AgentEnvelopeKind {
    #[n(0)]
    Command,
    #[n(1)]
    Event,
    #[n(2)]
    Result,
    #[n(3)]
    Receipt,
    #[n(4)]
    Snapshot,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct AgentEnvelope {
    #[n(0)]
    pub profile_version: u16,
    #[n(1)]
    pub message_id: [u8; 16],
    #[n(2)]
    pub kind: AgentEnvelopeKind,
    #[n(3)]
    pub source_agent_id: String,
    #[n(4)]
    pub source_runtime_id: [u8; 16],
    #[n(5)]
    pub target_agent_id: String,
    #[n(6)]
    pub target_runtime_id: Option<[u8; 16]>,
    #[n(7)]
    pub root_operation_id: String,
    #[n(8)]
    pub task_id: Option<String>,
    #[n(9)]
    pub parent_task_id: Option<String>,
    #[n(10)]
    pub stream_id: String,
    #[n(11)]
    pub sequence: u64,
    #[n(12)]
    pub created_at_ms: u64,
    #[n(13)]
    pub expires_at_ms: Option<u64>,
    #[n(14)]
    pub content_type: String,
    #[n(15)]
    pub payload_encoding: String,
    #[n(16)]
    pub payload_schema: String,
    #[n(17)]
    pub a2a_payload: Vec<u8>,
    #[n(18)]
    pub authorization: Option<Vec<u8>>,
    #[n(19)]
    pub grant_reference: Option<String>,
    #[n(20)]
    pub traceparent: Option<String>,
}

impl AgentEnvelope {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        kind: AgentEnvelopeKind,
        source_agent_id: &AgentId,
        source_runtime_id: RuntimeId,
        target_agent_id: &AgentId,
        root_operation_id: &RootOperationId,
        task_id: Option<String>,
        stream_id: impl Into<String>,
        sequence: u64,
        created_at_ms: u64,
        payload_schema: impl Into<String>,
        a2a_payload: Vec<u8>,
    ) -> Self {
        Self {
            profile_version: AGENT_ENVELOPE_PROFILE_VERSION,
            message_id: *Uuid::new_v4().as_bytes(),
            kind,
            source_agent_id: source_agent_id.as_str().to_owned(),
            source_runtime_id: source_runtime_id.into_bytes(),
            target_agent_id: target_agent_id.as_str().to_owned(),
            target_runtime_id: None,
            root_operation_id: root_operation_id.as_str().to_owned(),
            task_id,
            parent_task_id: None,
            stream_id: stream_id.into(),
            sequence,
            created_at_ms,
            expires_at_ms: None,
            content_type: A2A_JSON_CONTENT_TYPE.to_owned(),
            payload_encoding: "json".to_owned(),
            payload_schema: payload_schema.into(),
            a2a_payload,
            authorization: None,
            grant_reference: None,
            traceparent: None,
        }
    }

    pub fn validate(&self, now_ms: u64) -> Result<(), EnvelopeError> {
        self.validate_structure()?;
        if self.expires_at_ms.is_some_and(|expiry| expiry <= now_ms) {
            return Err(EnvelopeError::Expired);
        }
        Ok(())
    }

    /// Validate bearer-independent invariants. This is also enforced before encoding.
    pub fn validate_structure(&self) -> Result<(), EnvelopeError> {
        if self.profile_version != AGENT_ENVELOPE_PROFILE_VERSION {
            return Err(EnvelopeError::UnsupportedVersion(self.profile_version));
        }
        AgentId::new(&self.source_agent_id).map_err(|_| EnvelopeError::InvalidSourceAgentId)?;
        AgentId::new(&self.target_agent_id).map_err(|_| EnvelopeError::InvalidTargetAgentId)?;
        RootOperationId::new(&self.root_operation_id)
            .map_err(|_| EnvelopeError::InvalidRootOperationId)?;
        if self.sequence == 0 {
            return Err(EnvelopeError::InvalidSequence);
        }
        if self.stream_id.trim().is_empty() {
            return Err(EnvelopeError::MissingStreamId);
        }
        match self.kind {
            AgentEnvelopeKind::Event | AgentEnvelopeKind::Result => {
                let task_id = self
                    .task_id
                    .as_deref()
                    .filter(|id| !id.trim().is_empty())
                    .ok_or(EnvelopeError::MissingTaskId)?;
                if self.stream_id != task_id {
                    return Err(EnvelopeError::StreamIdMismatch);
                }
            }
            AgentEnvelopeKind::Snapshot if self.stream_id != self.root_operation_id => {
                return Err(EnvelopeError::StreamIdMismatch);
            }
            _ => {}
        }
        if self.parent_task_id.as_deref().is_some_and(|id| id.trim().is_empty()) {
            return Err(EnvelopeError::InvalidParentTaskId);
        }
        if self.content_type != A2A_JSON_CONTENT_TYPE || self.payload_encoding != "json" {
            return Err(EnvelopeError::UnsupportedPayloadFormat);
        }
        if self.payload_schema.trim().is_empty() {
            return Err(EnvelopeError::MissingPayloadSchema);
        }
        if self.a2a_payload.len() > MAX_A2A_PAYLOAD_SIZE {
            return Err(EnvelopeError::PayloadTooLarge(self.a2a_payload.len()));
        }
        serde_json::from_slice::<serde_json::Value>(&self.a2a_payload)
            .map_err(|error| EnvelopeError::InvalidJsonPayload(error.to_string()))?;
        if self.expires_at_ms.is_some_and(|expiry| expiry <= self.created_at_ms) {
            return Err(EnvelopeError::InvalidExpiry);
        }
        Ok(())
    }

    pub fn encode_cbor(&self) -> Result<Vec<u8>, EnvelopeError> {
        self.validate_structure()?;
        let bytes =
            minicbor::to_vec(self).map_err(|error| EnvelopeError::Encode(error.to_string()))?;
        if bytes.len() > MAX_AGENT_ENVELOPE_SIZE {
            return Err(EnvelopeError::EnvelopeTooLarge(bytes.len()));
        }
        Ok(bytes)
    }

    pub fn decode_cbor(bytes: &[u8], now_ms: u64) -> Result<Self, EnvelopeError> {
        if bytes.len() > MAX_AGENT_ENVELOPE_SIZE {
            return Err(EnvelopeError::EnvelopeTooLarge(bytes.len()));
        }
        let envelope: Self =
            minicbor::decode(bytes).map_err(|error| EnvelopeError::Decode(error.to_string()))?;
        envelope.validate(now_ms)?;
        Ok(envelope)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EnvelopeError {
    #[error("failed to encode agent envelope: {0}")]
    Encode(String),
    #[error("failed to decode agent envelope: {0}")]
    Decode(String),
    #[error("unsupported agent envelope profile version {0}")]
    UnsupportedVersion(u16),
    #[error("source agent id is invalid")]
    InvalidSourceAgentId,
    #[error("target agent id is invalid")]
    InvalidTargetAgentId,
    #[error("root operation id is invalid")]
    InvalidRootOperationId,
    #[error("agent envelope sequence must start at 1")]
    InvalidSequence,
    #[error("agent envelope stream id is required")]
    MissingStreamId,
    #[error("task event/result envelope requires a task id")]
    MissingTaskId,
    #[error("stream id does not match the envelope kind's ordering scope")]
    StreamIdMismatch,
    #[error("parent task id cannot be empty")]
    InvalidParentTaskId,
    #[error("agent envelope payload schema is required")]
    MissingPayloadSchema,
    #[error("unsupported A2A payload content type or encoding")]
    UnsupportedPayloadFormat,
    #[error("A2A JSON payload is invalid: {0}")]
    InvalidJsonPayload(String),
    #[error("agent envelope expiry must be later than its creation time")]
    InvalidExpiry,
    #[error("agent envelope has expired")]
    Expired,
    #[error("A2A payload is too large: {0} bytes")]
    PayloadTooLarge(usize),
    #[error("agent envelope is too large: {0} bytes")]
    EnvelopeTooLarge(usize),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope() -> AgentEnvelope {
        AgentEnvelope::new(
            AgentEnvelopeKind::Command,
            &AgentId::new("styrene:agent:source").unwrap(),
            RuntimeId::new(),
            &AgentId::new("styrene:agent:target").unwrap(),
            &RootOperationId::new("root-1").unwrap(),
            Some("task-1".to_owned()),
            "task-1",
            1,
            1_000,
            "a2a.message/1.0",
            br#"{"kind":"message"}"#.to_vec(),
        )
    }

    #[test]
    fn envelope_round_trip_preserves_index_fields() {
        let envelope = envelope();
        let encoded = envelope.encode_cbor().unwrap();
        let decoded = AgentEnvelope::decode_cbor(&encoded, 1_000).unwrap();
        assert_eq!(decoded, envelope);
        assert_eq!(decoded.stream_id, "task-1");
        assert_eq!(decoded.root_operation_id, "root-1");
    }

    #[test]
    fn rejects_zero_sequence_and_expired_envelope() {
        let mut envelope = envelope();
        envelope.sequence = 0;
        assert_eq!(envelope.validate(1_000), Err(EnvelopeError::InvalidSequence));

        envelope.sequence = 1;
        envelope.expires_at_ms = Some(1_001);
        assert_eq!(envelope.validate(1_001), Err(EnvelopeError::Expired));
    }

    #[test]
    fn encode_rejects_invalid_structure() {
        let mut envelope = envelope();
        envelope.source_agent_id.clear();
        assert_eq!(envelope.encode_cbor(), Err(EnvelopeError::InvalidSourceAgentId));
    }

    #[test]
    fn validates_stream_scope_by_envelope_kind() {
        let mut envelope = envelope();
        envelope.kind = AgentEnvelopeKind::Event;
        envelope.stream_id = "different-task".to_owned();
        assert_eq!(envelope.validate_structure(), Err(EnvelopeError::StreamIdMismatch));

        envelope.kind = AgentEnvelopeKind::Snapshot;
        envelope.task_id = None;
        envelope.stream_id = envelope.root_operation_id.clone();
        assert!(envelope.validate_structure().is_ok());
    }

    #[test]
    fn rejects_invalid_json_and_expiry_interval() {
        let mut envelope = envelope();
        envelope.a2a_payload = b"not-json".to_vec();
        assert!(matches!(envelope.validate_structure(), Err(EnvelopeError::InvalidJsonPayload(_))));

        envelope.a2a_payload = b"{}".to_vec();
        envelope.expires_at_ms = Some(envelope.created_at_ms);
        assert_eq!(envelope.validate_structure(), Err(EnvelopeError::InvalidExpiry));
    }

    #[test]
    fn rejects_oversized_payload() {
        let mut envelope = envelope();
        envelope.a2a_payload = vec![0; MAX_A2A_PAYLOAD_SIZE + 1];
        assert_eq!(
            envelope.validate(1_000),
            Err(EnvelopeError::PayloadTooLarge(MAX_A2A_PAYLOAD_SIZE + 1))
        );
    }
}
