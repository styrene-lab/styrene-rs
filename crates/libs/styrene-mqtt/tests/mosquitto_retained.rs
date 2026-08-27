use std::net::TcpStream;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use styrene_a2a::{AgentEnvelope, AgentEnvelopeKind, AgentId, RootOperationId, RuntimeId};
use styrene_mqtt::{A2aTopic, A2aTopicKind, MqttA2aClient, MQTT_A2A_CONTENT_TYPE};

fn broker() -> Option<(String, u16)> {
    let raw =
        std::env::var("STYRENE_MQTT_TEST_URL").unwrap_or_else(|_| "mqtt://127.0.0.1:1883".into());
    let authority = raw.strip_prefix("mqtt://")?;
    let (host, port) = authority.rsplit_once(':')?;
    let port = port.parse().ok()?;
    TcpStream::connect_timeout(&format!("{host}:{port}").parse().ok()?, Duration::from_millis(250))
        .ok()?;
    Some((host.to_owned(), port))
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

fn command(source: &str, target: &str) -> AgentEnvelope {
    let now = now_ms();
    let mut envelope = AgentEnvelope::new(
        AgentEnvelopeKind::Command,
        &AgentId::new(source).unwrap(),
        RuntimeId::new(),
        &AgentId::new(target).unwrap(),
        &RootOperationId::new("mosquitto-retained").unwrap(),
        Some("retained-task".into()),
        "retained-task",
        1,
        now,
        "a2a.message.v1",
        br#"{"role":"user"}"#.to_vec(),
    );
    envelope.expires_at_ms = Some(now + 30_000);
    envelope
}

#[tokio::test]
#[ignore = "requires a live MQTT 5 broker configured by STYRENE_MQTT_TEST_URL"]
async fn retained_command_is_rejected_from_real_broker() {
    let Some((host, port)) = broker() else {
        eprintln!("skipping: no MQTT 5 broker");
        return;
    };
    let nonce = now_ms();
    let tenant = format!("retain-{nonce}");
    let source = format!("source-{nonce}");
    let target = format!("target-{nonce}");
    let envelope = command(&source, &target);
    let topic = A2aTopic::new(&tenant, &target, A2aTopicKind::Inbox).unwrap().render();

    let mut options = rumqttc::v5::MqttOptions::new(format!("retain-pub-{nonce}"), &host, port);
    options.set_keep_alive(Duration::from_secs(5));
    let (publisher, mut event_loop) = rumqttc::v5::AsyncClient::new(options, 8);
    publisher
        .publish_with_properties(
            topic.clone(),
            rumqttc::v5::mqttbytes::QoS::AtLeastOnce,
            true,
            envelope.encode_cbor().unwrap(),
            rumqttc::v5::mqttbytes::v5::PublishProperties {
                correlation_data: Some(bytes::Bytes::copy_from_slice(&envelope.message_id)),
                content_type: Some(MQTT_A2A_CONTENT_TYPE.into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    loop {
        if matches!(
            event_loop.poll().await.unwrap(),
            rumqttc::v5::Event::Incoming(rumqttc::v5::Incoming::PubAck(_))
        ) {
            break;
        }
    }

    let mut subscriber = MqttA2aClient::connect(
        &tenant,
        format!("retain-sub-{nonce}"),
        &host,
        port,
        Duration::from_secs(5),
        8,
    );
    subscriber.subscribe_agent(&target).await.unwrap();
    subscriber.poll_transport().await.unwrap();
    subscriber.poll_transport().await.unwrap();
    let error = tokio::time::timeout(Duration::from_secs(3), subscriber.recv(now_ms()))
        .await
        .unwrap()
        .unwrap_err();
    assert!(matches!(error, styrene_mqtt::MqttA2aError::RetainedNonSnapshot));

    // Clear retained state and flush the cleanup publication.
    publisher
        .publish(topic, rumqttc::v5::mqttbytes::QoS::AtLeastOnce, true, Vec::<u8>::new())
        .await
        .unwrap();
    loop {
        if matches!(
            event_loop.poll().await.unwrap(),
            rumqttc::v5::Event::Incoming(rumqttc::v5::Incoming::PubAck(_))
        ) {
            break;
        }
    }
    subscriber.disconnect().await.unwrap();
    publisher.disconnect().await.unwrap();
}
