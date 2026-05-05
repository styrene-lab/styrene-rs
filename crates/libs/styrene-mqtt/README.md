# styrene-mqtt

`styrene-mqtt` is the shared MQTT 5.0 transport layer for the Styrene ecosystem.

It provides:

- typed publish/subscribe over a fixed topic schema
- MQTT 5.0 metadata carried as user properties instead of payload wrappers
- a high-level async client for remote brokers
- an optional embedded broker surface for local integration tests and control-plane hosts

## Topic Schema

```text
styrene/{operator_id}/{service}/{instance_id}/events/{event_type}
```

Use `TopicBuilder` to construct publish topics and subscription filters.

## Features

- `embedded-broker`: enables the `broker` module and in-process broker support via `rumqttd`

## Install

```toml
[dependencies]
styrene-mqtt = "0.1.0"
```

Enable the embedded broker only when you need it:

```toml
[dependencies]
styrene-mqtt = { version = "0.1.0", features = ["embedded-broker"] }
```

## Testing

```bash
cargo test -p styrene-mqtt
cargo test -p styrene-mqtt --features embedded-broker
```

The embedded roundtrip integration test is feature-gated and only runs when `embedded-broker` is enabled.
