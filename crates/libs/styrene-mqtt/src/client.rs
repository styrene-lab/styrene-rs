use std::sync::Arc;
use std::time::Duration;

use rumqttc::v5::mqttbytes::QoS;
use rumqttc::v5::{AsyncClient, MqttOptions};
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::envelope::{
    decode_payload, decode_user_properties, encode_payload, encode_user_properties, Envelope,
    Message, Metadata,
};
use crate::error::{MqttError, Result};
use crate::qos::QosOverride;
use crate::stream::{RawMessage, Subscription};
use crate::topic::{TopicAddress, TopicBuilder};

/// Identity of this client on the Aether fabric.
#[derive(Debug, Clone)]
pub struct ServiceIdentity {
    pub operator_id: String,
    pub service: String,
    pub instance_id: String,
}

/// How to connect to the MQTT broker.
pub enum ConnectionTarget {
    /// Connect via an in-process rumqttd link (Tier 1).
    /// Requires the `embedded-broker` feature.
    #[cfg(feature = "embedded-broker")]
    InProcess { link: crate::broker::BrokerLink },
    /// Connect to a remote MQTT 5.0 broker via TCP.
    Remote { host: String, port: u16 },
}

/// Configuration for creating a [`Client`].
pub struct ClientConfig {
    pub identity: ServiceIdentity,
    pub target: ConnectionTarget,
    /// MQTT client ID. Defaults to `"{service}-{instance_id}"`.
    pub client_id: Option<String>,
    /// Channel capacity for the internal event loop. Default: 128.
    pub channel_capacity: usize,
    /// Keep-alive interval (remote only). Default: 30s.
    pub keep_alive: Duration,
}

impl ClientConfig {
    pub fn new(identity: ServiceIdentity, target: ConnectionTarget) -> Self {
        Self {
            identity,
            target,
            client_id: None,
            channel_capacity: 128,
            keep_alive: Duration::from_secs(30),
        }
    }
}

/// High-level Aether MQTT 5.0 client.
///
/// Publishes typed events and subscribes to topic patterns with automatic
/// deserialization. Supports both in-process (embedded broker) and remote
/// (TCP) connections.
pub struct Client {
    identity: ServiceIdentity,
    inner: ClientInner,
    raw_subscribers: Arc<tokio::sync::Mutex<Vec<FilteredSubscriber>>>,
}

/// A subscriber with its topic filter for fan-out matching.
struct FilteredSubscriber {
    filter: String,
    tx: mpsc::Sender<RawMessage>,
}

enum ClientInner {
    #[cfg(feature = "embedded-broker")]
    InProcess {
        link_tx: Arc<tokio::sync::Mutex<rumqttd::local::LinkTx>>,
        _recv_task: JoinHandle<()>,
    },
    Remote {
        mqtt: AsyncClient,
        _event_loop: JoinHandle<()>,
    },
}

impl Client {
    /// Connect to the broker and start the internal event loop.
    pub async fn connect(config: ClientConfig) -> Result<Self> {
        let raw_subscribers: Arc<tokio::sync::Mutex<Vec<FilteredSubscriber>>> =
            Arc::new(tokio::sync::Mutex::new(Vec::new()));

        let inner = match config.target {
            #[cfg(feature = "embedded-broker")]
            ConnectionTarget::InProcess { link } => {
                Self::connect_in_process(link, raw_subscribers.clone())
            }
            ConnectionTarget::Remote { host, port } => {
                Self::connect_remote(
                    &config.identity,
                    config.client_id.as_deref(),
                    &host,
                    port,
                    config.channel_capacity,
                    config.keep_alive,
                    raw_subscribers.clone(),
                )
                .await?
            }
        };

        Ok(Self { identity: config.identity, inner, raw_subscribers })
    }

