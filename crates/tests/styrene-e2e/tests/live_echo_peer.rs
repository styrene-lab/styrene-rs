//! Live basics against a deployed `styrened` echo peer.
//!
//! This suite is opt-in. It needs a reachable node running `auto_reply.mode =
//! "echo"` and is skipped unless both variables are set:
//!
//! ```text
//! STYRENE_LIVE_PEER=192.0.2.10:4242
//! STYRENE_LIVE_PEER_DESTINATION=<32 hex chars of the peer's lxmf.delivery hash>
//! cargo test -p styrene-e2e --test live_echo_peer -- --ignored --nocapture
//! ```
//!
//! The peer does not have to announce on its own: the probe announces itself,
//! then requests a path to the configured destination. Evidence for every
//! stage is written as JSON to `STYRENE_LIVE_EVIDENCE` or
//! `target/live-peer/evidence.json`.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rns_core::hash::AddressHash;
use rns_core::transport::iface::InterfaceState;
use serde_json::{Value, json};
use styrene_e2e::node::{TestNode, TestNodeBuilder};
use styrened::storage::messages::MessageRecord;

const POLL: Duration = Duration::from_millis(200);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const PATH_TIMEOUT: Duration = Duration::from_secs(30);
const ECHO_TIMEOUT: Duration = Duration::from_secs(45);
const RECEIPT_TIMEOUT: Duration = Duration::from_secs(30);
/// Comfortably above the packet MDU so the second echo travels as a resource.
const RESOURCE_BODY_BYTES: usize = 1_500;

struct Target {
    addr: SocketAddr,
    destination_hex: String,
    destination: AddressHash,
}

fn target() -> Option<Target> {
    let addr = std::env::var("STYRENE_LIVE_PEER").ok()?;
    let destination_hex = std::env::var("STYRENE_LIVE_PEER_DESTINATION").ok()?;
    let addr: SocketAddr = addr.parse().expect("STYRENE_LIVE_PEER must be host:port");
    let bytes = hex::decode(&destination_hex).expect("destination must be hex");
    let bytes: [u8; 16] = bytes.try_into().expect("destination must be 16 bytes");
    Some(Target { addr, destination_hex, destination: AddressHash::new(bytes) })
}

struct Evidence {
    started: Instant,
    stages: Vec<Value>,
}

impl Evidence {
    fn new() -> Self {
        Self { started: Instant::now(), stages: Vec::new() }
    }

    fn record(&mut self, stage: &str, detail: Value) {
        let elapsed_ms = self.started.elapsed().as_millis();
        eprintln!("[live-peer] +{elapsed_ms}ms {stage} {detail}");
        self.stages.push(json!({ "stage": stage, "elapsed_ms": elapsed_ms, "detail": detail }));
    }

    fn write(&self, outcome: &str, target: &Target, probe: &TestNode) {
        let path = std::env::var("STYRENE_LIVE_EVIDENCE")
            .map_or_else(|_| PathBuf::from("target/live-peer/evidence.json"), PathBuf::from);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let document = json!({
            "schema_version": 1,
            "suite": "live_echo_peer",
            "outcome": outcome,
            "peer": { "address": target.addr.to_string(), "destination": target.destination_hex },
            "probe": { "identity": probe.identity_hash, "destination": probe.delivery_hash },
            "recorded_at": unix_seconds(),
            "stages": self.stages,
        });
        std::fs::write(&path, serde_json::to_vec_pretty(&document).expect("evidence json"))
            .expect("write evidence");
        eprintln!("[live-peer] evidence written to {}", path.display());
    }
}

