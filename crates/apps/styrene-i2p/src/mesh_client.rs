//! Mesh transport client — sends I2pProxyRequest messages to the hub
//! over RNS and receives responses.
//!
//! Initializes a minimal RNS transport with a TCP client connection
//! to a known peer (the hub or a relay), registers an LXMF delivery
//! destination for receiving responses, and provides a request/response
//! API for the local HTTP proxy.

use anyhow::{Context, Result};
use rns_core::destination::DestinationName;
use rns_core::hash::AddressHash;
use rns_core::identity::PrivateIdentity;
use rns_core::transport::core_transport::{ReceivedData, Transport, TransportConfig};
use rns_core::transport::iface::tcp_client::TcpClient;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use styrene_mesh::i2p::{I2pProxyData, I2pProxyError, I2pProxyRequest, I2pProxyResponse};
use styrene_mesh::{StyreneMessage, StyreneMessageType};
use styrened::identity_store::load_or_create_identity;
use styrened::transport::adapter::TokioTransportAdapter;
use styrened::transport::mesh_transport::MeshTransport;
use tokio::sync::{broadcast, Mutex, Notify};

/// A pending proxy request awaiting response chunks.
struct PendingRequest {
    /// Notified when the response is complete.
    done: Arc<Notify>,
    /// Response header (set when I2pProxyResponse arrives).
    response: Mutex<Option<I2pProxyResponse>>,
    /// Accumulated body chunks (keyed by chunk_index).
    chunks: Mutex<HashMap<u32, Vec<u8>>>,
    /// Error (set when I2pProxyError arrives).
    error: Mutex<Option<I2pProxyError>>,
}

/// Mesh-connected I2P proxy client.
pub struct MeshClient {
    transport: Arc<dyn MeshTransport>,
    signer: Arc<PrivateIdentity>,
    hub_delivery_hash: String,
    /// Pending requests keyed by request_id.
    pending: Arc<Mutex<HashMap<[u8; 16], Arc<PendingRequest>>>>,
    /// Next sequence number.
    seq: Mutex<u32>,
}

impl MeshClient {
    /// Initialize a mesh client connected to the hub.
    ///
    /// - `hub_addr`: TCP address of the hub (e.g., "192.168.0.10:4242")
    /// - `hub_delivery_hash`: hex-encoded delivery destination hash of the hub
    /// - `identity_path`: path to identity key file (created if missing)
    pub async fn new(
        hub_addr: &str,
        hub_delivery_hash: &str,
        identity_path: Option<PathBuf>,
    ) -> Result<Self> {
        let identity_path = identity_path.unwrap_or_else(|| {
            dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("styrene")
                .join("i2p-proxy-identity.key")
        });

        // Ensure parent directory exists
        if let Some(parent) = identity_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create identity dir: {}", parent.display()))?;
        }

        let identity = load_or_create_identity(&identity_path)
            .with_context(|| format!("load identity: {}", identity_path.display()))?;

        eprintln!(
            "[mesh] identity loaded: {}",
            hex::encode(identity.address_hash().as_slice())
        );

        // Initialize transport
        let transport_identity =
            rns_core::transport::identity_bridge::to_transport_private_identity(&identity);
        let config = TransportConfig::new("i2p-proxy-client", &transport_identity, true);
        let mut transport_instance = Transport::new(config);

        // Connect to hub via TCP
        let iface_manager = transport_instance.iface_manager();
        let iface_id = iface_manager
            .lock()
            .await
            .spawn(TcpClient::new(hub_addr), TcpClient::spawn);
        eprintln!("[mesh] TCP client iface={iface_id} connecting to {hub_addr}");

        // Register LXMF delivery destination (for receiving responses)
        let destination = transport_instance
            .add_destination(
                transport_identity.clone(),
                DestinationName::new("lxmf", "delivery"),
            )
            .await;
        let (delivery_hash, delivery_addr) = {
            let dest = destination.lock().await;
            (
                hex::encode(dest.desc.address_hash.as_slice()),
                dest.desc.address_hash,
            )
        };
        eprintln!("[mesh] delivery destination: {delivery_hash}");

        let id_hash = identity.address_hash();

        let transport = Arc::new(transport_instance);

        // Create adapter
        let adapter = TokioTransportAdapter::new(
            transport.clone(),
            *id_hash,
            delivery_addr,
            destination.clone(),
            None,
        )
        .await;
        let mesh_transport: Arc<dyn MeshTransport> = Arc::new(adapter);

        let pending: Arc<Mutex<HashMap<[u8; 16], Arc<PendingRequest>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        // Spawn inbound response listener
        let inbound_rx = mesh_transport.subscribe_inbound();
        let pending_clone = pending.clone();
        tokio::spawn(async move {
            Self::response_listener(inbound_rx, pending_clone).await;
        });

        // Announce ourselves so the hub can route responses back
        mesh_transport.announce(None).await;

