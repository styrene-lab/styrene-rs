//! MQTT 5 bearer adapter for canonical [`styrene_a2a::AgentEnvelope`] values.
//!
//! This crate owns MQTT topic mapping and MQTT 5 properties only. A2A task,
//! identity, signing, delegation, replay, and snapshot semantics remain owned by
//! `styrene-a2a` and the agent service.

mod adapter;
mod client;
mod error;
mod topic;

pub use adapter::{
    decode_publication, payload_content_type, publication_for, A2aPublication, ReceivedA2aEnvelope,
    MQTT_A2A_CONTENT_TYPE,
};
pub use client::MqttA2aClient;
pub use error::{MqttA2aError, Result};
pub use topic::{A2aTopic, A2aTopicKind, A2A_TOPIC_PREFIX};