fn unix_seconds() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Waits for an interface to reach a connected state. With `wanted` set, only
/// that interface counts, so a client that was just cancelled cannot satisfy
/// the wait while its last snapshot is still being torn down.
async fn await_interface_up(
    probe: &TestNode,
    wanted: Option<AddressHash>,
    timeout: Duration,
) -> AddressHash {
    let deadline = Instant::now() + timeout;
    loop {
        let snapshots = probe.transport.interface_snapshots().await;
        if let Some(up) = snapshots.iter().find(|s| {
            wanted.is_none_or(|hash| s.hash == hash)
                && matches!(s.state, InterfaceState::Connected | InterfaceState::Active)
        }) {
            return up.hash;
        }
        assert!(Instant::now() < deadline, "no interface came up within {timeout:?}");
        tokio::time::sleep(POLL).await;
    }
}

/// Request a path until the peer answers; the peer may never announce
/// unsolicited, so the request is repeated on a short cadence.
async fn await_path_with_requests(probe: &TestNode, destination: &AddressHash, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    let mut next_request = Instant::now();
    loop {
        if probe.app_context.transport().query_path(destination).await.is_some()
            && probe.app_context.transport().resolve_identity(destination).await.is_some()
        {
            return;
        }
        if Instant::now() >= next_request {
            probe.transport.request_path(destination, None, None).await;
            next_request = Instant::now() + Duration::from_secs(4);
        }
        assert!(
            Instant::now() < deadline,
            "peer {} did not answer a path request within {timeout:?}",
            hex::encode(destination.as_slice())
        );
        tokio::time::sleep(POLL).await;
    }
}

fn echo_request_id(message: &MessageRecord) -> Option<&str> {
    message.fields.as_ref()?.get("styrene_echo")?.get("request_id")?.as_str()
}

async fn await_echo(probe: &TestNode, request_id: &str, timeout: Duration) -> MessageRecord {
    let deadline = Instant::now() + timeout;
    loop {
        {
            let store = probe.app_context.store().lock().expect("store");
            let messages = store
                .list_messages(styrene_ipc::types::MAX_MESSAGE_QUERY_LIMIT as usize, None)
                .expect("list messages");
            if let Some(echo) = messages
                .into_iter()
                .find(|m| m.direction == "in" && echo_request_id(m) == Some(request_id))
            {
                return echo;
            }
        }
        if Instant::now() >= deadline {
            let store = probe.app_context.store().lock().expect("store");
            let messages = store
                .list_messages(styrene_ipc::types::MAX_MESSAGE_QUERY_LIMIT as usize, None)
                .expect("list messages");
            for m in &messages {
                let canonical_fields = store
                    .canonical_inbound(&m.id)
                    .ok()
                    .flatten()
                    .and_then(|c| c.fields_msgpack)
                    .map(hex::encode);
                eprintln!(
                    "[live-peer] store {} dir={} src={} title={:?} receipt={:?} fields={:?} wire_fields={:?}",
                    m.id,
                    m.direction,
                    m.source,
                    m.title,
                    m.receipt_status,
                    m.fields,
                    canonical_fields
                );
            }
            panic!(
                "no echo for {request_id} within {timeout:?} ({} messages in store)",
                messages.len()
            );
        }
        tokio::time::sleep(POLL).await;
    }
}

async fn await_receipt(probe: &TestNode, message_id: &str, timeout: Duration) -> String {
    let deadline = Instant::now() + timeout;
    loop {
        let status = {
            let store = probe.app_context.store().lock().expect("store");
            store.get_message(message_id).expect("get message").and_then(|m| m.receipt_status)
        };
        if let Some(delivered) = status.as_ref().filter(|s| s.starts_with("delivered")) {
            return delivered.clone();
        }
        assert!(
            Instant::now() < deadline,
            "message {message_id} was not marked delivered within {timeout:?} (status {status:?})"
        );
        tokio::time::sleep(POLL).await;
    }
}

