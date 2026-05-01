//! MQTT 5.0 infrastructure for the Aether event fabric.
//!
//! This crate provides the shared MQTT layer used by all Styrene ecosystem
//! services (Omegon, Scry, Viz, Aether, etc.) to communicate over the
//! Aether event bus.
//!
//! # Topic Schema
//!
//! All events are published to topics following the hierarchy:
//!
//! ```text
//! styrene/{operator_id}/{service}/{instance_id}/events/{event_type}
//! ```
//!
//! Use [`TopicBuilder`] to construct publish topics and subscription filters.
//!
//! # Broker Ownership
//!
//! The MQTT broker is owned by the operator control plane (Auspex), not by
//! individual services. Services connect as TCP clients. Auspex uses the
//! `embedded-broker` feature to start an in-process rumqttd instance and
//! holds an in-process link for its own aggregation pipeline.
//!
//! # Feature Gates
//!
//! - `embedded-broker` — includes rumqttd for hosting the broker (Auspex only)
//! - `tls` — TLS support for remote broker connections (future)
//! - `styrene-identity` — Ed25519 enhanced auth (future)

pub mod client;
pub mod envelope;
pub mod error;
pub mod qos;
pub mod stream;
pub mod topic;

#[cfg(feature = "embedded-broker")]
pub mod broker;

pub use client::{Client, ClientConfig, ConnectionTarget, ServiceIdentity};
pub use envelope::{Envelope, Message, Metadata};
pub use error::{MqttError, Result};
pub use qos::{QosOverride, qos_for_event};
pub use stream::{RawMessage, Subscription};
pub use topic::{TopicAddress, TopicBuilder};

#[cfg(feature = "embedded-broker")]
pub use broker::{BrokerLink, EmbeddedBroker, EmbeddedBrokerBuilder, EmbeddedBrokerConfig};
