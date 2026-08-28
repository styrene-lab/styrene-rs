use std::time::Duration;

use bytes::Bytes;
use rumqttc::v5::mqttbytes::v5::PublishProperties;
use styrene_a2a::{A2A_JSON_CONTENT_TYPE, AgentEnvelope, AgentEnvelopeKind, EnvelopeError};

use crate::{A2aTopic, A2aTopicKind, MqttA2aError, Result};

pub const MQTT_A2A_CONTENT_TYPE: &str = "application/styrene-a2a+cbor;v=1";

#[derive(Clone, Debug)]
pub struct A2aPublication {
    pub topic: String,
    pub payload: Vec<u8>,
    pub qos: rumqttc::v5::mqttbytes::QoS,
    pub retain: bool,
    pub properties: PublishProperties,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceivedA2aEnvelope {
    pub topic: A2aTopic,
    pub envelope: AgentEnvelope,
}

pub fn publication_for(
    tenant: &str,
    envelope: &AgentEnvelope,
    now_ms: u64,
) -> Result<A2aPublication> {
    envelope.validate(now_ms).map_err(MqttA2aError::Envelope)?;
    let kind = match envelope.kind {
        AgentEnvelopeKind::Command => A2aTopicKind::Inbox,
        AgentEnvelopeKind::Event | AgentEnvelopeKind::Result | AgentEnvelopeKind::Receipt => {
            A2aTopicKind::Events
        }
        AgentEnvelopeKind::Snapshot => A2aTopicKind::Snapshot,
    };
    let topic = A2aTopic::new(tenant, envelope.target_agent_id.clone(), kind)?.render();
    let payload = envelope.encode_cbor().map_err(MqttA2aError::Envelope)?;
    let expiry =
        envelope.expires_at_ms.map(|expires| expiry_seconds(expires, now_ms)).transpose()?;
    let properties = PublishProperties {
        payload_format_indicator: Some(0),
        message_expiry_interval: expiry,
        correlation_data: Some(Bytes::copy_from_slice(&envelope.message_id)),
        content_type: Some(MQTT_A2A_CONTENT_TYPE.to_owned()),
        ..Default::default()
    };
    Ok(A2aPublication {
        topic,
        payload,
        qos: rumqttc::v5::mqttbytes::QoS::AtLeastOnce,
        retain: matches!(envelope.kind, AgentEnvelopeKind::Snapshot),
        properties,
    })
}

pub fn decode_publication(
    topic: &str,
    payload: &[u8],
    retained: bool,
    properties: &PublishProperties,
    now_ms: u64,
) -> Result<ReceivedA2aEnvelope> {
    let topic = A2aTopic::parse(topic)?;
    if properties.content_type.as_deref() != Some(MQTT_A2A_CONTENT_TYPE) {
        return Err(MqttA2aError::InvalidContentType);
    }
    let envelope = AgentEnvelope::decode_cbor(payload, now_ms).map_err(MqttA2aError::Envelope)?;
    if topic.agent_id != envelope.target_agent_id {
        return Err(MqttA2aError::TargetMismatch);
    }
    if properties.correlation_data.as_deref() != Some(envelope.message_id.as_slice()) {
        return Err(MqttA2aError::CorrelationMismatch);
    }
    if retained && !matches!(envelope.kind, AgentEnvelopeKind::Snapshot) {
        return Err(MqttA2aError::RetainedNonSnapshot);
    }
    let expected_kind = match envelope.kind {
        AgentEnvelopeKind::Command => A2aTopicKind::Inbox,
        AgentEnvelopeKind::Event | AgentEnvelopeKind::Result | AgentEnvelopeKind::Receipt => {
            A2aTopicKind::Events
        }
        AgentEnvelopeKind::Snapshot => A2aTopicKind::Snapshot,
    };
    if topic.kind != expected_kind {
        return Err(MqttA2aError::TopicKindMismatch);
    }
    Ok(ReceivedA2aEnvelope { topic, envelope })
}

fn expiry_seconds(expires_at_ms: u64, now_ms: u64) -> Result<u32> {
    if expires_at_ms <= now_ms {
        return Err(MqttA2aError::Envelope(EnvelopeError::Expired));
    }
    let remaining = Duration::from_millis(expires_at_ms - now_ms).as_secs().max(1);
    u32::try_from(remaining).map_err(|_| MqttA2aError::ExpiryTooLarge)
}

pub fn payload_content_type(envelope: &AgentEnvelope) -> Result<&str> {
    if envelope.content_type != A2A_JSON_CONTENT_TYPE {
        return Err(MqttA2aError::InvalidEnvelopeContentType);
    }
    Ok(&envelope.content_type)
}

#[cfg(test)]
mod tests {
    use super::*;
    use styrene_a2a::{AgentId, RootOperationId, RuntimeId};

    fn command() -> AgentEnvelope {
        let mut envelope = AgentEnvelope::new(
            AgentEnvelopeKind::Command,
            &AgentId::new("source").unwrap(),
            RuntimeId::new(),
            &AgentId::new("target/+/#").unwrap(),
            &RootOperationId::new("root").unwrap(),
            Some("task-1".into()),
            "task-1",
            1,
            1_000,
            "a2a.message.v1",
            br#"{"role":"user"}"#.to_vec(),
        );
        envelope.expires_at_ms = Some(31_000);
        envelope
    }

    #[test]
    fn command_maps_to_qos_one_non_retained_inbox() {
        let envelope = command();
        let publication = publication_for("tenant", &envelope, 1_000).unwrap();
        assert_eq!(publication.qos, rumqttc::v5::mqttbytes::QoS::AtLeastOnce);
        assert!(!publication.retain);
        assert!(publication.topic.ends_with("/inbox"));
        assert_eq!(publication.properties.message_expiry_interval, Some(30));
        let decoded = decode_publication(
            &publication.topic,
            &publication.payload,
            publication.retain,
            &publication.properties,
            1_000,
        )
        .unwrap();
        assert_eq!(decoded.envelope, envelope);
    }

    #[test]
    fn retained_command_is_rejected() {
        let envelope = command();
        let publication = publication_for("tenant", &envelope, 1_000).unwrap();
        assert_eq!(
            decode_publication(
                &publication.topic,
                &publication.payload,
                true,
                &publication.properties,
                1_000,
            ),
            Err(MqttA2aError::RetainedNonSnapshot)
        );
    }
}
