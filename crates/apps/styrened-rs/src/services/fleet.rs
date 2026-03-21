//! FleetService — RPC client for remote device management.
//!
//! Owns: 7.1 RPC dispatch, 7.2 fleet state, 7.3 remote exec.
//! Package: F (stub), expanded here.
//!
//! Provides request-response semantics over LXMF messaging using the
//! Styrene wire protocol. Handles request correlation via 16-byte
//! random request_ids, timeout management, and response routing.

use crate::transport::mesh_transport::MeshTransport;
use rns_core::hash::AddressHash;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use styrene_ipc::types::{ExecResult, RebootResult, RemoteStatusInfo};
use styrene_mesh::{StyreneMessage, StyreneMessageType};
use tokio::sync::oneshot;

/// A pending RPC request awaiting response.
pub(crate) struct PendingRequest {
    pub(crate) tx: oneshot::Sender<StyreneMessage>,
    pub(crate) created_at: std::time::Instant,
    #[allow(dead_code)]
    pub(crate) dest_hash: String,
}

/// Service managing fleet RPC operations (status, exec, reboot, etc.).
pub struct FleetService {
    /// Pending requests keyed by 16-byte request_id.
    pub(crate) pending: Mutex<HashMap<[u8; 16], PendingRequest>>,
    /// Transport for sending RPC messages (None until signer wired).
    transport: Mutex<Option<Arc<dyn MeshTransport>>>,
    /// Signing key for LXMF messages.
    signer: Mutex<Option<Arc<rns_core::identity::PrivateIdentity>>>,
}

