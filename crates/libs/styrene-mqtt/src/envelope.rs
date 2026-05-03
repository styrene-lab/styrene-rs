use chrono::{DateTime, Utc};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::error::{MqttError, Result};
use crate::topic::TopicAddress;

// MQTT 5.0 user property keys.
const KEY_TIMESTAMP: &str = "ts";
const KEY_SOURCE_SERVICE: &str = "svc";
const KEY_SOURCE_INSTANCE: &str = "inst";
const KEY_OPERATOR_ID: &str = "op";
const KEY_SCHEMA_VERSION: &str = "sv";
const KEY_CORRELATION_ID: &str = "cid";

/// Metadata carried alongside every Aether event.
///
/// Serialized as MQTT 5.0 user properties (not in the payload body) so that
/// brokers and middleware can route/filter without deserializing payloads.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Metadata {
    /// Publication timestamp.
    pub timestamp: DateTime<Utc>,
    /// Source service name (e.g. "omegon", "scry", "viz").
    pub source_service: String,
    /// Source instance ID (unique per process).
    pub source_instance: String,
    /// Operator identity that owns this service instance.
    pub operator_id: String,
    /// Schema version of the payload for forward-compatible evolution.
    pub schema_version: u16,
    /// Optional correlation ID for request-response or causal chains.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
}

/// An Aether event envelope: metadata + typed payload.
///
/// The payload `T` is the JSON body; metadata travels as MQTT user properties.
#[derive(Debug, Clone)]
pub struct Envelope<T> {
    pub meta: Metadata,
    pub payload: T,
}

/// A received Aether message with parsed topic address and MQTT metadata.
#[derive(Debug, Clone)]
pub struct Message<T> {
    pub envelope: Envelope<T>,
    pub address: TopicAddress,
    /// MQTT QoS level the message was received at.
    pub qos: u8,
    /// Whether this was a retained message (late-join snapshot).
    pub retained: bool,
}

/// Encode metadata as MQTT 5.0 user property key-value pairs.
pub fn encode_user_properties(meta: &Metadata) -> Vec<(String, String)> {
    let mut props = vec![
        (KEY_TIMESTAMP.into(), meta.timestamp.to_rfc3339()),
        (KEY_SOURCE_SERVICE.into(), meta.source_service.clone()),
        (KEY_SOURCE_INSTANCE.into(), meta.source_instance.clone()),
        (KEY_OPERATOR_ID.into(), meta.operator_id.clone()),
        (KEY_SCHEMA_VERSION.into(), meta.schema_version.to_string()),
    ];
    if let Some(cid) = &meta.correlation_id {
        props.push((KEY_CORRELATION_ID.into(), cid.clone()));
    }
    props
}

/// Decode metadata from MQTT 5.0 user property key-value pairs.
pub fn decode_user_properties(props: &[(String, String)]) -> Result<Metadata> {
    let find = |key: &str| -> Option<&str> {
        props.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    };

    let ts_str = find(KEY_TIMESTAMP).ok_or_else(|| {
        MqttError::InvalidTopic(format!("missing user property `{KEY_TIMESTAMP}`"))
    })?;
    let timestamp = DateTime::parse_from_rfc3339(ts_str)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| MqttError::InvalidTopic(format!("invalid timestamp `{ts_str}`: {e}")))?;

    let source_service = find(KEY_SOURCE_SERVICE)
        .ok_or_else(|| {
            MqttError::InvalidTopic(format!("missing user property `{KEY_SOURCE_SERVICE}`"))
        })?
        .to_owned();

    let source_instance = find(KEY_SOURCE_INSTANCE)
        .ok_or_else(|| {
            MqttError::InvalidTopic(format!("missing user property `{KEY_SOURCE_INSTANCE}`"))
        })?
        .to_owned();

    let operator_id = find(KEY_OPERATOR_ID)
        .ok_or_else(|| {
            MqttError::InvalidTopic(format!("missing user property `{KEY_OPERATOR_ID}`"))
        })?
        .to_owned();

    let sv_str = find(KEY_SCHEMA_VERSION).ok_or_else(|| {
        MqttError::InvalidTopic(format!("missing user property `{KEY_SCHEMA_VERSION}`"))
    })?;
    let schema_version: u16 = sv_str
        .parse()
        .map_err(|e| MqttError::InvalidTopic(format!("invalid schema_version `{sv_str}`: {e}")))?;

    let correlation_id = find(KEY_CORRELATION_ID).map(str::to_owned);

    Ok(Metadata {
        timestamp,
        source_service,
        source_instance,
        operator_id,
        schema_version,
        correlation_id,
    })
}

/// Serialize a payload to JSON bytes.
pub fn encode_payload<T: Serialize>(payload: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(payload).map_err(MqttError::from)
}

/// Deserialize a payload from JSON bytes.
pub fn decode_payload<T: DeserializeOwned>(bytes: &[u8], topic: &str) -> Result<T> {
    serde_json::from_slice(bytes)
        .map_err(|e| MqttError::Deserialization { topic: topic.to_owned(), source: e })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn sample_meta() -> Metadata {
        Metadata {
            timestamp: Utc.with_ymd_and_hms(2026, 4, 25, 12, 0, 0).unwrap(),
            source_service: "omegon".into(),
            source_instance: "abc123".into(),
            operator_id: "op1".into(),
            schema_version: 1,
            correlation_id: None,
        }
    }

    #[test]
    fn user_properties_roundtrip() {
        let meta = sample_meta();
        let props = encode_user_properties(&meta);
        let decoded = decode_user_properties(&props).expect("decode should succeed");
        assert_eq!(decoded, meta);
    }

    #[test]
    fn user_properties_with_correlation_id() {
        let mut meta = sample_meta();
        meta.correlation_id = Some("req-42".into());
        let props = encode_user_properties(&meta);
        let decoded = decode_user_properties(&props).expect("decode should succeed");
        assert_eq!(decoded.correlation_id, Some("req-42".into()));
    }

    #[test]
    fn decode_rejects_missing_timestamp() {
        let props = vec![
            ("svc".into(), "omegon".into()),
            ("inst".into(), "abc".into()),
            ("op".into(), "op1".into()),
            ("sv".into(), "1".into()),
        ];
        assert!(decode_user_properties(&props).is_err());
    }

    #[test]
    fn payload_roundtrip() {
        #[derive(Debug, Serialize, Deserialize, PartialEq)]
        struct TestEvent {
            turn: u32,
        }
        let evt = TestEvent { turn: 7 };
        let bytes = encode_payload(&evt).expect("encode");
        let decoded: TestEvent = decode_payload(&bytes, "test/topic").expect("decode");
        assert_eq!(decoded, evt);
    }
}
