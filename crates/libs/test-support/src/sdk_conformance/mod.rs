use lxmf_sdk::{
    domain::{
        IdentityImportRequest, IdentityResolveRequest, MarkerCreateRequest, MarkerListRequest,
        MarkerUpdatePositionRequest, PaperMessageEnvelope, RemoteCommandRequest,
        RemoteCommandResponse, TelemetryQuery, TopicCreateRequest, TopicListRequest, TopicPath,
        TopicPublishRequest, VoiceSessionOpenRequest, VoiceSessionUpdateRequest,
    },
    CancelResult, Client, ConfigPatch, EventCursor, LxmfSdk, LxmfSdkAttachments, LxmfSdkIdentity,
    LxmfSdkMarkers, LxmfSdkPaper, LxmfSdkRemoteCommands, LxmfSdkTelemetry, LxmfSdkTopics,
    LxmfSdkVoiceSignaling, MessageId, RpcBackendClient, SendRequest, StartRequest,
};
use rns_rpc::e2e_harness::{
    build_http_post, build_rpc_frame, parse_http_response_body, parse_rpc_frame,
};
use rns_rpc::storage::messages::MessagesStore;
use rns_rpc::{http, RpcDaemon, RpcEvent, RpcResponse};
use serde_json::{json, Value as JsonValue};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const EVENT_LOG_OVERFLOW_TRIGGER: usize = 1_100;

struct RpcHarness {
    endpoint: String,
    daemon: Arc<Mutex<RpcDaemon>>,
    stop: Arc<AtomicBool>,
    next_request_id: AtomicU64,
    join: Option<JoinHandle<()>>,
}

impl RpcHarness {
    fn new() -> Self {
        let daemon = Arc::new(Mutex::new(RpcDaemon::with_store(
            MessagesStore::in_memory().expect("in-memory message store"),
            "sdk-test-runtime".to_owned(),
        )));

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind rpc harness listener");
        listener.set_nonblocking(true).expect("set listener non-blocking");
        let endpoint = listener.local_addr().expect("listener addr").to_string();

        let stop = Arc::new(AtomicBool::new(false));
        let daemon_for_thread = Arc::clone(&daemon);
        let stop_for_thread = Arc::clone(&stop);

        let join = thread::spawn(move || {
            while !stop_for_thread.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, addr)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                        let mut request = Vec::new();
                        if stream.read_to_end(&mut request).is_err() {
                            continue;
                        }
                        if request.is_empty() {
                            continue;
                        }
                        let response = {
                            let guard = daemon_for_thread.lock().expect("rpc daemon lock poisoned");
                            http::handle_http_request_with_peer(&guard, &request, Some(addr))
                        }
                        .unwrap_or_else(|_| {
                            b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n".to_vec()
                        });
                        let _ = stream.write_all(&response);
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });

        Self { endpoint, daemon, stop, next_request_id: AtomicU64::new(1), join: Some(join) }
    }

    fn client(&self) -> Client<RpcBackendClient> {
        Client::new(RpcBackendClient::new(self.endpoint.clone()))
    }

    fn emit_event(&self, event_type: &str, payload: JsonValue) {
        self.daemon
            .lock()
            .expect("rpc daemon lock poisoned")
            .emit_event(RpcEvent { event_type: event_type.to_owned(), payload });
    }

    fn rpc_call(&self, method: &str, params: Option<JsonValue>) -> RpcResponse {
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let frame = build_rpc_frame(request_id, method, params).expect("encode rpc frame");
        let request = build_http_post("/rpc", &self.endpoint, &frame);

        let mut stream = TcpStream::connect(&self.endpoint).expect("connect harness endpoint");
        stream.set_read_timeout(Some(Duration::from_secs(2))).expect("set rpc read timeout");
        stream.set_write_timeout(Some(Duration::from_secs(2))).expect("set rpc write timeout");
        stream.write_all(&request).expect("write rpc request");
        stream.shutdown(std::net::Shutdown::Write).expect("shutdown write side");

        let mut raw_response = Vec::new();
        stream.read_to_end(&mut raw_response).expect("read rpc response");
        let body = parse_http_response_body(&raw_response).expect("parse response body");
        parse_rpc_frame(&body).expect("decode rpc response frame")
    }
}

