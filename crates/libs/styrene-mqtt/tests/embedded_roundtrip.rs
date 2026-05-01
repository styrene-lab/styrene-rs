#![cfg(feature = "embedded-broker")]
//! Integration test: full pub/sub round-trip through the embedded broker.

use serde::{Deserialize, Serialize};
use std::time::Duration;
use styrene_mqtt::{
    Client, ClientConfig, ConnectionTarget, EmbeddedBrokerBuilder, EmbeddedBrokerConfig,
    QosOverride, ServiceIdentity,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct TurnStarted {
    turn: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct ToolEnded {
    id: String,
    name: String,
    is_error: bool,
}

#[tokio::test]
async fn publish_subscribe_typed_roundtrip() {
    let (_broker, links) = EmbeddedBrokerBuilder::new(EmbeddedBrokerConfig::default())
        .add_link("publisher")
        .add_link("subscriber")
        .start()
        .expect("broker should start");

    let mut links = links;
    let pub_link = links.remove(0);
    let sub_link = links.remove(0);

    // Create publisher client.
    let publisher = Client::connect(ClientConfig::new(
        ServiceIdentity {
            operator_id: "op1".into(),
            service: "omegon".into(),
            instance_id: "test-pub".into(),
        },
        ConnectionTarget::InProcess { link: pub_link },
    ))
    .await
    .expect("publisher should connect");

    // Create subscriber client.
    let subscriber = Client::connect(ClientConfig::new(
        ServiceIdentity {
            operator_id: "op1".into(),
            service: "viz".into(),
            instance_id: "test-sub".into(),
        },
        ConnectionTarget::InProcess { link: sub_link },
    ))
    .await
    .expect("subscriber should connect");

    // Subscribe to all omegon events.
    let mut sub: styrene_mqtt::Subscription<TurnStarted> = subscriber
        .subscribe(
            "styrene/op1/omegon/+/events/turn.started",
            rumqttc::v5::mqttbytes::QoS::AtMostOnce,
        )
        .await
        .expect("subscribe should work");

    // Give the subscription time to register with the broker.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Publish a turn.started event.
    let event = TurnStarted { turn: 42 };
    publisher
        .publish("turn.started", &event, QosOverride::default())
        .await
        .expect("publish should work");

    // Receive the event.
    let msg = tokio::time::timeout(Duration::from_secs(2), sub.recv())
        .await
        .expect("should receive within timeout")
        .expect("stream should not be closed")
        .expect("message should deserialize");

    assert_eq!(msg.envelope.payload, event);
    assert_eq!(msg.address.operator_id, "op1");
    assert_eq!(msg.address.service, "omegon");
    assert_eq!(msg.address.instance_id, "test-pub");
    assert_eq!(msg.address.event_type, "turn.started");
    assert_eq!(msg.envelope.meta.source_service, "omegon");
    assert_eq!(msg.envelope.meta.operator_id, "op1");
    assert_eq!(msg.envelope.meta.schema_version, 1);
}

#[tokio::test]
async fn multiple_event_types() {
    let (_broker, links) = EmbeddedBrokerBuilder::new(EmbeddedBrokerConfig::default())
        .add_link("pub2")
        .add_link("sub2")
        .start()
        .expect("broker should start");

    let mut links = links;
    let pub_link = links.remove(0);
    let sub_link = links.remove(0);

    let publisher = Client::connect(ClientConfig::new(
        ServiceIdentity {
            operator_id: "op1".into(),
            service: "omegon".into(),
            instance_id: "inst-a".into(),
        },
        ConnectionTarget::InProcess { link: pub_link },
    ))
    .await
    .expect("connect");

    let subscriber = Client::connect(ClientConfig::new(
        ServiceIdentity {
            operator_id: "op1".into(),
            service: "auspex".into(),
            instance_id: "inst-b".into(),
        },
        ConnectionTarget::InProcess { link: sub_link },
    ))
    .await
    .expect("connect");

    // Subscribe to all events from omegon with wildcard.
    let mut sub: styrene_mqtt::Subscription<serde_json::Value> = subscriber
        .subscribe(
            "styrene/op1/omegon/+/events/#",
            rumqttc::v5::mqttbytes::QoS::AtMostOnce,
        )
        .await
        .expect("subscribe");

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Publish different event types.
    publisher
        .publish(
            "turn.started",
            &TurnStarted { turn: 1 },
            QosOverride::default(),
        )
        .await
        .expect("pub1");

    publisher
        .publish(
            "tool.ended",
            &ToolEnded {
                id: "t1".into(),
                name: "bash".into(),
                is_error: false,
            },
            QosOverride::default(),
        )
        .await
        .expect("pub2");

    // Should receive both.
    let msg1 = tokio::time::timeout(Duration::from_secs(2), sub.recv())
        .await
        .expect("timeout")
        .expect("stream")
        .expect("deser");
    assert_eq!(msg1.address.event_type, "turn.started");

    let msg2 = tokio::time::timeout(Duration::from_secs(2), sub.recv())
        .await
        .expect("timeout")
        .expect("stream")
        .expect("deser");
    assert_eq!(msg2.address.event_type, "tool.ended");
}
