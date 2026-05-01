//! RPC request handler — processes incoming Styrene RPC requests
//! and sends responses back to the caller.
//!
//! Registered as a ProtocolHandler for the "styrene" protocol type.
//! Handles request message types (StatusRequest, Exec, Reboot, etc.)
//! by building a response with the same request_id and delivering it
//! back via the transport layer.

use crate::services::{AuthService, Capability, MessagingService};
use crate::transport::mesh_transport::MeshTransport;
use async_trait::async_trait;
use rand_core::{OsRng, RngCore};
use rns_core::destination::DestinationName;
use rns_core::hash::AddressHash;
use rns_core::identity::PrivateIdentity;
use std::sync::Arc;
use styrene_mesh::{StyreneMessage, StyreneMessageType};
use styrene_services::protocol_registry::{HandleResult, InboundMessage, ProtocolHandler};

/// RAII guard that removes a temp file on drop (including panics).
struct TempFileGuard(String);
impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Protocol handler that processes incoming Styrene RPC requests
/// and sends responses back to the requesting peer.
///
/// Checks RBAC before executing privileged operations (Exec, Reboot).
/// StatusRequest is allowed from any peer (read-only).
pub struct RpcRequestHandler {
    transport: Arc<dyn MeshTransport>,
    signer: Arc<PrivateIdentity>,
    auth: Arc<AuthService>,
}

impl RpcRequestHandler {
    pub fn new(
        transport: Arc<dyn MeshTransport>,
        signer: Arc<PrivateIdentity>,
        auth: Arc<AuthService>,
    ) -> Self {
        Self { transport, signer, auth }
    }

    /// Build and send a response message back to the source peer.
    async fn send_response(
        &self,
        source_hash: &str,
        request_id: [u8; 16],
        response_type: StyreneMessageType,
        payload: &[u8],
    ) -> Result<(), String> {
        // source_hash is the sender's identity hash. We need their LXMF
        // delivery destination hash for routing: Hash(name_hash || identity_hash).
        let identity_bytes: [u8; 16] = hex::decode(source_hash)
            .map_err(|e| format!("invalid source hash: {e}"))?
            .try_into()
            .map_err(|_| "source hash must be 16 bytes".to_string())?;

        let delivery_addr = {
            let name = DestinationName::new("lxmf", "delivery");
            let mut combined = Vec::with_capacity(48);
            combined.extend_from_slice(name.as_name_hash_slice());
            combined.extend_from_slice(&identity_bytes);
            let truncated = rns_core::hash::address_hash(&combined);
            AddressHash::new(truncated)
        };
        let mut dest_bytes = [0u8; 16];
        dest_bytes.copy_from_slice(delivery_addr.as_slice());

        let wire_msg = StyreneMessage::with_request_id(response_type, request_id, payload);
        let wire_bytes = wire_msg.encode();

        let source_hash_addr = self.transport.identity_hash();
        let mut source_bytes = [0u8; 16];
        source_bytes.copy_from_slice(source_hash_addr.as_slice());

        let fields = serde_json::json!({
            "protocol": "styrene",
            "custom_type": "styrene.io",
            "custom_data": hex::encode(&wire_bytes),
        });

        let lxmf_payload = crate::lxmf_bridge::build_wire_message(
            source_bytes,
            dest_bytes,
            "",
            "",
            Some(fields),
            &self.signer,
        )
        .map_err(|e| format!("wire encode: {e}"))?;

        MessagingService::deliver(self.transport.as_ref(), delivery_addr, &lxmf_payload)
            .await
            .map_err(|e| format!("delivery failed: {e}"))?;

        Ok(())
    }

    fn cbor_encode(value: &serde_json::Value) -> Vec<u8> {
        let mut buf = Vec::new();
        ciborium::into_writer(value, &mut buf).unwrap_or_default();
        buf
    }