impl Drop for RpcHarness {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(&self.endpoint);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn base_start_request() -> StartRequest {
    serde_json::from_value(json!({
        "supported_contract_versions": [2],
        "requested_capabilities": [],
        "config": {
            "profile": "desktop-full",
            "bind_mode": "local_only",
            "auth_mode": "local_trusted",
            "overflow_policy": "reject",
            "event_stream": {
                "max_poll_events": 256,
                "max_event_bytes": 65536,
                "max_batch_bytes": 1048576,
                "max_extension_keys": 32
            },
            "idempotency_ttl_ms": 86400000,
            "redaction": {
                "enabled": true,
                "sensitive_transform": "hash",
                "break_glass_allowed": false
            }
        }
    }))
    .expect("deserialize start request")
}

fn insecure_remote_start_request() -> StartRequest {
    serde_json::from_value(json!({
        "supported_contract_versions": [2],
        "requested_capabilities": [],
        "config": {
            "profile": "desktop-full",
            "bind_mode": "remote",
            "auth_mode": "local_trusted",
            "overflow_policy": "reject",
            "event_stream": {
                "max_poll_events": 256,
                "max_event_bytes": 65536,
                "max_batch_bytes": 1048576,
                "max_extension_keys": 32
            },
            "idempotency_ttl_ms": 86400000,
            "redaction": {
                "enabled": true,
                "sensitive_transform": "hash",
                "break_glass_allowed": false
            }
        }
    }))
    .expect("deserialize insecure remote start request")
}

fn token_without_config_start_request() -> StartRequest {
    serde_json::from_value(json!({
        "supported_contract_versions": [2],
        "requested_capabilities": [],
        "config": {
            "profile": "desktop-full",
            "bind_mode": "remote",
            "auth_mode": "token",
            "overflow_policy": "reject",
            "event_stream": {
                "max_poll_events": 256,
                "max_event_bytes": 65536,
                "max_batch_bytes": 1048576,
                "max_extension_keys": 32
            },
            "idempotency_ttl_ms": 86400000,
            "redaction": {
                "enabled": true,
                "sensitive_transform": "hash",
                "break_glass_allowed": false
            }
        }
    }))
    .expect("deserialize token-mode start request")
}

fn token_remote_start_request() -> StartRequest {
    serde_json::from_value(json!({
        "supported_contract_versions": [2],
        "requested_capabilities": [],
        "config": {
            "profile": "desktop-full",
            "bind_mode": "remote",
            "auth_mode": "token",
            "overflow_policy": "reject",
            "event_stream": {
                "max_poll_events": 256,
                "max_event_bytes": 65536,
                "max_batch_bytes": 1048576,
                "max_extension_keys": 32
            },
            "idempotency_ttl_ms": 86400000,
            "redaction": {
                "enabled": true,
                "sensitive_transform": "hash",
                "break_glass_allowed": false
            },
            "rpc_backend": {
                "listen_addr": "127.0.0.1:0",
                "read_timeout_ms": 2000,
                "write_timeout_ms": 2000,
                "max_header_bytes": 8192,
                "max_body_bytes": 1048576,
                "token_auth": {
                    "issuer": "sdk-test",
                    "audience": "rns-rpc",
                    "jti_cache_ttl_ms": 60000,
                    "clock_skew_ms": 0,
                    "shared_secret": "sdk-shared-secret"
                }
            }
        }
    }))
    .expect("deserialize token remote start request")
}

fn send_request(payload_content: &str, idempotency_key: Option<&str>) -> SendRequest {
    serde_json::from_value(json!({
        "source": "source.test",
        "destination": "destination.test",
        "payload": {
            "title": "test payload",
            "content": payload_content
        },
        "idempotency_key": idempotency_key,
        "ttl_ms": null,
        "correlation_id": null,
        "extensions": {}
    }))
    .expect("deserialize send request")
}

fn overflow_patch() -> ConfigPatch {
    serde_json::from_value(json!({
        "overflow_policy": "reject"
    }))
    .expect("deserialize overflow patch")
}

#[test]
fn sdk_conformance_negotiation_success_and_no_overlap_failure() {
    let harness = RpcHarness::new();
    let client = harness.client();
    let handle = client.start(base_start_request()).expect("start with compatible capabilities");
    assert_eq!(handle.active_contract_version, 2);
    assert!(!handle.runtime_id.is_empty());

    let incompatible_client = harness.client();
    let mut incompatible_request = base_start_request();
    incompatible_request.requested_capabilities = vec!["sdk.capability.not_supported".to_owned()];
    let err = incompatible_client
        .start(incompatible_request)
        .expect_err("start must fail when no capability overlap exists");
    assert_eq!(err.machine_code, "SDK_CAPABILITY_CONTRACT_INCOMPATIBLE");
}

#[test]
fn sdk_conformance_idempotent_send_reuses_message_id() {
    let harness = RpcHarness::new();
    let client = harness.client();
    client.start(base_start_request()).expect("start");

    let first = client.send(send_request("payload-a", Some("idem-key"))).expect("first send");
    let second = client.send(send_request("payload-a", Some("idem-key"))).expect("deduped send");
    assert_eq!(first, second);
}

#[test]
fn sdk_conformance_idempotency_conflict_is_rejected() {
    let harness = RpcHarness::new();
    let client = harness.client();
    client.start(base_start_request()).expect("start");

    client.send(send_request("payload-a", Some("idem-key"))).expect("first send");
    let err = client
        .send(send_request("payload-b", Some("idem-key")))
        .expect_err("same idempotency key with different payload must fail");
    assert_eq!(err.machine_code, "SDK_VALIDATION_IDEMPOTENCY_CONFLICT");
}

#[test]
fn sdk_conformance_poll_cursor_monotonicity_and_invalid_cursor() {
    let harness = RpcHarness::new();
    let client = harness.client();
    client.start(base_start_request()).expect("start");

    harness.emit_event("health_snapshot", json!({ "status": "ok", "idx": 1 }));
    harness.emit_event("health_snapshot", json!({ "status": "ok", "idx": 2 }));

    let first = client.poll_events(None, 1).expect("first poll");
    assert_eq!(first.events.len(), 1);
    let first_seq = first.events[0].seq_no;
    let second =
        client.poll_events(Some(first.next_cursor.clone()), 1).expect("second poll with cursor");
    assert_eq!(second.events.len(), 1);
    assert!(second.events[0].seq_no > first_seq);

    let err = client
        .poll_events(Some(EventCursor("invalid-cursor-token".to_owned())), 1)
        .expect_err("invalid cursor must fail");
    assert_eq!(err.machine_code, "SDK_RUNTIME_INVALID_CURSOR");
}

#[test]
fn sdk_conformance_stream_gap_is_emitted_after_log_overflow() {
    let harness = RpcHarness::new();
    let client = harness.client();
    client.start(base_start_request()).expect("start");

    for idx in 0..EVENT_LOG_OVERFLOW_TRIGGER {
        harness.emit_event("flood", json!({ "idx": idx }));
    }

    let batch = client.poll_events(None, 8).expect("poll with overflow");
    assert!(!batch.events.is_empty(), "batch should include stream gap event");
    assert!(
        batch.events.iter().any(|event| event.event_type == "StreamGap"),
        "batch should contain StreamGap"
    );
    assert!(batch.dropped_count > 0, "dropped_count should report overflow");
}

#[test]
fn sdk_conformance_cancel_accepted_and_too_late_paths() {
    let harness = RpcHarness::new();
    let client = harness.client();
    client.start(base_start_request()).expect("start");

    let pending_message_id = "pending-cancel-message";
    let receive_response = harness.rpc_call(
        "receive_message",
        Some(json!({
            "id": pending_message_id,
            "source": "source.test",
            "destination": "destination.test",
            "title": "",
            "content": "inbound message for cancel test",
            "fields": null
        })),
    );
    assert!(receive_response.error.is_none(), "receive_message should succeed");

    let cancel_result = client.cancel(MessageId(pending_message_id.to_owned())).expect("cancel");
    assert_eq!(cancel_result, CancelResult::Accepted);

    let sent_id = client.send(send_request("already-sent", None)).expect("send");
    let sent_id_raw = sent_id.0.clone();
    let receipt_response = harness.rpc_call(
        "record_receipt",
        Some(json!({
            "message_id": sent_id_raw,
            "status": "sent",
        })),
    );
    assert!(receipt_response.error.is_none(), "record_receipt should succeed");
    let too_late = client.cancel(sent_id).expect("cancel too late path");
    assert_eq!(too_late, CancelResult::TooLateToCancel);
}

#[test]
fn sdk_conformance_configure_cas_conflict() {
    let harness = RpcHarness::new();
    let client = harness.client();
    client.start(base_start_request()).expect("start");

    let first = client.configure(0, overflow_patch()).expect("first configure");
    assert!(first.accepted);
    assert_eq!(first.revision, Some(1));

    let err = client.configure(0, overflow_patch()).expect_err("stale revision must fail");
    assert_eq!(err.machine_code, "SDK_CONFIG_CONFLICT");
}

#[test]
fn sdk_conformance_snapshot_tracks_event_position() {
    let harness = RpcHarness::new();
    let client = harness.client();
    client.start(base_start_request()).expect("start");

    harness.emit_event("policy_changed", json!({ "scope": "delivery" }));

    let snapshot = client.snapshot().expect("snapshot");
    assert_eq!(snapshot.active_contract_version, 2);
    assert!(snapshot.event_stream_position > 0);
}

#[test]
fn sdk_conformance_poll_rejects_max_over_limit() {
    let harness = RpcHarness::new();
    let client = harness.client();
    client.start(base_start_request()).expect("start");

    let err = client.poll_events(None, 257).expect_err("poll max above negotiated limit must fail");
    assert_eq!(err.machine_code, "SDK_VALIDATION_MAX_POLL_EVENTS_EXCEEDED");
}

#[test]
fn sdk_conformance_remote_bind_requires_secure_auth_mode() {
    let harness = RpcHarness::new();
    let client = harness.client();

    let err = client
        .start(insecure_remote_start_request())
        .expect_err("remote bind without token/mtls must fail");
    assert_eq!(err.machine_code, "SDK_SECURITY_REMOTE_BIND_DISALLOWED");
}

#[test]
fn sdk_conformance_token_mode_requires_token_config() {
    let harness = RpcHarness::new();
    let client = harness.client();

    let err = client
        .start(token_without_config_start_request())
        .expect_err("token mode requires token config");
    assert_eq!(err.machine_code, "SDK_SECURITY_AUTH_REQUIRED");
}

#[test]
fn sdk_conformance_token_mode_supports_multiple_authenticated_rpc_calls() {
    let harness = RpcHarness::new();
    let client = harness.client();
    client
        .start(token_remote_start_request())
        .expect("token-mode start with config should succeed");

    let first = client.snapshot().expect("first snapshot");
    let second = client.snapshot().expect("second snapshot");
    assert_eq!(first.runtime_id, second.runtime_id);
}

#[test]
fn sdk_conformance_sent_terminality_depends_on_receipt_capability() {
    let harness = RpcHarness::new();
    let client = harness.client();
    client.start(base_start_request()).expect("start");

    let message_id = client.send(send_request("terminality", None)).expect("send");
    let message_id_raw = message_id.0.clone();
    let response = harness.rpc_call(
        "record_receipt",
        Some(json!({
            "message_id": message_id_raw,
            "status": "sent",
        })),
    );
    assert!(response.error.is_none(), "record_receipt should succeed");
    let snapshot = client
        .status(MessageId(message_id.0.clone()))
        .expect("status")
        .expect("message should exist");
    assert!(!snapshot.terminal, "sent must be non-terminal with receipt_terminality");
}

#[test]
fn sdk_conformance_release_bc_domain_methods_work_through_rpc_adapter() {
    let harness = RpcHarness::new();
    let client = harness.client();
    client.start(base_start_request()).expect("start");

    let topic = client
        .topic_create(TopicCreateRequest {
            topic_path: Some(TopicPath("ops/alerts".to_string())),
            metadata: BTreeMap::new(),
            extensions: BTreeMap::new(),
        })
        .expect("topic_create");
    let listed = client
        .topic_list(TopicListRequest { cursor: None, limit: Some(16), extensions: BTreeMap::new() })
        .expect("topic_list");
    assert!(
        listed.topics.iter().any(|record| record.topic_id == topic.topic_id),
        "created topic must appear in list"
    );

    client
        .topic_publish(TopicPublishRequest {
            topic_id: topic.topic_id.clone(),
            payload: json!({ "kind": "alert", "msg": "hello" }),
            correlation_id: Some("corr-1".to_string()),
            extensions: BTreeMap::new(),
        })
        .expect("topic_publish");
    let telemetry = client
        .telemetry_query(TelemetryQuery {
            peer_id: None,
            topic_id: Some(topic.topic_id.clone()),
            from_ts_ms: None,
            to_ts_ms: None,
            limit: Some(32),
            extensions: BTreeMap::new(),
        })
        .expect("telemetry_query");
    assert!(!telemetry.is_empty(), "topic publish should produce telemetry");

    let attachment = client
        .attachment_store(lxmf_sdk::domain::AttachmentStoreRequest {
            name: "note.txt".to_string(),
            content_type: "text/plain".to_string(),
            bytes_base64: "aGVsbG8=".to_string(),
            expires_ts_ms: None,
            topic_ids: vec![topic.topic_id.clone()],
            extensions: BTreeMap::new(),
        })
        .expect("attachment_store");
    let fetched_attachment = client
        .attachment_get(attachment.attachment_id.clone())
        .expect("attachment_get")
        .expect("attachment exists");
    assert_eq!(fetched_attachment.name, "note.txt");

    let marker = client
        .marker_create(MarkerCreateRequest {
            label: "alpha".to_string(),
            position: lxmf_sdk::domain::GeoPoint { lat: 34.0, lon: -118.0, alt_m: Some(100.0) },
            topic_id: Some(topic.topic_id.clone()),
            extensions: BTreeMap::new(),
        })
        .expect("marker_create");
    let marker_list = client
        .marker_list(MarkerListRequest {
            topic_id: Some(topic.topic_id.clone()),
            cursor: None,
            limit: Some(16),
            extensions: BTreeMap::new(),
        })
        .expect("marker_list");
    assert!(
        marker_list.markers.iter().any(|record| record.marker_id == marker.marker_id),
        "created marker must appear in list"
    );
    let updated_marker = client
        .marker_update_position(MarkerUpdatePositionRequest {
            marker_id: marker.marker_id.clone(),
            position: lxmf_sdk::domain::GeoPoint { lat: 35.0, lon: -117.0, alt_m: None },
            extensions: BTreeMap::new(),
        })
        .expect("marker_update_position");
    assert_eq!(updated_marker.position.lat, 35.0);

    let identities = client.identity_list().expect("identity_list");
    assert!(!identities.is_empty(), "default identity expected");
    let imported = client
        .identity_import(IdentityImportRequest {
            bundle_base64: "eyJpZGVudGl0eSI6Im5vZGUtYiIsInB1YmxpY19rZXkiOiJub2RlLWItcHViIiwiZGlzcGxheV9uYW1lIjoiTm9kZSBCIiwiY2FwYWJpbGl0aWVzIjpbXSwiZXh0ZW5zaW9ucyI6e319".to_string(),
            passphrase: None,
            extensions: BTreeMap::new(),
        })
        .expect("identity_import");
    let resolved = client
        .identity_resolve(IdentityResolveRequest {
            hash: imported.public_key.clone(),
            extensions: BTreeMap::new(),
        })
        .expect("identity_resolve");
    assert!(resolved.is_some(), "imported identity should resolve by public key");

    let sent = client.send(send_request("paper", None)).expect("send");
    let envelope = client.paper_encode(sent.clone()).expect("paper_encode");
    client
        .paper_decode(PaperMessageEnvelope {
            uri: envelope.uri,
            transient_id: envelope.transient_id,
            destination_hint: envelope.destination_hint,
            extensions: BTreeMap::new(),
        })
        .expect("paper_decode");

    let command_response = client
        .command_invoke(RemoteCommandRequest {
            command: "ping".to_string(),
            target: Some("node-b".to_string()),
            payload: json!({ "body": "hello" }),
            timeout_ms: Some(1_000),
            extensions: BTreeMap::new(),
        })
        .expect("command_invoke");
    let correlation_id = command_response
        .payload
        .get("correlation_id")
        .and_then(JsonValue::as_str)
        .expect("command response correlation_id")
        .to_string();
    client
        .command_reply(
            correlation_id,
            RemoteCommandResponse {
                accepted: true,
                payload: json!({ "body": "pong" }),
                extensions: BTreeMap::new(),
            },
        )
        .expect("command_reply");

    let voice_session = client
        .voice_session_open(VoiceSessionOpenRequest {
            peer_id: "node-b".to_string(),
            codec_hint: Some("opus".to_string()),
            extensions: BTreeMap::new(),
        })
        .expect("voice_session_open");
    let state = client
        .voice_session_update(VoiceSessionUpdateRequest {
            session_id: voice_session.clone(),
            state: lxmf_sdk::domain::VoiceSessionState::Active,
            extensions: BTreeMap::new(),
        })
        .expect("voice_session_update");
    assert_eq!(state, lxmf_sdk::domain::VoiceSessionState::Active);
    client.voice_session_close(voice_session).expect("voice_session_close");
}
