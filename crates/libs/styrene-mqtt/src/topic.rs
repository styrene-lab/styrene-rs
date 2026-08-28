use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use crate::{MqttA2aError, Result};

pub const A2A_TOPIC_PREFIX: &str = "styrene/v1/a2a";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum A2aTopicKind {
    Inbox,
    Events,
    Snapshot,
    Presence,
}

impl A2aTopicKind {
    fn segment(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Events => "events",
            Self::Snapshot => "snapshot",
            Self::Presence => "presence",
        }
    }

    fn parse(segment: &str) -> Option<Self> {
        match segment {
            "inbox" => Some(Self::Inbox),
            "events" => Some(Self::Events),
            "snapshot" => Some(Self::Snapshot),
            "presence" => Some(Self::Presence),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A2aTopic {
    pub tenant: String,
    pub agent_id: String,
    pub kind: A2aTopicKind,
}

impl A2aTopic {
    pub fn new(
        tenant: impl Into<String>,
        agent_id: impl Into<String>,
        kind: A2aTopicKind,
    ) -> Result<Self> {
        let tenant = tenant.into();
        let agent_id = agent_id.into();
        validate_tenant(&tenant)?;
        validate_agent_id(&agent_id)?;
        Ok(Self { tenant, agent_id, kind })
    }

    pub fn inbox(tenant: impl Into<String>, agent_id: impl Into<String>) -> Result<Self> {
        Self::new(tenant, agent_id, A2aTopicKind::Inbox)
    }

    pub fn render(&self) -> String {
        format!(
            "{}/{}/{}/{}",
            A2A_TOPIC_PREFIX,
            self.tenant,
            encode_agent_id(&self.agent_id),
            self.kind.segment()
        )
    }

    pub fn parse(topic: &str) -> Result<Self> {
        let parts: Vec<_> = topic.split('/').collect();
        if parts.len() != 6 || parts[..3] != ["styrene", "v1", "a2a"] {
            return Err(MqttA2aError::InvalidTopic(topic.to_owned()));
        }
        let tenant = parts[3].to_owned();
        validate_tenant(&tenant)?;
        let agent_id = decode_agent_id(parts[4])?;
        let kind = A2aTopicKind::parse(parts[5])
            .ok_or_else(|| MqttA2aError::InvalidTopic(topic.to_owned()))?;
        Ok(Self { tenant, agent_id, kind })
    }

    pub fn agent_filter(tenant: &str, agent_id: &str) -> Result<String> {
        validate_tenant(tenant)?;
        validate_agent_id(agent_id)?;
        Ok(format!("{}/{}/{}/+", A2A_TOPIC_PREFIX, tenant, encode_agent_id(agent_id)))
    }
}

fn validate_tenant(tenant: &str) -> Result<()> {
    if tenant.is_empty()
        || tenant.len() > 128
        || tenant.bytes().any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')))
    {
        return Err(MqttA2aError::InvalidTenant(tenant.to_owned()));
    }
    Ok(())
}

fn validate_agent_id(agent_id: &str) -> Result<()> {
    if agent_id.trim().is_empty() || agent_id.len() > 256 {
        return Err(MqttA2aError::InvalidAgentId);
    }
    Ok(())
}

fn encode_agent_id(agent_id: &str) -> String {
    URL_SAFE_NO_PAD.encode(agent_id.as_bytes())
}

fn decode_agent_id(value: &str) -> Result<String> {
    let bytes = URL_SAFE_NO_PAD.decode(value).map_err(|_| MqttA2aError::InvalidTopicComponent)?;
    let value = String::from_utf8(bytes).map_err(|_| MqttA2aError::InvalidTopicComponent)?;
    validate_agent_id(&value)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topic_round_trip_escapes_mqtt_wildcards() {
        let topic = A2aTopic::inbox("acme", "did:styrene:agent/a+#").unwrap();
        let rendered = topic.render();
        assert!(!rendered.contains("agent/a+#"));
        assert_eq!(A2aTopic::parse(&rendered).unwrap(), topic);
    }

    #[test]
    fn rejects_wildcards_in_tenant() {
        assert!(A2aTopic::inbox("bad/+", "agent").is_err());
    }
}