impl FleetService {
    /// Create a stub for tests.
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            transport: Mutex::new(None),
            signer: Mutex::new(None),
        }
    }

    /// Wire transport and signer for outbound RPC.
    /// Called by AppContext.set_signer() when identity becomes available.
    pub fn set_signer(
        &self,
        transport: Arc<dyn MeshTransport>,
        signer: Arc<rns_core::identity::PrivateIdentity>,
    ) {
        *self.transport.lock().unwrap() = Some(transport);
        *self.signer.lock().unwrap() = Some(signer);
    }

    /// Query remote device status via RPC.
    pub async fn device_status(
        &self,
        dest_hash: &str,
        timeout: Option<u64>,
    ) -> Result<RemoteStatusInfo, std::io::Error> {
        let timeout = Duration::from_secs(timeout.unwrap_or(30));
        let response = self
            .rpc_call(dest_hash, StyreneMessageType::StatusRequest, &[], timeout)
            .await?;

        // Decode response payload
        if response.message_type != StyreneMessageType::StatusResponse {
            return Err(std::io::Error::other(format!(
                "unexpected response type: {:?}",
                response.message_type
            )));
        }

        // Parse StatusResponse from msgpack payload
        let payload: HashMap<String, rmpv::Value> =
            rmp_serde::from_slice(&response.payload)
                .map_err(|e| std::io::Error::other(format!("decode status response: {e}")))?;

        let mut info = RemoteStatusInfo::default();
        info.destination_hash = dest_hash.to_string();
        if let Some(uptime) = payload.get("uptime").and_then(|v| v.as_u64()) {
            info.uptime = Some(uptime);
        }
        if let Some(version) = payload.get("version").and_then(|v| v.as_str()) {
            info.daemon_version = Some(version.to_string());
        }
        Ok(info)
    }

    /// Execute a command on a remote device via RPC.
    pub async fn exec(
        &self,
        dest_hash: &str,
        cmd: &str,
        args: &[String],
        timeout: Option<u64>,
    ) -> Result<ExecResult, std::io::Error> {
        let timeout = Duration::from_secs(timeout.unwrap_or(60));

        // Build exec payload
        let payload = rmp_serde::to_vec(&rmpv::Value::Map(vec![
            (rmpv::Value::from("cmd"), rmpv::Value::from(cmd)),
            (
                rmpv::Value::from("args"),
                rmpv::Value::Array(args.iter().map(|a| rmpv::Value::from(a.as_str())).collect()),
            ),
        ]))
        .map_err(|e| std::io::Error::other(format!("encode exec request: {e}")))?;

        let response = self
            .rpc_call(dest_hash, StyreneMessageType::Exec, &payload, timeout)
            .await?;

        if response.message_type != StyreneMessageType::ExecResult {
            return Err(std::io::Error::other(format!(
                "unexpected response type: {:?}",
                response.message_type
            )));
        }

        let result: HashMap<String, rmpv::Value> =
            rmp_serde::from_slice(&response.payload)
                .map_err(|e| std::io::Error::other(format!("decode exec result: {e}")))?;

        let mut exec_result = ExecResult::default();
        exec_result.exit_code = result.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(-1) as i32;
        exec_result.stdout = result.get("stdout").and_then(|v| v.as_str()).unwrap_or("").to_string();
        exec_result.stderr = result.get("stderr").and_then(|v| v.as_str()).unwrap_or("").to_string();

        Ok(exec_result)
    }

    /// Reboot a remote device via RPC.
    pub async fn reboot_device(
        &self,
        dest_hash: &str,
        delay: Option<u64>,
        timeout: Option<u64>,
    ) -> Result<RebootResult, std::io::Error> {
        let timeout = Duration::from_secs(timeout.unwrap_or(30));

        let payload = if let Some(delay_secs) = delay {
            rmp_serde::to_vec(&rmpv::Value::Map(vec![
                (rmpv::Value::from("delay"), rmpv::Value::from(delay_secs)),
            ]))
            .unwrap_or_default()
        } else {
            vec![]
        };

        let response = self
            .rpc_call(dest_hash, StyreneMessageType::Reboot, &payload, timeout)
            .await?;

        let mut result = RebootResult::default();
        result.accepted = response.message_type == StyreneMessageType::RebootResult;
        result.delay_secs = delay;
        Ok(result)
    }

    /// Query remote device's inbox (conversation list) via RPC.
    pub async fn remote_inbox(
        &self,
        dest_hash: &str,
        limit: u32,
        timeout: Option<u64>,
    ) -> Result<Vec<styrene_ipc::types::ConversationInfo>, std::io::Error> {
        let timeout_dur = Duration::from_secs(timeout.unwrap_or(30));
        let payload = rmp_serde::to_vec(&rmpv::Value::Map(vec![
            (rmpv::Value::from("limit"), rmpv::Value::from(limit as i64)),
        ]))
        .unwrap_or_default();

        let response = self
            .rpc_call(dest_hash, StyreneMessageType::InboxQuery, &payload, timeout_dur)
            .await?;

        if response.message_type != StyreneMessageType::InboxResponse {
            return Err(std::io::Error::other(format!(
                "unexpected response type: {:?}",
                response.message_type
            )));
        }

        // Parse conversation list from response
        let result: HashMap<String, rmpv::Value> =
            rmp_serde::from_slice(&response.payload)
                .map_err(|e| std::io::Error::other(format!("decode inbox: {e}")))?;

        let conversations = result
            .get("conversations")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| {
                        let map = v.as_map()?;
                        let mut info = styrene_ipc::types::ConversationInfo::default();
                        for (k, val) in map {
                            match k.as_str()? {
                                "peer_hash" => info.peer_hash = val.as_str()?.to_string(),
                                "unread_count" => info.unread_count = val.as_u64()? as u32,
                                "message_count" => info.message_count = val.as_u64()? as u32,
                                _ => {}
                            }
                        }
                        Some(info)
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(conversations)
    }

    /// Query remote device's messages for a specific peer via RPC.
    pub async fn remote_messages(
        &self,
        dest_hash: &str,
        peer_hash: &str,
        limit: u32,
        timeout: Option<u64>,
    ) -> Result<Vec<styrene_ipc::types::MessageInfo>, std::io::Error> {
        let timeout_dur = Duration::from_secs(timeout.unwrap_or(30));
        let payload = rmp_serde::to_vec(&rmpv::Value::Map(vec![
            (rmpv::Value::from("peer_hash"), rmpv::Value::from(peer_hash)),
            (rmpv::Value::from("limit"), rmpv::Value::from(limit as i64)),
        ]))
        .unwrap_or_default();

        let response = self
            .rpc_call(dest_hash, StyreneMessageType::MessagesQuery, &payload, timeout_dur)
            .await?;

        if response.message_type != StyreneMessageType::MessagesResponse {
            return Err(std::io::Error::other(format!(
                "unexpected response type: {:?}",
                response.message_type
            )));
        }

        let result: HashMap<String, rmpv::Value> =
            rmp_serde::from_slice(&response.payload)
                .map_err(|e| std::io::Error::other(format!("decode messages: {e}")))?;

        let messages = result
            .get("messages")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| {
                        let map = v.as_map()?;
                        let mut info = styrene_ipc::types::MessageInfo::default();
                        for (k, val) in map {
                            match k.as_str()? {
                                "id" => info.id = val.as_str()?.to_string(),
                                "source_hash" => info.source_hash = val.as_str()?.to_string(),
                                "content" => info.content = val.as_str()?.to_string(),
                                "timestamp" => info.timestamp = val.as_i64()?,
                                _ => {}
                            }
                        }
                        Some(info)
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(messages)
    }

    /// Handle an incoming RPC response (called by ProtocolService).
    /// Correlates with pending requests and resolves them.
    pub fn handle_response(&self, message: StyreneMessage) -> bool {
        let mut pending = self.pending.lock().unwrap();
        if let Some(req) = pending.remove(&message.request_id) {
            let _ = req.tx.send(message);
            true
        } else {
            false
        }
    }

    /// Low-level RPC call: send request, await correlated response.
    async fn rpc_call(
        &self,
        dest_hash: &str,
        msg_type: StyreneMessageType,
        payload: &[u8],
        timeout: Duration,
    ) -> Result<StyreneMessage, std::io::Error> {
        let transport = self.transport.lock().unwrap().clone().ok_or_else(|| {
            std::io::Error::other("transport not available for RPC")
        })?;
        let signer = self.signer.lock().unwrap().clone().ok_or_else(|| {
            std::io::Error::other("signer not available for RPC")
        })?;

        if !transport.is_connected() {
            return Err(std::io::Error::other("transport not connected"));
        }

        // Build Styrene wire message
        let wire_msg = StyreneMessage::new(msg_type, payload);
        let request_id = wire_msg.request_id;
        let wire_bytes = wire_msg.encode();

        // Build LXMF message wrapping the Styrene wire payload
        // The wire bytes go into fields["custom_data"], protocol="styrene"
        let dest_bytes: [u8; 16] = hex::decode(dest_hash)
            .map_err(|e| std::io::Error::other(format!("invalid dest hash: {e}")))?
            .try_into()
            .map_err(|_| std::io::Error::other("dest hash must be 16 bytes"))?;

        let source_hash = transport.identity_hash();
        let mut source_bytes = [0u8; 16];
        source_bytes.copy_from_slice(source_hash.as_slice());

        let fields = serde_json::json!({
            "protocol": "styrene",
            "custom_type": "styrene.io",
            "custom_data": hex::encode(&wire_bytes),
        });

        let lxmf_payload = crate::lxmf_bridge::build_wire_message(
            source_bytes,
            dest_bytes,
            "",    // no title for RPC
            "",    // no content for RPC
            Some(fields),
            &signer,
        )
        .map_err(|e| std::io::Error::other(format!("wire encode: {e}")))?;

        // Register pending request
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().unwrap();
            pending.insert(request_id, PendingRequest {
                tx,
                created_at: std::time::Instant::now(),
                dest_hash: dest_hash.to_string(),
            });
        }

        // Deliver via transport (path request → identity resolve → link send)
        let dest_addr = AddressHash::new(dest_bytes);
        let deliver_result = crate::services::messaging::MessagingService::deliver(
            transport.as_ref(),
            dest_addr,
            &lxmf_payload,
        )
        .await;

        if let Err(e) = deliver_result {
            // Remove pending request on delivery failure
            self.pending.lock().unwrap().remove(&request_id);
            return Err(std::io::Error::other(format!("delivery failed: {e}")));
        }

        // Await response with timeout
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => {
                // Sender dropped — shouldn't happen
                Err(std::io::Error::other("response channel closed"))
            }
            Err(_) => {
                self.pending.lock().unwrap().remove(&request_id);
                Err(std::io::Error::other(format!(
                    "RPC timeout after {}s to {dest_hash}",
                    timeout.as_secs()
                )))
            }
        }
    }

    /// Clean up expired pending requests (call periodically).
    pub fn cleanup_expired(&self, max_age: Duration) {
        let mut pending = self.pending.lock().unwrap();
        pending.retain(|_, req| req.created_at.elapsed() < max_age);
    }

    /// Number of pending RPC requests.
    pub fn pending_count(&self) -> usize {
        self.pending.lock().unwrap().len()
    }
}