    fn handle_status_request(&self) -> Vec<u8> {
        let uptime = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        Self::cbor_encode(&serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "uptime": uptime,
        }))
    }

    fn handle_exec_request(&self, payload: &[u8]) -> Vec<u8> {
        let request: serde_json::Value =
            ciborium::from_reader(payload).unwrap_or(serde_json::Value::Null);

        let cmd = request.get("cmd").and_then(|v| v.as_str()).unwrap_or("");
        let args: Vec<&str> = request
            .get("args")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        // Validate args count and individual arg length
        if args.len() > 256 {
            return Self::cbor_encode(&serde_json::json!({
                "exit_code": -1, "stdout": "", "stderr": "too many arguments (max 256)"
            }));
        }
        const MAX_ARG_LEN: usize = 32 * 1024;
        if args.iter().any(|a| a.len() > MAX_ARG_LEN) {
            return Self::cbor_encode(&serde_json::json!({
                "exit_code": -1, "stdout": "", "stderr": "argument too long (max 32KB each)"
            }));
        }

        let output = std::process::Command::new(cmd).args(&args).output();

        const MAX_OUTPUT: usize = 1024 * 1024; // 1 MB
        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(
                    &output.stdout[..output.stdout.len().min(MAX_OUTPUT)],
                );
                let stderr = String::from_utf8_lossy(
                    &output.stderr[..output.stderr.len().min(MAX_OUTPUT)],
                );
                Self::cbor_encode(&serde_json::json!({
                    "exit_code": output.status.code().unwrap_or(-1),
                    "stdout": stdout,
                    "stderr": stderr,
                }))
            }
            Err(e) => Self::cbor_encode(&serde_json::json!({
                "exit_code": -1,
                "stdout": "",
                "stderr": format!("exec error: {e}"),
            })),
        }
    }

    fn handle_reboot_request(&self, _payload: &[u8]) -> Vec<u8> {
        Self::cbor_encode(&serde_json::json!({"accepted": true}))
    }

    fn handle_config_update(&self, payload: &[u8]) -> Vec<u8> {
        // Issue 3: Propagate CBOR deserialization errors instead of silent null
        let request: serde_json::Value = match ciborium::from_reader(payload) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[rpc-request] invalid CBOR payload: {e}");
                return Self::cbor_encode(&serde_json::json!({
                    "success": false, "verified": false, "exit_code": -1,
                    "stdout": "", "stderr": "invalid request payload",
                }));
            }
        };

        // Issue 5: Reject missing or empty profile hex
        let profile_hex = match request.get("profile").and_then(|v| v.as_str()) {
            Some(h) if !h.is_empty() => h,
            _ => {
                return Self::cbor_encode(&serde_json::json!({
                    "success": false, "verified": false, "exit_code": -1,
                    "stdout": "", "stderr": "missing or empty profile",
                }));
            }
        };

        // Size limit on hex string (4 MB hex = 2 MB decoded, matches FleetService limit)
        if profile_hex.len() > 4 * 1024 * 1024 {
            return Self::cbor_encode(&serde_json::json!({
                "success": false, "verified": false, "exit_code": -1,
                "stdout": "", "stderr": format!("profile too large: {} bytes", profile_hex.len()),
            }));
        }

        let verify = request.get("verify").and_then(|v| v.as_bool()).unwrap_or(true);

        // Decode profile bytes from hex
        let profile_bytes = match hex::decode(profile_hex) {
            Ok(b) => b,
            Err(e) => {
                return Self::cbor_encode(&serde_json::json!({
                    "success": false, "verified": false, "exit_code": -1,
                    "stdout": "", "stderr": format!("invalid profile encoding: {e}"),
                }))
            }
        };

        // Issue 1: Write to temp file with random component, O_EXCL, and 0o600 permissions
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let random_suffix = OsRng.next_u64();
        let tmp_path = format!("/tmp/styrene-profile-{ts}-{random_suffix:016x}.toml");

        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true) // O_EXCL — fails if exists
                .mode(0o600)
                .open(&tmp_path)
            {
                Ok(f) => f,
                Err(e) => {
                    return Self::cbor_encode(&serde_json::json!({
                        "success": false, "verified": false, "exit_code": -1,
                        "stdout": "", "stderr": format!("failed to create temp profile: {e}"),
                    }));
                }
            };
            if let Err(e) = file.write_all(&profile_bytes) {
                let _ = std::fs::remove_file(&tmp_path);
                eprintln!("[rpc-request] failed to write temp profile: {e}");
                return Self::cbor_encode(&serde_json::json!({
                    "success": false, "verified": false, "exit_code": -1,
                    "stdout": "", "stderr": "internal error",
                }));
            }
        }

        // RAII guard ensures cleanup on all exit paths (including panics)
        let _guard = TempFileGuard(tmp_path.clone());

        // Verify if requested
        let mut verified = false;
        if verify {
            let verify_output = std::process::Command::new("nex")
                .args(["profile", "verify", &tmp_path])
                .output();
            match verify_output {
                Ok(output) if output.status.success() => {
                    verified = true;
                }
                Ok(output) => {
                    return Self::cbor_encode(&serde_json::json!({
                        "success": false, "verified": false,
                        "exit_code": output.status.code().unwrap_or(-1),
                        "stdout": String::from_utf8_lossy(&output.stdout),
                        "stderr": format!("signature verification failed: {}",
                            String::from_utf8_lossy(&output.stderr)),
                    }));
                }
                Err(e) => {
                    eprintln!("[rpc-request] nex verify failed to run: {e}");
                    return Self::cbor_encode(&serde_json::json!({
                        "success": false, "verified": false, "exit_code": -1,
                        "stdout": "", "stderr": "profile verification unavailable",
                    }));
                }
            }
        }

        // Apply
        let apply_output = std::process::Command::new("nex")
            .args(["profile", "apply", &tmp_path])
            .output();

        match apply_output {
            Ok(output) => Self::cbor_encode(&serde_json::json!({
                "success": output.status.success(),
                "verified": verified,
                "exit_code": output.status.code().unwrap_or(-1),
                "stdout": String::from_utf8_lossy(&output.stdout),
                "stderr": String::from_utf8_lossy(&output.stderr),
            })),
            Err(e) => Self::cbor_encode(&serde_json::json!({
                "success": false, "verified": verified, "exit_code": -1,
                "stdout": "", "stderr": format!("nex apply failed: {e}"),
            })),
        }
    }
}

