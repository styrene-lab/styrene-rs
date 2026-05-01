//! I2pProxyService — proxies HTTP requests to `.i2p` eepsites through
//! the hub's i2pd router.
//!
//! Mesh clients send I2pProxyRequest messages over LXMF. The hub forwards
//! the HTTP request to i2pd's HTTP proxy, chunks the response, and sends
//! it back as I2pProxyResponse + I2pProxyData messages.
//!
//! Registered as a ProtocolHandler for the "i2p_proxy" protocol type.

use crate::transport::mesh_transport::MeshTransport;
use async_trait::async_trait;
use rns_core::destination::DestinationName;
use rns_core::hash::AddressHash;
use rns_core::identity::PrivateIdentity;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use styrene_mesh::i2p::{
    I2pProxyData, I2pProxyError, I2pProxyRequest, I2pProxyResponse,
    I2PD_RESPONSE_TIMEOUT_SECS, MAX_CONCURRENT_REQUESTS, MAX_REQUEST_BODY, RATE_LIMIT_PER_MINUTE,
};
use styrene_mesh::{StyreneMessage, StyreneMessageType};
use styrene_services::protocol_registry::{HandleResult, InboundMessage, ProtocolHandler};

/// Chunk size for response body streaming (bytes).
/// Kept well under LXMF propagated limit to avoid fragmentation.
const RESPONSE_CHUNK_SIZE: usize = 8192;

/// Maximum response body size (1MB — larger responses get truncated with error).
const MAX_RESPONSE_BODY: usize = 1024 * 1024;

/// I2P HTTP proxy service for the hub.
pub struct I2pProxyService {
    transport: Mutex<Arc<dyn MeshTransport>>,
    signer: Mutex<Arc<PrivateIdentity>>,
    identity_hash: Mutex<String>,
    /// i2pd HTTP proxy address.
    i2pd_proxy_addr: String,
    /// Whether transport is wired.
    wired: Mutex<bool>,
    /// Rate limiter: identity_hash → list of request timestamps.
    rate_limits: Mutex<HashMap<String, Vec<Instant>>>,
    /// Active request count per identity (for concurrency limiting).
    active_requests: Mutex<HashMap<String, usize>>,
}

impl Default for I2pProxyService {
    fn default() -> Self {
        Self::new()
    }
}

impl I2pProxyService {
    /// Create a placeholder service (not wired to transport).
    pub fn new() -> Self {
        Self {
            transport: Mutex::new(Arc::new(crate::transport::null_transport::NullTransport::new())),
            signer: Mutex::new(Arc::new(PrivateIdentity::new_from_rand(rand_core::OsRng))),
            identity_hash: Mutex::new(String::new()),
            i2pd_proxy_addr: "http://127.0.0.1:4444".to_string(),
            wired: Mutex::new(false),
            rate_limits: Mutex::new(HashMap::new()),
            active_requests: Mutex::new(HashMap::new()),
        }
    }

    /// Wire a signing identity and transport for outbound delivery.
    /// Called after construction when the identity is available.
    pub fn set_signer(
        &self,
        transport: Arc<dyn MeshTransport>,
        signer: Arc<PrivateIdentity>,
        identity_hash: String,
    ) {
        *self.transport.lock().unwrap() = transport;
        *self.signer.lock().unwrap() = signer;
        *self.identity_hash.lock().unwrap() = identity_hash;
        *self.wired.lock().unwrap() = true;
    }

    /// Check rate limit for an identity. Returns true if allowed.
    fn check_rate_limit(&self, identity: &str) -> bool {
        let mut limits = self.rate_limits.lock().unwrap();
        let now = Instant::now();
        let window = Duration::from_secs(60);

        let timestamps = limits.entry(identity.to_string()).or_default();
        timestamps.retain(|t| now.duration_since(*t) < window);

        if timestamps.len() >= RATE_LIMIT_PER_MINUTE as usize {
            return false;
        }

        timestamps.push(now);
        true
    }

