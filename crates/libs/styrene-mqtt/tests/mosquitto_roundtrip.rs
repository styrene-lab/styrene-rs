use std::net::TcpStream;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use styrene_a2a::{AgentEnvelope, AgentEnvelopeKind, AgentId, RootOperationId, RuntimeId};
use styrene_mqtt::{A2aTopicKind, MqttA2aClient, MQTT_A2A_CONTENT_TYPE};

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

fn command(source: &str, target: &str, sequence: u64) -> AgentEnvelope {
    let now = now_ms();
    let mut envelope = AgentEnvelope::new(
        AgentEnvelopeKind::Command,
        &AgentId::new(source).unwrap(),
        RuntimeId::new(),
        &AgentId::new(target).unwrap(),
        &RootOperationId::new("mosquitto-smoke").unwrap(),
        Some(format!("task-{sequence}")),
        format!("task-{sequence}"),
        sequence,
        now,
        "a2a.message.v1",
        format!(r#"{{"sequence":{sequence}}}"#).into_bytes(),
    );
    envelope.expires_at_ms = Some(now + 30_000);
    envelope
}

#[tokio::test]
async fn mosquitto_roundtrip_and_reconnect() {
    let Some((host, port)) = broker() else {
        eprintln!("skipping: no MQTT 5 broker at STYRENE_MQTT_TEST_URL");
        return;
    };
    let nonce = now_ms();
    let tenant = format!("test-{nonce}");
    let source = format!("source-{nonce}");
    let target = format!("target-{nonce}");

    let mut subscriber = MqttA2aClient::connect(
        &tenant,
        format!("sub-{nonce}"),
        &host,
        port,
        Duration::from_secs(5),
        16,
    );
    subscriber.subscribe_agent(&target).await.unwrap();
    subscriber.poll_transport().await.unwrap();
    subscriber.poll_transport().await.unwrap();
    let mut publisher = MqttA2aClient::connect(
        &tenant,
        format!("pub-{nonce}"),
        &host,
        port,
        Duration::from_secs(5),
        16,
    );

    // Drive the subscriber event loop until CONNACK/SUBACK have arrived.
    let first = command(&source, &target, 1);
    tokio::time::sleep(Duration::from_millis(200)).await;
    publisher.poll_transport().await.unwrap();
    publisher.publish(&first, now_ms()).await.unwrap();
    publisher.flush_publish().await.unwrap();
    let received = tokio::time::timeout(Duration::from_secs(5), subscriber.recv(now_ms()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.envelope, first);
    assert_eq!(received.topic.tenant, tenant);
    assert_eq!(received.topic.kind, A2aTopicKind::Inbox);

    subscriber.disconnect().await.unwrap();
    let mut subscriber = MqttA2aClient::connect(
        &tenant,
        format!("sub-reconnect-{nonce}"),
        &host,
        port,
        Duration::from_secs(5),
        16,
    );
    subscriber.subscribe_agent(&target).await.unwrap();
    subscriber.poll_transport().await.unwrap();
    subscriber.poll_transport().await.unwrap();
    let second = command(&source, &target, 2);
    publisher.publish(&second, now_ms()).await.unwrap();
    publisher.flush_publish().await.unwrap();
    let received = tokio::time::timeout(Duration::from_secs(5), subscriber.recv(now_ms()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.envelope, second);
    assert_eq!(
        styrene_mqtt::payload_content_type(&received.envelope).unwrap(),
        "application/a2a+json"
    );
    assert_eq!(MQTT_A2A_CONTENT_TYPE, "application/styrene-a2a+cbor;v=1");

    subscriber.disconnect().await.unwrap();
    publisher.disconnect().await.unwrap();
}