#[async_trait]
impl ProtocolHandler for RpcRequestHandler {
    fn name(&self) -> &str {
        "styrene-rpc-request"
    }

    fn protocol_types(&self) -> Vec<String> {
        vec!["styrene".to_string()]
    }

    async fn handle(&self, msg: &InboundMessage) -> HandleResult {
        let custom_data_hex = msg.fields.get("custom_data").and_then(|v| v.as_str());

        let Some(hex_data) = custom_data_hex else {
            return HandleResult::NotHandled;
        };

        let Ok(wire_bytes) = hex::decode(hex_data) else {
            return HandleResult::NotHandled;
        };

        let Ok(message) = StyreneMessage::decode(&wire_bytes) else {
            return HandleResult::NotHandled;
        };

        let source = &msg.source_hash;

        let (response_type, response_payload) = match message.message_type {
            StyreneMessageType::StatusRequest => {
                // Status is read-only — allowed from any peer
                (
                    StyreneMessageType::StatusResponse,
                    self.handle_status_request(),
                )
            }
            StyreneMessageType::Exec => {
                if !self.auth.check(source, &Capability::Exec) {
                    eprintln!(
                        "[rpc-request] DENIED exec from {} — insufficient privileges",
                        source
                    );
                    (
                        StyreneMessageType::ExecResult,
                        Self::cbor_encode(&serde_json::json!({
                            "exit_code": -1,
                            "stdout": "",
                            "stderr": "permission denied: caller lacks Exec capability",
                        })),
                    )
                } else {
                    (
                        StyreneMessageType::ExecResult,
                        self.handle_exec_request(&message.payload),
                    )
                }
            }
            StyreneMessageType::Reboot => {
                if !self.auth.check(source, &Capability::Reboot) {
                    eprintln!(
                        "[rpc-request] DENIED reboot from {} — insufficient privileges",
                        source
                    );
                    (
                        StyreneMessageType::RebootResult,
                        Self::cbor_encode(&serde_json::json!({
                            "accepted": false,
                            "error": "permission denied: caller lacks Reboot capability",
                        })),
                    )
                } else {
                    (
                        StyreneMessageType::RebootResult,
                        self.handle_reboot_request(&message.payload),
                    )
                }
            }
            StyreneMessageType::ConfigUpdate => {
                if !self.auth.check(source, &Capability::UpdateConfig) {
                    eprintln!(
                        "[rpc-request] DENIED config_update from {} — insufficient privileges",
                        source
                    );
                    (
                        StyreneMessageType::ConfigUpdateResult,
                        Self::cbor_encode(&serde_json::json!({
                            "success": false,
                            "verified": false,
                            "exit_code": -1,
                            "stdout": "",
                            "stderr": "permission denied: caller lacks UpdateConfig capability",
                        })),
                    )
                } else {
                    (
                        StyreneMessageType::ConfigUpdateResult,
                        self.handle_config_update(&message.payload),
                    )
                }
            }
            _ => return HandleResult::NotHandled,
        };

        let request_id = message.request_id;
        let source = msg.source_hash.clone();

        // Send response asynchronously
        match self
            .send_response(&source, request_id, response_type, &response_payload)
            .await
        {
            Ok(()) => {
                eprintln!(
                    "[rpc-request] handled {:?} from {}, sent {:?}",
                    message.message_type, source, response_type
                );
                HandleResult::Handled
            }
            Err(e) => {
                eprintln!(
                    "[rpc-request] failed to send response for {:?} from {}: {}",
                    message.message_type, source, e
                );
                HandleResult::Error(e)
            }
        }
    }
}