    /// Check concurrent request limit for an identity.
    fn check_concurrency(&self, identity: &str) -> bool {
        let active = self.active_requests.lock().unwrap();
        active.get(identity).copied().unwrap_or(0) < MAX_CONCURRENT_REQUESTS
    }

    /// Increment active request count.
    fn inc_active(&self, identity: &str) {
        let mut active = self.active_requests.lock().unwrap();
        *active.entry(identity.to_string()).or_insert(0) += 1;
    }

    /// Decrement active request count.
    fn dec_active(&self, identity: &str) {
        let mut active = self.active_requests.lock().unwrap();
        if let Some(count) = active.get_mut(identity) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                active.remove(identity);
            }
        }
    }

    /// Forward an HTTP request to i2pd and return chunked response messages.
    async fn proxy_request(
        &self,
        source: &str,
        request: I2pProxyRequest,
        request_id: [u8; 16],
    ) -> Vec<StyreneMessage> {
        // Validate request body size
        if let Some(ref body) = request.body {
            if body.len() > MAX_REQUEST_BODY {
                return vec![self.make_error_msg(
                    request.seq,
                    request_id,
                    413,
                    "Request body exceeds 64KB limit",
                )];
            }
        }

        // Validate URL is .i2p
        if !request.url.contains(".i2p") {
            return vec![self.make_error_msg(
                request.seq,
                request_id,
                400,
                "Only .i2p URLs are accepted",
            )];
        }

        self.inc_active(source);
        let result = self.do_proxy(&request, request_id).await;
        self.dec_active(source);
        result
    }

    /// Perform the actual HTTP proxy request to i2pd.
    async fn do_proxy(
        &self,
        request: &I2pProxyRequest,
        request_id: [u8; 16],
    ) -> Vec<StyreneMessage> {
        // Build HTTP client with i2pd as proxy
        let proxy = match reqwest::Proxy::http(&self.i2pd_proxy_addr) {
            Ok(p) => p,
            Err(e) => {
                return vec![self.make_error_msg(
                    request.seq,
                    request_id,
                    502,
                    &format!("Failed to configure i2pd proxy: {e}"),
                )];
            }
        };

        let client = match reqwest::Client::builder()
            .proxy(proxy)
            .timeout(Duration::from_secs(I2PD_RESPONSE_TIMEOUT_SECS))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                return vec![self.make_error_msg(
                    request.seq,
                    request_id,
                    502,
                    &format!("Failed to build HTTP client: {e}"),
                )];
            }
        };

        // Build the request
        let method = match request.method.to_uppercase().as_str() {
            "GET" => reqwest::Method::GET,
            "POST" => reqwest::Method::POST,
            "HEAD" => reqwest::Method::HEAD,
            "PUT" => reqwest::Method::PUT,
            "DELETE" => reqwest::Method::DELETE,
            _ => {
                return vec![self.make_error_msg(
                    request.seq,
                    request_id,
                    400,
                    &format!("Unsupported HTTP method: {}", request.method),
                )];
            }
        };

        let mut req_builder = client.request(method, &request.url);

        for (key, value) in &request.headers {
            req_builder = req_builder.header(key.as_str(), value.as_str());
        }

        if let Some(ref body) = request.body {
            req_builder = req_builder.body(body.clone());
        }

        // Send request to i2pd
        let response = match req_builder.send().await {
            Ok(r) => r,
            Err(e) => {
                let code = if e.is_timeout() { 504 } else { 502 };
                return vec![self.make_error_msg(
                    request.seq,
                    request_id,
                    code,
                    &format!("i2pd proxy request failed: {e}"),
                )];
            }
        };

        // Collect response
        let status = response.status().as_u16();
        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();

        let body = match response.bytes().await {
            Ok(b) => b.to_vec(),
            Err(e) => {
                return vec![self.make_error_msg(
                    request.seq,
                    request_id,
                    502,
                    &format!("Failed to read response body: {e}"),
                )];
            }
        };

        if body.len() > MAX_RESPONSE_BODY {
            return vec![self.make_error_msg(
                request.seq,
                request_id,
                413,
                &format!(
                    "Response too large: {} bytes (max {})",
                    body.len(),
                    MAX_RESPONSE_BODY
                ),
            )];
        }

        // Build response messages
        let mut messages = Vec::new();

        let chunks: Vec<&[u8]> = body.chunks(RESPONSE_CHUNK_SIZE).collect();
        let total_chunks = if body.is_empty() { 0 } else { chunks.len() as u32 };

        // Response header
        let resp = I2pProxyResponse {
            status,
            headers,
            seq: request.seq,
            total_size: Some(body.len() as u64),
            total_chunks: Some(total_chunks),
        };
        let mut resp_buf = Vec::new();
        ciborium::into_writer(&resp, &mut resp_buf).unwrap();
        messages.push(StyreneMessage::with_request_id(
            StyreneMessageType::I2pProxyResponse,
            request_id,
            &resp_buf,
        ));

        // Response body chunks
        for (i, chunk) in chunks.iter().enumerate() {
            let data = I2pProxyData {
                seq: request.seq,
                chunk_index: i as u32,
                data: chunk.to_vec(),
                final_chunk: i == chunks.len() - 1,
            };
            let mut data_buf = Vec::new();
            ciborium::into_writer(&data, &mut data_buf).unwrap();
            messages.push(StyreneMessage::with_request_id(
                StyreneMessageType::I2pProxyData,
                request_id,
                &data_buf,
            ));
        }

        messages
    }

    /// Build an error response message.
    fn make_error_msg(
        &self,
        seq: u32,
        request_id: [u8; 16],
        code: u16,
        message: &str,
    ) -> StyreneMessage {
        let err = I2pProxyError {
            seq,
            code,
            message: message.to_string(),
        };
        let mut buf = Vec::new();
        ciborium::into_writer(&err, &mut buf).unwrap();
        StyreneMessage::with_request_id(StyreneMessageType::I2pProxyError, request_id, &buf)
    }

    /// Send a StyreneMessage to a peer via LXMF (same pattern as TunnelService).
    /// Send a StyreneMessage to a peer via LXMF (same pattern as TunnelService).
    async fn send_i2p_message(
        &self,
        peer_identity: &str,
        msg: &StyreneMessage,
    ) -> Result<(), String> {
        let wire_bytes = msg.encode();
        let wire_hex = hex::encode(&wire_bytes);

        let identity_bytes: [u8; 16] = hex::decode(peer_identity)
            .map_err(|e| format!("invalid peer hash: {e}"))?
            .try_into()
            .map_err(|_| "peer hash must be 16 bytes".to_string())?;

        let delivery_addr = {
            let name = DestinationName::new("lxmf", "delivery");
            let mut combined = Vec::with_capacity(48);
            combined.extend_from_slice(name.as_name_hash_slice());
            combined.extend_from_slice(&identity_bytes);
            let truncated = rns_core::hash::address_hash(&combined);
            AddressHash::new(truncated)
        };

        let transport = self.transport.lock().unwrap().clone();
        let signer = self.signer.lock().unwrap().clone();

        let source_hash = transport.identity_hash();
        let mut source_bytes = [0u8; 16];
        source_bytes.copy_from_slice(source_hash.as_slice());
        let mut dest_bytes = [0u8; 16];
        dest_bytes.copy_from_slice(delivery_addr.as_slice());

        let lxmf_payload = crate::lxmf_bridge::build_wire_message(
            source_bytes,
            dest_bytes,
            "",
            "",
            Some(serde_json::json!({"protocol": "i2p_proxy", "custom_data": wire_hex})),
            &signer,
        )
        .map_err(|e| format!("wire encode: {e}"))?;

        crate::services::MessagingService::deliver(
            transport.as_ref(),
            delivery_addr,
            &lxmf_payload,
        )
        .await
        .map_err(|e| format!("deliver: {e}"))?;

        Ok(())
    }
}

