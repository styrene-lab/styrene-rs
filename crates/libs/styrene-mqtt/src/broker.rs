//! Embedded MQTT 5.0 broker for the Aether event fabric.
//!
//! Intended for use by Auspex (the operator control plane), which owns the
//! broker and exposes a TCP listener for Omegon instances, Scry, Viz, and
//! other services to connect as clients. Auspex itself uses an in-process
//! link for zero-copy access to the event stream.
//!
//! Wraps [`rumqttd::Broker`] with sensible defaults. rumqttd requires that
//! in-process links are created *before* the broker starts. Use
//! [`EmbeddedBrokerBuilder`] to pre-register links, then call
//! [`start`](EmbeddedBrokerBuilder::start).

use crate::error::{MqttError, Result};
use rumqttd::local::{LinkRx, LinkTx};
use rumqttd::{Broker, Config, ConnectionSettings, RouterConfig, ServerSettings};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::thread;

/// Configuration for the embedded broker.
#[derive(Debug, Clone)]
pub struct EmbeddedBrokerConfig {
    /// Optional TCP listener for external clients. `None` = in-process only.
    pub tcp_listener: Option<TcpListenerConfig>,
    /// Max incoming message size in bytes. Default: 256 KiB.
    pub max_payload_size: usize,
    /// Max in-flight messages per connection. Default: 100.
    pub max_inflight_count: usize,
    /// Max connections. Default: 64.
    pub max_connections: usize,
}

/// TCP listener configuration for the embedded broker.
#[derive(Debug, Clone)]
pub struct TcpListenerConfig {
    pub bind_addr: SocketAddr,
}

impl Default for EmbeddedBrokerConfig {
    fn default() -> Self {
        Self {
            tcp_listener: None,
            max_payload_size: 256 * 1024,
            max_inflight_count: 100,
            max_connections: 64,
        }
    }
}

/// Handle to a running embedded MQTT broker.
pub struct EmbeddedBroker {
    _thread: thread::JoinHandle<()>,
}

/// Builder for creating an embedded broker with pre-registered in-process links.
///
/// rumqttd requires links to be created before the broker starts. Register
/// all needed links via [`add_link`](Self::add_link), then call
/// [`start`](Self::start).
///
/// # Example
///
/// ```ignore
/// let (broker, mut links) = EmbeddedBrokerBuilder::new(EmbeddedBrokerConfig::default())
///     .add_link("omegon")
///     .add_link("scry")
///     .start()?;
///
/// let omegon_link = links.remove(0);
/// let scry_link = links.remove(0);
/// ```
pub struct EmbeddedBrokerBuilder {
    config: EmbeddedBrokerConfig,
    link_ids: Vec<String>,
}

impl EmbeddedBrokerBuilder {
    pub fn new(config: EmbeddedBrokerConfig) -> Self {
        Self {
            config,
            link_ids: Vec::new(),
        }
    }

    /// Register a client ID for an in-process link.
    pub fn add_link(mut self, client_id: impl Into<String>) -> Self {
        self.link_ids.push(client_id.into());
        self
    }

    /// Build the broker and start it. Returns the broker handle and all
    /// pre-registered links in registration order.
    pub fn start(self) -> Result<(EmbeddedBroker, Vec<BrokerLink>)> {
        let rumqttd_config = build_config(&self.config);
        let broker = Broker::new(rumqttd_config);

        // Create links before start() — rumqttd requirement.
        let mut links = Vec::with_capacity(self.link_ids.len());
        for id in &self.link_ids {
            let (tx, rx) = broker
                .link(id)
                .map_err(|e| MqttError::Broker(format!("link `{id}` failed: {e}")))?;
            links.push(BrokerLink { tx, rx });
        }

        let handle = thread::Builder::new()
            .name("aether-broker".into())
            .spawn(move || {
                let mut broker = broker;
                if let Err(e) = broker.start() {
                    tracing::error!("embedded MQTT broker stopped: {e}");
                }
            })
            .map_err(|e| MqttError::Broker(format!("failed to spawn broker thread: {e}")))?;

        Ok((EmbeddedBroker { _thread: handle }, links))
    }
}

/// In-process link to the embedded broker — no TCP overhead.
pub struct BrokerLink {
    pub tx: LinkTx,
    pub rx: LinkRx,
}

fn build_config(cfg: &EmbeddedBrokerConfig) -> Config {
    let connection = ConnectionSettings {
        connection_timeout_ms: 5000,
        max_payload_size: cfg.max_payload_size,
        max_inflight_count: cfg.max_inflight_count,
        auth: None,
        external_auth: None,
        dynamic_filters: false,
    };

    let router = RouterConfig {
        max_connections: cfg.max_connections,
        max_outgoing_packet_count: 200,
        max_segment_size: 100 * 1024,
        max_segment_count: 10,
        custom_segment: None,
        initialized_filters: None,
        shared_subscriptions_strategy: Default::default(),
    };

    let mut v5 = HashMap::new();
    if let Some(tcp) = &cfg.tcp_listener {
        v5.insert(
            "v5-tcp".into(),
            ServerSettings {
                name: "v5-tcp".into(),
                listen: tcp.bind_addr,
                tls: None,
                next_connection_delay_ms: 0,
                connections: connection.clone(),
            },
        );
    }

    Config {
        id: 0,
        router,
        v4: None,
        v5: if v5.is_empty() { None } else { Some(v5) },
        ws: None,
        cluster: None,
        console: None,
        bridge: None,
        prometheus: None,
        metrics: None,
    }
}
