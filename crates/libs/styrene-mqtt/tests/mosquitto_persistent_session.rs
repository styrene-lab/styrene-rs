use std::net::TcpStream;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use styrene_a2a::{AgentEnvelope, AgentEnvelopeKind, AgentId, RootOperationId, RuntimeId};
use styrene_mqtt::MqttA2aClient;

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
        &RootOperationId::new("session-redelivery").unwrap(),
        Some("offline-task".into()),
        "offline-task",
        1,
        now,
        "a2a.message.v1",
        br#"{"role":"user","text":"queued while offline"}"#.to_vec(),
    );
    envelope.expires_at_ms = Some(now + 60_000);
    envelope
}

#[tokio::test]
#[ignore = "requires a live MQTT 5 broker configured by STYRENE_MQTT_TEST_URL"]
async fn persistent_session_redelivers_qos_one_command_after_reconnect() {
    let Some((host, port)) = broker() else {
        eprintln!("skipping: no MQTT 5 broker");
        return;
    };
    let nonce = now_ms();
    let tenant = format!("session-{nonce}");
    let source = format!("source-{nonce}");
    let target = format!("target-{nonce}");
    let client_id = format!("persistent-sub-{nonce}");

    // Establish a persistent subscription, then disconnect cleanly while retaining broker session state.
    let mut subscriber = MqttA2aClient::connect_persistent(
        &tenant,
        &client_id,
        &host,
        port,
        Duration::from_secs(5),
        8,
        Duration::from_secs(60),
    );
    subscriber.subscribe_agent(&target).await.unwrap();
    subscriber.poll_transport().await.unwrap(); // CONNACK
    subscriber.poll_transport().await.unwrap(); // SUBACK
    subscriber.disconnect().await.unwrap();
    subscriber.poll_transport().await.unwrap(); // flush DISCONNECT
    drop(subscriber);

    // Publish while the persistent subscriber is offline. Broker must queue this QoS 1 command.
    let mut publisher = MqttA2aClient::connect(
        &tenant,
        format!("session-pub-{nonce}"),
        &host,
        port,
        Duration::from_secs(5),
        8,
    );
    publisher.poll_transport().await.unwrap();
    let expected = command(&source, &target);
    publisher.publish(&expected, now_ms()).await.unwrap();
    publisher.flush_publish().await.unwrap();

    // Reconnect with the same client ID and persistent-session settings. Do not resubscribe.
    let mut subscriber = MqttA2aClient::connect_persistent(
        &tenant,
        &client_id,
        &host,
        port,
        Duration::from_secs(5),
        8,
        Duration::from_secs(60),
    );
    let received = tokio::time::timeout(Duration::from_secs(5), subscriber.recv(now_ms()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.envelope, expected);

    subscriber.disconnect().await.unwrap();
    publisher.disconnect().await.unwrap();
}