    /// Publish a typed event.
    ///
    /// Topic is built from the client's identity + `event_type`.
    /// QoS is determined by policy unless overridden.
    pub async fn publish<T: Serialize>(
        &self,
        event_type: &str,
        payload: &T,
        qos_override: QosOverride,
    ) -> Result<()> {
        let topic = TopicBuilder::new()
            .operator(&self.identity.operator_id)
            .service(&self.identity.service)
            .instance(&self.identity.instance_id)
            .event_type(event_type)
            .build_publish()?;

        self.publish_inner(&topic, event_type, payload, qos_override, false).await
    }

    /// Publish to an explicit topic address (for relay/proxy scenarios).
    pub async fn publish_to<T: Serialize>(
        &self,
        address: &TopicAddress,
        payload: &T,
        qos_override: QosOverride,
    ) -> Result<()> {
        let topic = address.to_topic_string();
        self.publish_inner(&topic, &address.event_type, payload, qos_override, false).await
    }

    /// Publish a retained message (late-join state snapshot).
    pub async fn publish_retained<T: Serialize>(
        &self,
        event_type: &str,
        payload: &T,
    ) -> Result<()> {
        let topic = TopicBuilder::new()
            .operator(&self.identity.operator_id)
            .service(&self.identity.service)
            .instance(&self.identity.instance_id)
            .event_type(event_type)
            .build_publish()?;

        self.publish_inner(&topic, event_type, payload, QosOverride::Force(QoS::AtLeastOnce), true)
            .await
    }