async fn echo_round_trip(
    probe: &TestNode,
    target: &Target,
    evidence: &mut Evidence,
    label: &str,
    content: String,
) {
    let sent = Instant::now();
    let id = probe.send_chat(&target.destination_hex, &content).await.expect("send chat");
    evidence.record(&format!("{label}.sent"), json!({ "id": id, "bytes": content.len() }));
    let echo = await_echo(probe, &id, ECHO_TIMEOUT).await;
    assert_eq!(echo.content, content, "{label}: echo body must match what was sent");
    assert_eq!(echo.title, "[auto-reply]", "{label}: echo carries the auto-reply title");
    let response_flag = echo
        .fields
        .as_ref()
        .and_then(|f| f.get("styrene_echo"))
        .and_then(|e| e.get("response"))
        .and_then(Value::as_bool);
    assert_eq!(response_flag, Some(true), "{label}: echo marks itself as a response");
    evidence.record(
        &format!("{label}.echoed"),
        json!({
            "id": echo.id,
            "source": echo.source,
            "round_trip_ms": sent.elapsed().as_millis(),
        }),
    );
    let receipt = await_receipt(probe, &id, RECEIPT_TIMEOUT).await;
    evidence.record(&format!("{label}.receipt"), json!({ "status": receipt }));
}

#[tokio::test]
#[ignore = "requires a reachable live echo peer; see the module documentation"]
async fn live_echo_peer_basics() {
    let Some(target) = target() else {
        eprintln!("[live-peer] STYRENE_LIVE_PEER not set; skipping");
        return;
    };
    let mut evidence = Evidence::new();
    let stamp = unix_seconds();
    let probe =
        TestNodeBuilder::new(&format!("live-probe-{stamp}")).tcp_client(target.addr).build().await;
    evidence.record(
        "probe.started",
        json!({ "identity": probe.identity_hash, "destination": probe.delivery_hash }),
    );

    let iface = await_interface_up(&probe, None, CONNECT_TIMEOUT).await;
    evidence.record("interface.connected", json!({ "interface": iface.to_string() }));

    probe.announce().await;
    evidence.record("probe.announced", json!({}));

    await_path_with_requests(&probe, &target.destination, PATH_TIMEOUT).await;
    let hops = probe.app_context.transport().query_path(&target.destination).await;
    evidence.record("peer.resolved", json!({ "path": format!("{hops:?}") }));

    echo_round_trip(&probe, &target, &mut evidence, "packet", format!("live echo packet {stamp}"))
        .await;

    let filler = "resource ".repeat(RESOURCE_BODY_BYTES / 9 + 1);
    echo_round_trip(
        &probe,
        &target,
        &mut evidence,
        "resource",
        format!("live echo resource {stamp} {}", &filler[..RESOURCE_BODY_BYTES]),
    )
    .await;

    probe.cancel_interface(&iface).await;
    evidence.record("interface.dropped", json!({ "interface": iface.to_string() }));
    let reattached = probe.attach_tcp_client(target.addr).await;
    let iface = await_interface_up(&probe, Some(reattached), CONNECT_TIMEOUT).await;
    evidence.record("interface.reconnected", json!({ "interface": iface.to_string() }));
    // The path learned over the dropped client must not keep routing sends
    // into it; after a fresh announce and path request the path has to move
    // to the interface that is actually up.
    let stale = probe.app_context.transport().query_path(&target.destination).await;
    evidence.record("path.after_drop", json!({ "path": format!("{stale:?}") }));
    probe.announce().await;
    let deadline = Instant::now() + PATH_TIMEOUT;
    let mut next_request = Instant::now();
    loop {
        let path = probe.app_context.transport().query_path(&target.destination).await;
        if path.is_some_and(|(_, via)| via == iface) {
            evidence.record("path.moved", json!({ "path": format!("{path:?}") }));
            break;
        }
        if Instant::now() >= next_request {
            probe.transport.request_path(&target.destination, None, None).await;
            next_request = Instant::now() + Duration::from_secs(4);
        }
        assert!(
            Instant::now() < deadline,
            "path to the peer stayed on the dropped interface: {path:?}"
        );
        tokio::time::sleep(POLL).await;
    }
    echo_round_trip(
        &probe,
        &target,
        &mut evidence,
        "after-reconnect",
        format!("live echo after reconnect {stamp}"),
    )
    .await;

    evidence.write("passed", &target, &probe);
    probe.shutdown().await;
}
