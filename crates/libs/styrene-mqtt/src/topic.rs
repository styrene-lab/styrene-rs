use crate::error::{MqttError, Result};

const PREFIX: &str = "styrene";
const EVENTS_SEGMENT: &str = "events";

/// Components of a fully-qualified Aether topic.
///
/// Topic format: `styrene/{operator_id}/{service}/{instance_id}/events/{event_type}`
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TopicAddress {
    pub operator_id: String,
    pub service: String,
    pub instance_id: String,
    pub event_type: String,
}

impl TopicAddress {
    /// Render as an MQTT topic string.
    pub fn to_topic_string(&self) -> String {
        format!(
            "{}/{}/{}/{}/{}/{}",
            PREFIX,
            self.operator_id,
            self.service,
            self.instance_id,
            EVENTS_SEGMENT,
            self.event_type,
        )
    }

    /// Parse from an MQTT topic string.
    pub fn parse(topic: &str) -> Result<Self> {
        let parts: Vec<&str> = topic.splitn(6, '/').collect();

        if parts.len() < 6 {
            return Err(MqttError::InvalidTopic(format!(
                "expected at least 6 segments, got {}: `{topic}`",
                parts.len()
            )));
        }

        if parts[0] != PREFIX {
            return Err(MqttError::InvalidTopic(format!(
                "expected prefix `{PREFIX}`, got `{}`",
                parts[0]
            )));
        }

        if parts[4] != EVENTS_SEGMENT {
            return Err(MqttError::InvalidTopic(format!(
                "expected `{EVENTS_SEGMENT}` at segment 4, got `{}`",
                parts[4]
            )));
        }

        Ok(Self {
            operator_id: parts[1].to_owned(),
            service: parts[2].to_owned(),
            instance_id: parts[3].to_owned(),
            event_type: parts[5].to_owned(),
        })
    }
}

impl std::fmt::Display for TopicAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_topic_string())
    }
}

/// Fluent builder for constructing MQTT topic strings and subscription filters.
///
/// Unset fields become MQTT wildcards in subscription filters:
/// - operator/service/instance: `+` (single-level)
/// - event_type: `#` (multi-level, matches all sub-levels)
#[derive(Debug, Clone, Default)]
pub struct TopicBuilder {
    operator_id: Option<String>,
    service: Option<String>,
    instance_id: Option<String>,
    event_type: Option<String>,
}

impl TopicBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn operator(mut self, id: impl Into<String>) -> Self {
        self.operator_id = Some(id.into());
        self
    }

    pub fn service(mut self, name: impl Into<String>) -> Self {
        self.service = Some(name.into());
        self
    }

    pub fn instance(mut self, id: impl Into<String>) -> Self {
        self.instance_id = Some(id.into());
        self
    }

    pub fn event_type(mut self, ty: impl Into<String>) -> Self {
        self.event_type = Some(ty.into());
        self
    }

    /// Build a concrete publish topic. All fields must be set.
    pub fn build_publish(&self) -> Result<String> {
        let operator_id = self
            .operator_id
            .as_deref()
            .ok_or_else(|| MqttError::InvalidTopic("operator_id required for publish".into()))?;
        let service = self
            .service
            .as_deref()
            .ok_or_else(|| MqttError::InvalidTopic("service required for publish".into()))?;
        let instance_id = self
            .instance_id
            .as_deref()
            .ok_or_else(|| MqttError::InvalidTopic("instance_id required for publish".into()))?;
        let event_type = self
            .event_type
            .as_deref()
            .ok_or_else(|| MqttError::InvalidTopic("event_type required for publish".into()))?;

        Ok(format!(
            "{PREFIX}/{operator_id}/{service}/{instance_id}/{EVENTS_SEGMENT}/{event_type}"
        ))
    }

    /// Build a subscription filter. Unset fields become MQTT wildcards.
    pub fn build_subscribe(&self) -> String {
        let op = self.operator_id.as_deref().unwrap_or("+");
        let svc = self.service.as_deref().unwrap_or("+");
        let inst = self.instance_id.as_deref().unwrap_or("+");
        let evt = self.event_type.as_deref().unwrap_or("#");

        format!("{PREFIX}/{op}/{svc}/{inst}/{EVENTS_SEGMENT}/{evt}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_roundtrip() {
        let addr = TopicAddress {
            operator_id: "op1".into(),
            service: "omegon".into(),
            instance_id: "abc123".into(),
            event_type: "turn.started".into(),
        };
        let topic = addr.to_topic_string();
        assert_eq!(topic, "styrene/op1/omegon/abc123/events/turn.started");

        let parsed = TopicAddress::parse(&topic).expect("parse should succeed");
        assert_eq!(parsed, addr);
    }

    #[test]
    fn parse_rejects_short_topic() {
        assert!(TopicAddress::parse("styrene/op1/omegon").is_err());
    }

    #[test]
    fn parse_rejects_wrong_prefix() {
        assert!(TopicAddress::parse("wrong/op1/omegon/abc/events/turn.started").is_err());
    }

    #[test]
    fn parse_rejects_missing_events_segment() {
        assert!(TopicAddress::parse("styrene/op1/omegon/abc/other/turn.started").is_err());
    }

    #[test]
    fn builder_publish_requires_all_fields() {
        let b = TopicBuilder::new().operator("op1").service("omegon");
        assert!(b.build_publish().is_err());
    }

    #[test]
    fn builder_publish_full() {
        let topic = TopicBuilder::new()
            .operator("op1")
            .service("omegon")
            .instance("abc123")
            .event_type("tool.ended")
            .build_publish()
            .expect("should succeed");
        assert_eq!(topic, "styrene/op1/omegon/abc123/events/tool.ended");
    }

    #[test]
    fn builder_subscribe_all_wildcards() {
        let filter = TopicBuilder::new().build_subscribe();
        assert_eq!(filter, "styrene/+/+/+/events/#");
    }

    #[test]
    fn builder_subscribe_partial() {
        let filter = TopicBuilder::new()
            .operator("op1")
            .service("omegon")
            .build_subscribe();
        assert_eq!(filter, "styrene/op1/omegon/+/events/#");
    }

    #[test]
    fn builder_subscribe_specific_event() {
        let filter = TopicBuilder::new()
            .operator("op1")
            .event_type("turn.started")
            .build_subscribe();
        assert_eq!(filter, "styrene/op1/+/+/events/turn.started");
    }
}
