use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{AgentId, RuntimeId};

pub const AGENT_ENVELOPE_PROFILE_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode, Serialize, Deserialize)]
#[cbor(index_only)]
pub enum AgentEnvelopeKind {
    #[n(0)] Command,
    #[n(1)] Event,
    #[n(2)] Result,
    #[n(3)] Receipt,
    #[n(4)] Snapshot,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct AgentEnvelope {
    #[n(0)] pub profile_version: u16,
    #[n(1)] pub message_id: [u8; 16],
    #[n(2)] pub kind: AgentEnvelopeKind,
    #[n(3)] pub source_agent_id: String,
    #[n(4)] pub source_runtime_id: [u8; 16],
    #[n(5)] pub target_agent_id: String,
    #[n(6)] pub sequence: u64,
    #[n(7)] pub created_at_ms: u64,
    #[n(8)] pub expires_at_ms: Option<u64>,
    #[n(9)] pub payload_schema: String,
    #[n(10)] pub a2a_payload: Vec<u8>,
    #[n(11)] pub authorization: Option<Vec<u8>>,
    #[n(12)] pub traceparent: Option<String>,
}

impl AgentEnvelope {
    pub fn new(
        kind: AgentEnvelopeKind,
        source_agent_id: &AgentId,
        source_runtime_id: RuntimeId,
        target_agent_id: &AgentId,
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
            sequence,
            created_at_ms,
            expires_at_ms: None,
            payload_schema: payload_schema.into(),
            a2a_payload,
            authorization: None,
            traceparent: None,
        }
    }

    pub fn encode_cbor(&self) -> Result<Vec<u8>, EnvelopeError> {
        minicbor::to_vec(self).map_err(|error| EnvelopeError::Encode(error.to_string()))
    }

    pub fn decode_cbor(bytes: &[u8]) -> Result<Self, EnvelopeError> {
        let envelope: Self = minicbor::decode(bytes)
            .map_err(|error| EnvelopeError::Decode(error.to_string()))?;
        if envelope.profile_version != AGENT_ENVELOPE_PROFILE_VERSION {
            return Err(EnvelopeError::UnsupportedVersion(envelope.profile_version));
        }
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_envelope_round_trip() {
        let source = AgentId::new("styrene:agent:source").unwrap();
        let target = AgentId::new("styrene:agent:target").unwrap();
        let envelope = AgentEnvelope::new(
            AgentEnvelopeKind::Command,
            &source,
            RuntimeId::new(),
            &target,
            7,
            42,
            "https://a2a-protocol.org/schemas/message/v1",
            br#"{"role":"user"}"#.to_vec(),
        );
        let decoded = AgentEnvelope::decode_cbor(&envelope.encode_cbor().unwrap()).unwrap();
        assert_eq!(decoded, envelope);
    }
}
