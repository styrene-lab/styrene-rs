# styrene-mqtt

MQTT 5.0 client and embedded broker for the Aether event fabric. Provides typed pub/sub over a fixed topic schema with metadata carried as MQTT 5.0 user properties (not in the payload body).

## Module Map

| Module | Purpose |
|--------|---------|
| `client.rs` | High-level `Client` — connect, publish typed events, subscribe with auto-deser. Supports in-process (rumqttd link) and remote (TCP via rumqttc) backends. |
| `envelope.rs` | `Envelope<T>`, `Message<T>`, `Metadata`. Encode/decode metadata as MQTT 5.0 user properties. JSON payload codec. |
| `error.rs` | `MqttError` enum (thiserror), crate-wide `Result<T>` alias. |
| `qos.rs` | QoS policy engine. Maps event types to QoS levels (0/1/2). `QosOverride` to force a specific level. |
| `stream.rs` | `Subscription<T>` — implements `futures_core::Stream`. `RawMessage` for untyped access. |
| `topic.rs` | `TopicAddress` (parse/render) and `TopicBuilder` (fluent construction of publish topics and subscription filters with wildcards). |
| `broker.rs` | Embedded rumqttd broker (feature-gated). `EmbeddedBrokerBuilder` pre-registers in-process links before start. Runs broker on a dedicated OS thread. |

## Key Types

- `Client` — main entry point. Call `Client::connect(ClientConfig)` to get a connected client.
- `ClientConfig` — identity + connection target + tuning (channel capacity, keep-alive).
- `ServiceIdentity` — `{operator_id, service, instance_id}` triple identifying this node on the fabric.
- `ConnectionTarget` — enum: `InProcess { link }` (requires `embedded-broker` feature) or `Remote { host, port }`.
- `TopicAddress` — parsed topic components. Implements `Display`.
- `TopicBuilder` — builds publish topics (all fields required) or subscribe filters (missing fields become `+`/`#` wildcards).
- `Envelope<T>` — metadata + typed payload.
- `Message<T>` — envelope + parsed `TopicAddress` + QoS + retained flag.
- `Subscription<T>` — async stream of `Result<Message<T>>`. Also has `recv()` for pull-style consumption.
- `Metadata` — timestamp, source service/instance, operator ID, schema version, optional correlation ID.

## Topic Schema

```
styrene/{operator_id}/{service}/{instance_id}/events/{event_type}
```

Fixed 6-segment hierarchy. The `events` literal is at segment index 4. Event types use dot notation (e.g. `turn.started`, `tool.ended`, `message.delta`).

Subscription wildcards: `+` for single-level, `#` for multi-level (event_type position only).

## Feature Flags

| Flag | Default | Effect |
|------|---------|--------|
| `embedded-broker` | off | Enables `broker` module, `ConnectionTarget::InProcess`, pulls in `rumqttd` dependency. |

Declared but not yet implemented: `tls`, `styrene-identity` (mentioned in lib.rs doc comments only).

## QoS Policy

Hardcoded in `qos_for_event()`:

- **QoS 0** (at most once): `message.delta`, `thinking.delta`, `tool.updated`
- **QoS 2** (exactly once): `session.reset`, `agent.completed`, `decomposition.started`, `decomposition.completed`
- **QoS 1** (at least once): everything else (catch-all default)

In-process links always operate at QoS 0 regardless of policy — this is a rumqttd limitation, not a bug.

## Test Commands

```bash
# Unit tests only (no broker dependency)
cargo test -p styrene-mqtt

# Unit + integration tests (embedded broker roundtrip)
cargo test -p styrene-mqtt --features embedded-broker
```

`tests/embedded_roundtrip.rs` is feature-gated and only compiles when `--features embedded-broker` is enabled.

## Build Notes

```bash
cargo check -p styrene-mqtt                # default features
cargo check -p styrene-mqtt --all-features # with embedded-broker
```

Both pass clean (zero warnings).

## Known Issues

1. **No TLS support** — `tls` feature is documented in lib.rs but not declared in Cargo.toml and has no implementation.

2. **No `styrene-identity` integration** — documented as future; no code exists.

3. **No reconnection logic** — remote client event loop logs errors and sleeps but never re-establishes the connection.

4. **`unwrap()` in test code only** — single occurrence in `envelope::tests::sample_meta()` for a known-valid chrono date. No `unwrap()` in library code.

5. **No doc-tests run** — the broker example in `broker.rs` is marked `ignore`, so it is not exercised.

## Fixed (this session)

- `bytes::Bytes` import gated behind `embedded-broker` feature
- Integration test gated with `#![cfg(feature = "embedded-broker")]`
- Fan-out now filters by topic (MQTT `+`/`#` wildcards via `topic_matches_filter`)
- `NotConnected` dead error variant removed
