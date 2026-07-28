use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MqttA2aError {
    #[error("MQTT connection failed: {0}")]
    Connection(String),
    #[error("MQTT publish failed: {0}")]
    Publish(String),
    #[error("MQTT subscribe failed: {0}")]
    Subscribe(String),
    #[error("invalid MQTT A2A topic `{0}`")]
    InvalidTopic(String),
    #[error("invalid MQTT tenant `{0}`")]
    InvalidTenant(String),
    #[error("invalid A2A agent identity")]
    InvalidAgentId,
    #[error("invalid encoded MQTT topic component")]
    InvalidTopicComponent,
    #[error("invalid MQTT A2A content type")]
    InvalidContentType,
    #[error("A2A envelope content type is unsupported")]
    InvalidEnvelopeContentType,
    #[error("MQTT correlation data does not match envelope message id")]
    CorrelationMismatch,
    #[error("MQTT topic target does not match signed envelope target")]
    TargetMismatch,
    #[error("MQTT topic kind does not match envelope kind")]
    TopicKindMismatch,
    #[error("only snapshots may be retained")]
    RetainedNonSnapshot,
    #[error("MQTT message expiry exceeds the protocol range")]
    ExpiryTooLarge,
    #[error(transparent)]
    Envelope(#[from] styrene_a2a::EnvelopeError),
}

pub type Result<T> = std::result::Result<T, MqttA2aError>;