#[async_trait]
impl ProtocolHandler for I2pProxyService {
    fn name(&self) -> &str {
        "i2p-proxy-handler"
    }

    fn protocol_types(&self) -> Vec<String> {
        vec!["i2p_proxy".to_string()]
    }

    async fn handle(&self, msg: &InboundMessage) -> HandleResult {
        let hex_data = match msg.fields.get("custom_data").and_then(|v| v.as_str()) {
            Some(data) => data,
            None => return HandleResult::NotHandled,
        };

        let wire_bytes = match hex::decode(hex_data) {
            Ok(bytes) => bytes,
            Err(_) => return HandleResult::NotHandled,
        };

        let message = match StyreneMessage::decode(&wire_bytes) {
            Ok(msg) => msg,
            Err(_) => return HandleResult::NotHandled,
        };

        let source = &msg.source_hash;

        match message.message_type {
            StyreneMessageType::I2pProxyRequest => {
                if !*self.wired.lock().unwrap() {
                    eprintln!("[i2p-proxy] not wired to transport, ignoring request");
                    return HandleResult::Handled;
                }

                // Rate limit check
                if !self.check_rate_limit(source) {
                    eprintln!("[i2p-proxy] rate limit exceeded for {}", &source[..12.min(source.len())]);
                    let err_msg = self.make_error_msg(0, message.request_id, 429, "Rate limit exceeded");
                    if let Err(e) = self.send_i2p_message(source, &err_msg).await {
                        eprintln!("[i2p-proxy] failed to send rate limit error: {e}");
                    }
                    return HandleResult::Handled;
                }

                // Concurrency check
                if !self.check_concurrency(source) {
                    let err_msg = self.make_error_msg(0, message.request_id, 429, "Too many concurrent requests");
                    if let Err(e) = self.send_i2p_message(source, &err_msg).await {
                        eprintln!("[i2p-proxy] failed to send concurrency error: {e}");
                    }
                    return HandleResult::Handled;
                }

                // Decode the proxy request
                let request: I2pProxyRequest = match ciborium::from_reader(&message.payload[..]) {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("[i2p-proxy] decode I2pProxyRequest failed: {e}");
                        return HandleResult::Handled;
                    }
                };

                eprintln!(
                    "[i2p-proxy] {} {} from {}",
                    request.method,
                    request.url,
                    &source[..12.min(source.len())]
                );

                // Proxy the request and send response messages
                let responses = self.proxy_request(source, request, message.request_id).await;

                for resp in responses {
                    if let Err(e) = self.send_i2p_message(source, &resp).await {
                        eprintln!("[i2p-proxy] failed to send response: {e}");
                        break;
                    }
                }

                HandleResult::Handled
            }

            StyreneMessageType::I2pProxyClose => {
                // Client is aborting a request — nothing to clean up on the hub
                // since we process requests synchronously per message.
                eprintln!(
                    "[i2p-proxy] close from {}",
                    &source[..12.min(source.len())]
                );
                HandleResult::Handled
            }

            _ => HandleResult::NotHandled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limit_allows_within_limit() {
        let svc = I2pProxyService::new();
        for _ in 0..RATE_LIMIT_PER_MINUTE {
            assert!(svc.check_rate_limit("test-identity"));
        }
        // Next one should be rejected
        assert!(!svc.check_rate_limit("test-identity"));
    }

    #[test]
    fn concurrency_limit() {
        let svc = I2pProxyService::new();
        for _ in 0..MAX_CONCURRENT_REQUESTS {
            assert!(svc.check_concurrency("test"));
            svc.inc_active("test");
        }
        assert!(!svc.check_concurrency("test"));
        svc.dec_active("test");
        assert!(svc.check_concurrency("test"));
    }

    #[test]
    fn different_identities_independent_limits() {
        let svc = I2pProxyService::new();
        for _ in 0..RATE_LIMIT_PER_MINUTE {
            assert!(svc.check_rate_limit("alice"));
        }
        assert!(!svc.check_rate_limit("alice"));
        // Bob should still be allowed
        assert!(svc.check_rate_limit("bob"));
    }
}