impl Default for FleetService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_with_no_pending() {
        let svc = FleetService::new();
        assert_eq!(svc.pending_count(), 0);
    }

    #[test]
    fn handle_response_correlates_request() {
        let svc = FleetService::new();
        let (tx, _rx) = oneshot::channel();
        let request_id = [42u8; 16];
        svc.pending.lock().unwrap().insert(request_id, PendingRequest {
            tx,
            created_at: std::time::Instant::now(),
            dest_hash: "test".into(),
        });
        assert_eq!(svc.pending_count(), 1);

        let response = StyreneMessage::with_request_id(
            StyreneMessageType::StatusResponse,
            request_id,
            &[],
        );
        assert!(svc.handle_response(response));
        assert_eq!(svc.pending_count(), 0);
    }

    #[test]
    fn handle_response_unknown_id_returns_false() {
        let svc = FleetService::new();
        let response = StyreneMessage::new(StyreneMessageType::StatusResponse, &[]);
        assert!(!svc.handle_response(response));
    }

    #[test]
    fn cleanup_expired_removes_old_requests() {
        let svc = FleetService::new();
        let (tx, _rx) = oneshot::channel();
        svc.pending.lock().unwrap().insert([1u8; 16], PendingRequest {
            tx,
            created_at: std::time::Instant::now() - Duration::from_secs(120),
            dest_hash: "old".into(),
        });
        assert_eq!(svc.pending_count(), 1);
        svc.cleanup_expired(Duration::from_secs(60));
        assert_eq!(svc.pending_count(), 0);
    }

    #[tokio::test]
    async fn device_status_without_transport_returns_error() {
        let svc = FleetService::new();
        let result = svc.device_status("abcdef0123456789", None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("transport not available"));
    }
}
