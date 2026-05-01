use thiserror::Error;

#[derive(Debug, Error)]
pub enum MqttError {
    #[error("connection failed: {0}")]
    Connection(String),

    #[error("publish failed: {0}")]
    Publish(String),

    #[error("subscribe failed: {0}")]
    Subscribe(String),

    #[error("serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("deserialization failed on topic `{topic}`: {source}")]
    Deserialization {
        topic: String,
        source: serde_json::Error,
    },

    #[error("broker error: {0}")]
    Broker(String),

    #[error("invalid topic: {0}")]
    InvalidTopic(String),
}

pub type Result<T> = std::result::Result<T, MqttError>;
