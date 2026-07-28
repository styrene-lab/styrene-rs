use std::time::Duration;

use rumqttc::v5::mqttbytes::v5::Packet;
use rumqttc::v5::{AsyncClient, Event, EventLoop, MqttOptions};
use rumqttc::Outgoing;
use styrene_a2a::AgentEnvelope;

use crate::{
    decode_publication, publication_for, A2aTopic, MqttA2aError, ReceivedA2aEnvelope, Result,
};

pub struct MqttA2aClient {
    client: AsyncClient,
    event_loop: EventLoop,
    tenant: String,
}

impl MqttA2aClient {
    pub fn connect(
        tenant: impl Into<String>,
        client_id: impl Into<String>,
        host: impl Into<String>,
        port: u16,
        keep_alive: Duration,
        channel_capacity: usize,
    ) -> Self {
        let mut options = MqttOptions::new(client_id, host, port);
        options.set_keep_alive(keep_alive);
        let (client, event_loop) = AsyncClient::new(options, channel_capacity);
        Self { client, event_loop, tenant: tenant.into() }
    }

    pub async fn publish(&self, envelope: &AgentEnvelope, now_ms: u64) -> Result<()> {
        let publication = publication_for(&self.tenant, envelope, now_ms)?;
        self.client
            .publish_with_properties(
                publication.topic,
                publication.qos,
                publication.retain,
                publication.payload,
                publication.properties,
            )
            .await
            .map_err(|error| MqttA2aError::Publish(error.to_string()))
    }

    pub async fn subscribe_agent(&self, agent_id: &str) -> Result<()> {
        let filter = A2aTopic::agent_filter(&self.tenant, agent_id)?;
        self.client
            .subscribe(filter, rumqttc::v5::mqttbytes::QoS::AtLeastOnce)
            .await
            .map_err(|error| MqttA2aError::Subscribe(error.to_string()))
    }

    pub async fn poll_transport(&mut self) -> Result<()> {
        self.event_loop
            .poll()
            .await
            .map(|_| ())
            .map_err(|error| MqttA2aError::Connection(error.to_string()))
    }

    pub async fn flush_publish(&mut self) -> Result<()> {
        loop {
            match self.event_loop.poll().await {
                Ok(Event::Outgoing(Outgoing::Publish(_))) => return Ok(()),
                Ok(_) => {}
                Err(error) => return Err(MqttA2aError::Connection(error.to_string())),
            }
        }
    }

    pub async fn disconnect(&self) -> Result<()> {
        self.client.disconnect().await.map_err(|error| MqttA2aError::Connection(error.to_string()))
    }

    pub async fn recv(&mut self, now_ms: u64) -> Result<ReceivedA2aEnvelope> {
        loop {
            match self.event_loop.poll().await {
                Ok(Event::Incoming(Packet::Publish(publish))) => {
                    let topic = String::from_utf8_lossy(&publish.topic);
                    let properties = publish.properties.unwrap_or_default();
                    return decode_publication(
                        &topic,
                        &publish.payload,
                        publish.retain,
                        &properties,
                        now_ms,
                    );
                }
                Ok(_) => {}
                Err(error) => return Err(MqttA2aError::Connection(error.to_string())),
            }
        }
    }
}
