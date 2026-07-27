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
        if self.profile_version != AGENT_ENVELOPE_PROFILE_VERSION {
            return Err(EnvelopeError::UnsupportedVersion(self.profile_version));
        }
        if self.sequence == 0 {
            return Err(EnvelopeError::InvalidSequence);
        }
        if self.stream_id.trim().is_empty() {
            return Err(EnvelopeError::MissingStreamId);
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
        if self.expires_at_ms.is_some_and(|expiry| expiry <= now_ms) {
            return Err(EnvelopeError::Expired);
        }
        Ok(())
    }

    pub fn encode_cbor(&self) -> Result<Vec<u8>, EnvelopeError> {
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
    #[error("agent envelope sequence must start at 1")]
    InvalidSequence,
    #[error("agent envelope stream id is required")]
    MissingStreamId,
    #[error("agent envelope payload schema is required")]
    MissingPayloadSchema,
    #[error("unsupported A2A payload content type or encoding")]
    UnsupportedPayloadFormat,
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
        envelope.expires_at_ms = Some(1_000);
        assert_eq!(envelope.validate(1_000), Err(EnvelopeError::Expired));
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
