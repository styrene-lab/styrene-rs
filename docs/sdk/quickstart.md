# SDK Quickstart

This quickstart covers a minimal `lxmf-sdk` client using the RPC backend.

## Prerequisites

- Rust toolchain matching `rust-toolchain.toml`
- Running `reticulumd` endpoint (default `127.0.0.1:4242`)
- Workspace checked out with `cargo check --workspace` passing

## Start `reticulumd`

```bash
cargo run -p reticulumd --bin reticulumd -- --rpc-listen 127.0.0.1:4242
```

For secured remote bind, use token or mTLS configuration as described in:

- `docs/contracts/sdk-v2.md`
- `docs/contracts/sdk-v2-shared-instance-auth.md`

## Minimal SDK Client

```rust
use lxmf_sdk::{
    Client, LxmfSdk, RpcBackendClient, SendRequest, StartRequest,
};
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(RpcBackendClient::new("127.0.0.1:4242".to_owned()));

    let start: StartRequest = serde_json::from_value(json!({
        "supported_contract_versions": [2],
        "requested_capabilities": ["sdk.capability.cursor_replay"],
        "config": {
            "profile": "desktop-full",
            "bind_mode": "local_only",
            "auth_mode": "local_trusted",
            "overflow_policy": "reject",
            "event_stream": {
                "max_poll_events": 128,
                "max_event_bytes": 32768,
                "max_batch_bytes": 1048576,
                "max_extension_keys": 32
            },
            "idempotency_ttl_ms": 86400000,
            "redaction": {
                "enabled": true,
                "sensitive_transform": "hash",
                "break_glass_allowed": false,
                "break_glass_ttl_ms": null
            },
            "rpc_backend": {
                "listen_addr": "127.0.0.1:4242",
                "read_timeout_ms": 5000,
                "write_timeout_ms": 5000,
                "max_header_bytes": 16384,
                "max_body_bytes": 1048576
            },
            "extensions": {}
        }
    }))?;

    let handle = client.start(start)?;
    println!("runtime_id={} contract={}", handle.runtime_id, handle.active_contract_version);
    Ok(())
}
```

## Send and Poll Events

```rust
let send: SendRequest = serde_json::from_value(json!({
    "source": "example.service",
    "destination": "example.peer",
    "payload": {"title": "hello", "content": "sdk quickstart"},
    "idempotency_key": null,
    "ttl_ms": 30000,
    "correlation_id": "quickstart-send",
    "extensions": {}
}))?;

let message_id = client.send(send)?;
let batch = client.poll_events(None, 16)?;
println!("queued message_id={} events={}", message_id.0, batch.events.len());
```

## Next Steps

- Operational config patterns: `docs/sdk/configuration-profiles.md`
- Runtime lifecycle and cursor patterns: `docs/sdk/lifecycle-and-events.md`
- Capability-driven feature use: `docs/sdk/advanced-embedding.md`