    /// Subscribe to a topic filter and return a typed stream.
    ///
    /// Messages that fail deserialization are logged and skipped.
    pub async fn subscribe<T: DeserializeOwned + Send + 'static>(
        &self,
        filter: &str,
        qos: QoS,
    ) -> Result<Subscription<T>> {
        self.subscribe_mqtt(filter, qos).await?;

        let (raw_tx, mut raw_rx) = mpsc::channel::<RawMessage>(256);
        let (typed_tx, typed_rx) = mpsc::channel::<Result<Message<T>>>(256);

        {
            let mut guard = self.raw_subscribers.lock().await;
            guard.push(FilteredSubscriber { filter: filter.to_string(), tx: raw_tx });
        }

        tokio::spawn(async move {
            while let Some(raw) = raw_rx.recv().await {
                let address = match TopicAddress::parse(&raw.topic) {
                    Ok(a) => a,
                    Err(e) => {
                        tracing::debug!("skipping message on `{}`: {e}", raw.topic);
                        continue;
                    }
                };

                let meta = match decode_user_properties(&raw.user_properties) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::warn!("metadata decode failed on `{}`: {e}", raw.topic);
                        continue;
                    }
                };

                let payload: T = match decode_payload(&raw.payload, &raw.topic) {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::warn!("payload decode failed on `{}`: {e}", raw.topic);
                        continue;
                    }
                };

                let msg = Message {
                    envelope: Envelope { meta, payload },
                    address,
                    qos: raw.qos,
                    retained: raw.retained,
                };

                if typed_tx.send(Ok(msg)).await.is_err() {
                    break;
                }
            }
        });

        Ok(Subscription::new(typed_rx))
    }

    /// Subscribe with raw MQTT messages (no deserialization).
    pub async fn subscribe_raw(
        &self,
        filter: &str,
        qos: QoS,
    ) -> Result<mpsc::Receiver<RawMessage>> {
        self.subscribe_mqtt(filter, qos).await?;

        let (tx, rx) = mpsc::channel::<RawMessage>(256);
        {
            let mut guard = self.raw_subscribers.lock().await;
            guard.push(FilteredSubscriber { filter: filter.to_string(), tx });
        }
        Ok(rx)
    }

    /// Disconnect from the broker.
    pub async fn disconnect(self) -> Result<()> {
        match self.inner {
            #[cfg(feature = "embedded-broker")]
            ClientInner::InProcess { _recv_task, .. } => {
                _recv_task.abort();
            }
            ClientInner::Remote { mqtt, _event_loop } => {
                let _ = mqtt.disconnect().await;
                _event_loop.abort();
            }
        }
        Ok(())
    }

    // ── Connection constructors ─────────────────────────────────────────

    #[cfg(feature = "embedded-broker")]
    fn connect_in_process(
        link: crate::broker::BrokerLink,
        subs: Arc<tokio::sync::Mutex<Vec<FilteredSubscriber>>>,
    ) -> ClientInner {
        let link_tx = Arc::new(tokio::sync::Mutex::new(link.tx));
        let mut link_rx = link.rx;

        let recv_task = tokio::spawn(async move {
            loop {
                match link_rx.next().await {
                    Ok(Some(notification)) => {
                        if let rumqttd::Notification::Forward(fwd) = notification {
                            let raw = RawMessage {
                                topic: String::from_utf8_lossy(&fwd.publish.topic).to_string(),
                                payload: fwd.publish.payload.to_vec(),
                                qos: 0, // In-process links are QoS 0
                                retained: fwd.publish.retain,
                                user_properties: fwd
                                    .properties
                                    .as_ref()
                                    .map(|p| {
                                        p.user_properties
                                            .iter()
                                            .map(|(k, v)| (k.clone(), v.clone()))
                                            .collect()
                                    })
                                    .unwrap_or_default(),
                            };

                            let guard = subs.lock().await;
                            for sub in guard.iter() {
                                if topic_matches_filter(&raw.topic, &sub.filter) {
                                    let _ = sub.tx.try_send(raw.clone());
                                }
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::warn!("in-process link recv error: {e}");
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                }
            }
        });

        ClientInner::InProcess { link_tx, _recv_task: recv_task }
    }

    async fn connect_remote(
        identity: &ServiceIdentity,
        client_id: Option<&str>,
        host: &str,
        port: u16,
        channel_capacity: usize,
        keep_alive: Duration,
        subs: Arc<tokio::sync::Mutex<Vec<FilteredSubscriber>>>,
    ) -> Result<ClientInner> {
        let id = client_id
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{}-{}", identity.service, identity.instance_id));

        let mut opts = MqttOptions::new(&id, host, port);
        opts.set_keep_alive(keep_alive);

        let (mqtt, mut event_loop) = AsyncClient::new(opts, channel_capacity);

        let loop_handle = tokio::spawn(async move {
            use rumqttc::v5::mqttbytes::v5::Packet;
            loop {
                match event_loop.poll().await {
                    Ok(rumqttc::v5::Event::Incoming(Packet::Publish(publish))) => {
                        let raw = RawMessage {
                            topic: String::from_utf8_lossy(&publish.topic).to_string(),
                            payload: publish.payload.to_vec(),
                            qos: match publish.qos {
                                QoS::AtMostOnce => 0,
                                QoS::AtLeastOnce => 1,
                                QoS::ExactlyOnce => 2,
                            },
                            retained: publish.retain,
                            user_properties: publish
                                .properties
                                .as_ref()
                                .map(|p| {
                                    p.user_properties
                                        .iter()
                                        .map(|(k, v)| (k.clone(), v.clone()))
                                        .collect()
                                })
                                .unwrap_or_default(),
                        };

                        let guard = subs.lock().await;
                        for sub in guard.iter() {
                            if topic_matches_filter(&raw.topic, &sub.filter) {
                                let _ = sub.tx.try_send(raw.clone());
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!("MQTT event loop error: {e}");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        });

        Ok(ClientInner::Remote { mqtt, _event_loop: loop_handle })
    }

    // ── Internal helpers ────────────────────────────────────────────────

    fn build_metadata(&self) -> Metadata {
        Metadata {
            timestamp: chrono::Utc::now(),
            source_service: self.identity.service.clone(),
            source_instance: self.identity.instance_id.clone(),
            operator_id: self.identity.operator_id.clone(),
            schema_version: 1,
            correlation_id: None,
        }
    }

    async fn publish_inner<T: Serialize>(
        &self,
        topic: &str,
        event_type: &str,
        payload: &T,
        qos_override: QosOverride,
        retain: bool,
    ) -> Result<()> {
        let bytes = encode_payload(payload)?;
        let meta = self.build_metadata();
        let user_props = encode_user_properties(&meta);

        match &self.inner {
            #[cfg(feature = "embedded-broker")]
            ClientInner::InProcess { link_tx, .. } => {
                use bytes::Bytes;
                use rumqttd::protocol::{Packet, Publish, PublishProperties};

                let publish = Publish::new(
                    Bytes::copy_from_slice(topic.as_bytes()),
                    Bytes::from(bytes),
                    retain,
                );
                let properties =
                    PublishProperties { user_properties: user_props, ..Default::default() };

                let mut tx = link_tx.lock().await;
                tx.send(Packet::Publish(publish, Some(properties)))
                    .await
                    .map_err(|e| MqttError::Publish(e.to_string()))?;
                Ok(())
            }
            ClientInner::Remote { mqtt, .. } => {
                let qos = qos_override.resolve(event_type);
                let properties = rumqttc::v5::mqttbytes::v5::PublishProperties {
                    user_properties: user_props,
                    ..Default::default()
                };
                mqtt.publish_with_properties(topic, qos, retain, bytes, properties)
                    .await
                    .map_err(|e| MqttError::Publish(e.to_string()))?;
                Ok(())
            }
        }
    }

    async fn subscribe_mqtt(&self, filter: &str, qos: QoS) -> Result<()> {
        match &self.inner {
            #[cfg(feature = "embedded-broker")]
            ClientInner::InProcess { link_tx, .. } => {
                let mut tx = link_tx.lock().await;
                tx.subscribe(filter).map_err(|e| MqttError::Subscribe(e.to_string()))?;
                Ok(())
            }
            ClientInner::Remote { mqtt, .. } => {
                mqtt.subscribe(filter, qos)
                    .await
                    .map_err(|e| MqttError::Subscribe(e.to_string()))?;
                Ok(())
            }
        }
    }
}

/// MQTT topic filter matching (MQTT 5.0 §4.7).
///
/// - `+` matches exactly one topic level
/// - `#` matches zero or more trailing levels (must be last segment)
fn topic_matches_filter(topic: &str, filter: &str) -> bool {
    let mut topic_parts = topic.split('/');
    let mut filter_parts = filter.split('/').peekable();

    loop {
        match (filter_parts.next(), topic_parts.next()) {
            (Some("#"), _) => return true,
            (Some("+"), Some(_)) => continue,
            (Some(f), Some(t)) if f == t => continue,
            (None, None) => return true,
            _ => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topic_filter_exact_match() {
        assert!(topic_matches_filter("a/b/c", "a/b/c"));
        assert!(!topic_matches_filter("a/b/c", "a/b/d"));
    }

    #[test]
    fn topic_filter_single_level_wildcard() {
        assert!(topic_matches_filter("a/b/c", "a/+/c"));
        assert!(topic_matches_filter("a/x/c", "a/+/c"));
        assert!(!topic_matches_filter("a/b/c/d", "a/+/c"));
    }

    #[test]
    fn topic_filter_multi_level_wildcard() {
        assert!(topic_matches_filter("a/b/c", "a/#"));
        assert!(topic_matches_filter("a/b/c/d", "a/#"));
        assert!(topic_matches_filter("a", "a/#"));
        assert!(!topic_matches_filter("b/c", "a/#"));
    }

    #[test]
    fn topic_filter_combined_wildcards() {
        assert!(topic_matches_filter(
            "styrene/op1/omegon/inst-a/events/turn.started",
            "styrene/op1/omegon/+/events/#"
        ));
        assert!(!topic_matches_filter(
            "styrene/op1/viz/inst-a/events/turn.started",
            "styrene/op1/omegon/+/events/#"
        ));
    }

    #[test]
    fn topic_filter_length_mismatch() {
        assert!(!topic_matches_filter("a/b", "a/b/c"));
        assert!(!topic_matches_filter("a/b/c", "a/b"));
    }
}