        Ok(Self {
            transport: mesh_transport,
            signer: Arc::new(identity),
            hub_delivery_hash: hub_delivery_hash.to_string(),
            pending,
            seq: Mutex::new(0),
        })
    }

    /// Send an HTTP request to the hub and wait for the response.
    pub async fn proxy_request(
        &self,
        method: &str,
        url: &str,
        headers: Vec<(String, String)>,
        body: Option<Vec<u8>>,
    ) -> Result<ProxyResponse> {
        let seq = {
            let mut s = self.seq.lock().await;
            let v = *s;
            *s = s.wrapping_add(1);
            v
        };

        let request = I2pProxyRequest {
            method: method.to_string(),
            url: url.to_string(),
            headers,
            body,
            seq,
        };

        let mut payload_buf = Vec::new();
        ciborium::into_writer(&request, &mut payload_buf)
            .map_err(|e| anyhow::anyhow!("CBOR encode: {e}"))?;

        let msg = StyreneMessage::new(StyreneMessageType::I2pProxyRequest, &payload_buf);
        let request_id = msg.request_id;

        // Register pending request
        let pending_req = Arc::new(PendingRequest {
            done: Arc::new(Notify::new()),
            response: Mutex::new(None),
            chunks: Mutex::new(HashMap::new()),
            error: Mutex::new(None),
        });
        self.pending.lock().await.insert(request_id, pending_req.clone());

        // Send via LXMF — hub_delivery_hash is the delivery destination hash directly
        let wire_bytes = msg.encode();
        let wire_hex = hex::encode(&wire_bytes);

        let dest_bytes_vec: Vec<u8> = hex::decode(&self.hub_delivery_hash)
            .map_err(|e| anyhow::anyhow!("invalid hub delivery hash: {e}"))?;
        let delivery_addr = AddressHash::new(
            dest_bytes_vec
                .try_into()
                .map_err(|_| anyhow::anyhow!("hub delivery hash must be 16 bytes"))?,
        );

        let source_hash = self.transport.identity_hash();
        let mut source_bytes = [0u8; 16];
        source_bytes.copy_from_slice(source_hash.as_slice());
        let mut dest_bytes = [0u8; 16];
        dest_bytes.copy_from_slice(delivery_addr.as_slice());

        let lxmf_payload = styrened::lxmf_bridge::build_wire_message(
            source_bytes,
            dest_bytes,
            "",
            "",
            Some(serde_json::json!({"protocol": "i2p_proxy", "custom_data": wire_hex})),
            &self.signer,
        )
        .map_err(|e| anyhow::anyhow!("wire encode: {e}"))?;

        styrened::services::MessagingService::deliver(
            self.transport.as_ref(),
            delivery_addr,
            &lxmf_payload,
        )
        .await
        .map_err(|e| anyhow::anyhow!("deliver: {e}"))?;

        eprintln!("[mesh] sent {method} {url} (seq={seq})");

        // Wait for response with timeout
        let timeout = Duration::from_secs(styrene_mesh::i2p::I2PD_RESPONSE_TIMEOUT_SECS + 30);
        match tokio::time::timeout(timeout, pending_req.done.notified()).await {
            Ok(()) => {}
            Err(_) => {
                self.pending.lock().await.remove(&request_id);
                anyhow::bail!("request timed out after {}s", timeout.as_secs());
            }
        }

        // Clean up
        self.pending.lock().await.remove(&request_id);

        // Check for error
        if let Some(err) = pending_req.error.lock().await.take() {
            anyhow::bail!("hub error {}: {}", err.code, err.message);
        }

        // Assemble response
        let resp = pending_req
            .response
            .lock()
            .await
            .take()
            .ok_or_else(|| anyhow::anyhow!("no response header received"))?;

        let chunks = pending_req.chunks.lock().await;
        let total = resp.total_chunks.unwrap_or(0) as usize;
        let mut body = Vec::new();
        for i in 0..total {
            if let Some(chunk) = chunks.get(&(i as u32)) {
                body.extend_from_slice(chunk);
            }
        }

        Ok(ProxyResponse {
            status: resp.status,
            headers: resp.headers,
            body,
        })
    }

    /// Background listener for inbound response messages.
    async fn response_listener(
        mut rx: broadcast::Receiver<ReceivedData>,
        pending: Arc<Mutex<HashMap<[u8; 16], Arc<PendingRequest>>>>,
    ) {
        loop {
            match rx.recv().await {
                Ok(data) => {
                    // The inbound data arrives as raw bytes from RNS.
                    // Try LXMF decode first, then extract styrene wire from custom_data.
                    // For now, try direct StyreneMessage decode on the raw data.
                    let bytes = data.data.as_slice();
                    if let Ok(msg) = StyreneMessage::decode(bytes) {
                        Self::dispatch_response(&pending, msg).await;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    eprintln!("[mesh] response listener lagged {n} messages");
                }
                Err(broadcast::error::RecvError::Closed) => {
                    eprintln!("[mesh] response listener channel closed");
                    break;
                }
            }
        }
    }

    /// Dispatch a response message to the pending request.
    async fn dispatch_response(
        pending: &Mutex<HashMap<[u8; 16], Arc<PendingRequest>>>,
        msg: StyreneMessage,
    ) {
        let guard = pending.lock().await;
        let req = match guard.get(&msg.request_id) {
            Some(r) => r.clone(),
            None => return, // No pending request for this ID
        };
        drop(guard);

        match msg.message_type {
            StyreneMessageType::I2pProxyResponse => {
                if let Ok(resp) = ciborium::from_reader::<I2pProxyResponse, _>(&msg.payload[..]) {
                    *req.response.lock().await = Some(resp);
                    // If no chunks expected, we're done
                    let total = req.response.lock().await.as_ref()
                        .and_then(|r| r.total_chunks)
                        .unwrap_or(0);
                    if total == 0 {
                        req.done.notify_one();
                    }
                }
            }
            StyreneMessageType::I2pProxyData => {
                if let Ok(data) = ciborium::from_reader::<I2pProxyData, _>(&msg.payload[..]) {
                    let is_final = data.final_chunk;
                    req.chunks.lock().await.insert(data.chunk_index, data.data);
                    if is_final {
                        req.done.notify_one();
                    }
                }
            }
            StyreneMessageType::I2pProxyError => {
                if let Ok(err) = ciborium::from_reader::<I2pProxyError, _>(&msg.payload[..]) {
                    *req.error.lock().await = Some(err);
                    req.done.notify_one();
                }
            }
            _ => {}
        }
    }
}

/// Assembled proxy response ready to return to the browser.
pub struct ProxyResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
