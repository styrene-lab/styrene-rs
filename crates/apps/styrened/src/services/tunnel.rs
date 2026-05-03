//! TunnelService — tunnel lifecycle management and protocol handling.
//!
//! Handles inbound TUNNEL_OFFER/ACCEPT/REJECT/TEARDOWN messages,
//! orchestrates WireGuard backend operations, and manages tunnel state.
//!
//! Registered as a ProtocolHandler for the "tunnel" protocol type.

use crate::services::events::EventService;
use crate::transport::mesh_transport::MeshTransport;
use async_trait::async_trait;
use rns_core::destination::DestinationName;
use rns_core::hash::AddressHash;
use rns_core::identity::PrivateIdentity;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use styrene_mesh::tunnel_payloads::{
    self, TunnelAccept, TunnelOffer, TunnelReject, TunnelTeardown,
};
use styrene_mesh::{StyreneMessage, StyreneMessageType};
use styrene_services::protocol_registry::{HandleResult, InboundMessage, ProtocolHandler};

#[cfg(feature = "wireguard")]
use styrene_tunnel::wireguard::WireGuardBackend;
#[cfg(feature = "wireguard")]
use styrene_tunnel::TunnelBackend;

/// Peer state stored when a tunnel is established.
#[derive(Debug, Clone)]
pub struct TunnelPeerState {
    /// Peer's WireGuard public key (base64).
    pub wg_pubkey: String,
    /// Peer's endpoint (IP:port), if known.
    pub endpoint: String,
    /// Peer's mesh overlay IPv6 address.
    pub mesh_ip: String,
    /// Pre-shared key (base64) for this tunnel.
    pub psk: String,
    /// MTU preference.
    pub mtu: u16,
    /// Timestamp when the tunnel was established.
    pub established_at: i64,
}

/// Tunnel lifecycle management and protocol handler.
pub struct TunnelService {
    transport: Arc<dyn MeshTransport>,
    signer: Arc<PrivateIdentity>,
    identity_hash: String,
    local_wg_pubkey: String,
    #[allow(dead_code)]
    local_wg_privkey: [u8; 32],
    local_endpoint: Mutex<Option<String>>,
    /// Whether transport is wired (prevents sends on NullTransport).
    wired: bool,
    /// Pending outbound offers: nonce → (peer_identity, offer details).
    pending_offers: Mutex<HashMap<String, PendingOffer>>,
    /// Seen nonces: nonce → timestamp (time-windowed replay protection).
    seen_nonces: Mutex<HashMap<String, i64>>,
    /// Active tunnels: peer_identity → peer state.
    active_tunnels: Mutex<HashMap<String, TunnelPeerState>>,
    /// Allowed peer identities (empty = allow all).
    allowed_peers: Mutex<Option<Vec<String>>>,
    /// Optional event service for emitting tunnel state changes.
    events: Mutex<Option<Arc<EventService>>>,
    /// Optional WireGuard backend for configuring actual tunnels.
    #[cfg(feature = "wireguard")]
    backend: Mutex<Option<Arc<WireGuardBackend>>>,
}

#[derive(Clone)]
struct PendingOffer {
    peer_identity: String,
    psk: String,
    mtu: u16,
}

/// Max nonces to track before eviction of old entries.
const MAX_NONCE_CACHE: usize = 10_000;
/// Nonce expiry window (seconds) — matches timestamp tolerance.
const NONCE_EXPIRY_SECS: i64 = 300;
/// Timestamp tolerance for offers (seconds).
const TIMESTAMP_TOLERANCE_SECS: i64 = 300;

impl Default for TunnelService {
    fn default() -> Self {
        Self::new()
    }
}

impl TunnelService {
    /// Create a placeholder service (not wired to transport).
    /// Tunnel operations will fail gracefully until `with_transport()` is used.
    pub fn new() -> Self {
        Self {
            transport: Arc::new(crate::transport::null_transport::NullTransport::new()),
            signer: Arc::new(PrivateIdentity::new_from_rand(rand_core::OsRng)),
            identity_hash: String::new(),
            local_wg_pubkey: String::new(),
            local_wg_privkey: [0u8; 32],
            local_endpoint: Mutex::new(None),
            wired: false,
            pending_offers: Mutex::new(HashMap::new()),
            seen_nonces: Mutex::new(HashMap::new()),
            active_tunnels: Mutex::new(HashMap::new()),
            allowed_peers: Mutex::new(None),
            events: Mutex::new(None),
            #[cfg(feature = "wireguard")]
            backend: Mutex::new(None),
        }
    }

    /// Create a fully wired tunnel service.
    pub fn with_transport(
        transport: Arc<dyn MeshTransport>,
        signer: Arc<PrivateIdentity>,
        identity_hash: String,
        wg_privkey: [u8; 32],
    ) -> Self {
        let pubkey = x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(wg_privkey));
        let pubkey_b64 = base64::engine::general_purpose::STANDARD.encode(pubkey.as_bytes());

        Self {
            transport,
            signer,
            identity_hash,
            local_wg_pubkey: pubkey_b64,
            local_wg_privkey: wg_privkey,
            local_endpoint: Mutex::new(None),
            wired: true,
            pending_offers: Mutex::new(HashMap::new()),
            seen_nonces: Mutex::new(HashMap::new()),
            active_tunnels: Mutex::new(HashMap::new()),
            allowed_peers: Mutex::new(None),
            events: Mutex::new(None),
            #[cfg(feature = "wireguard")]
            backend: Mutex::new(None),
        }
    }

    pub fn set_endpoint(&self, endpoint: String) {
        *self.local_endpoint.lock().expect("lock") = Some(endpoint);
    }

    /// Set the event service for emitting tunnel state changes.
    pub fn set_events(&self, events: Arc<EventService>) {
        *self.events.lock().expect("lock") = Some(events);
    }

    /// Emit a tunnel state change event if the event service is wired.
    fn emit_state(&self, peer_hash: &str, state: &str) {
        if let Some(events) = self.events.lock().expect("lock").as_ref() {
            events.emit_tunnel_state(peer_hash, state, "wireguard");
        }
    }

    /// Set allowed peers. None = allow all. Some(vec) = only these identities.
    pub fn set_allowed_peers(&self, peers: Option<Vec<String>>) {
        *self.allowed_peers.lock().expect("lock") = peers;
    }

    /// Set the WireGuard backend for configuring actual tunnels.
    /// If not set, tunnel negotiation state is still tracked but no
    /// WireGuard configuration is performed.
    #[cfg(feature = "wireguard")]
    pub fn set_backend(&self, backend: Arc<WireGuardBackend>) {
        *self.backend.lock().expect("lock") = Some(backend);
    }

    /// Configure WireGuard for a peer using the stored tunnel state.
    /// Logs warnings and continues on failure — the protocol state is still valid.
    #[cfg(feature = "wireguard")]
    async fn configure_wireguard(&self, peer_identity: &str, state: &TunnelPeerState) {
        use base64::Engine;
        use std::net::IpAddr;

        let backend = self.backend.lock().expect("lock").clone();
        let backend = match backend {
            Some(b) => b,
            None => return, // no backend wired — skip silently
        };

        // Parse endpoint into (IpAddr, port)
        let (remote_endpoint, remote_port) = if state.endpoint.is_empty() {
            (None, None)
        } else {
            match state.endpoint.rsplit_once(':') {
                Some((ip_str, port_str)) => {
                    let ip = ip_str.parse::<IpAddr>().ok();
                    let port = port_str.parse::<u16>().ok();
                    (ip, port)
                }
                None => (None, None),
            }
        };

        // Decode wg_pubkey from base64 to [u8; 32]
        let peer_x25519_public = match base64::engine::general_purpose::STANDARD
            .decode(&state.wg_pubkey)
        {
            Ok(bytes) if bytes.len() == 32 => {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes);
                Some(arr)
            }
            Ok(_) => {
                eprintln!("[tunnel] WireGuard: peer pubkey wrong length, skipping backend config");
                return;
            }
            Err(e) => {
                eprintln!("[tunnel] WireGuard: failed to decode peer pubkey: {e}");
                return;
            }
        };

        // Decode PSK from base64 to [u8; 32]
        let psk = match base64::engine::general_purpose::STANDARD.decode(&state.psk) {
            Ok(bytes) if bytes.len() == 32 => {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes);
                arr
            }
            Ok(_) => {
                eprintln!("[tunnel] WireGuard: PSK wrong length, skipping backend config");
                return;
            }
            Err(e) => {
                eprintln!("[tunnel] WireGuard: failed to decode PSK: {e}");
                return;
            }
        };

        let params = styrene_tunnel::traits::TunnelParams {
            peer_identity: peer_identity.to_string(),
            remote_endpoint,
            remote_port,
            psk,
            peer_x25519_public,
            peer_mesh_ip: Some(state.mesh_ip.clone()),
            mtu: Some(state.mtu),
        };

        match backend.establish(params).await {
            Ok(tunnel_id) => {
                eprintln!(
                    "[tunnel] WireGuard peer configured: {}",
                    &tunnel_id[..12.min(tunnel_id.len())]
                );
            }
            Err(e) => {
                eprintln!(
                    "[tunnel] WireGuard establish failed for {}: {e} (tunnel state still valid)",
                    &peer_identity[..12.min(peer_identity.len())]
                );
            }
        }
    }

    /// Tear down WireGuard configuration for a peer.
    #[cfg(feature = "wireguard")]
    async fn teardown_wireguard(&self, peer_identity: &str) {
        let backend = self.backend.lock().expect("lock").clone();
        let backend = match backend {
            Some(b) => b,
            None => return,
        };

        match backend.teardown(peer_identity).await {
            Ok(()) => {
                eprintln!(
                    "[tunnel] WireGuard peer removed: {}",
                    &peer_identity[..12.min(peer_identity.len())]
                );
            }
            Err(e) => {
                eprintln!(
                    "[tunnel] WireGuard teardown failed for {}: {e}",
                    &peer_identity[..12.min(peer_identity.len())]
                );
            }
        }
    }

    /// Check if a peer is authorized for tunnel establishment.
    fn is_peer_allowed(&self, peer: &str) -> bool {
        match self.allowed_peers.lock().expect("lock").as_ref() {
            None => true, // no allowlist = allow all
            Some(list) => list.iter().any(|p| p == peer),
        }
    }

    /// Get active tunnel state for a peer.
    pub fn get_peer_state(&self, peer: &str) -> Option<TunnelPeerState> {
        self.active_tunnels.lock().expect("lock").get(peer).cloned()
    }

    /// Get all active tunnel peer identities.
    pub fn active_peers(&self) -> Vec<String> {
        self.active_tunnels.lock().expect("lock").keys().cloned().collect()
    }

    /// Tear down a tunnel to a peer (operator-initiated outbound teardown).
    /// Removes peer from active tunnels, tears down WireGuard, and sends
    /// TUNNEL_TEARDOWN to the remote peer.
    pub async fn teardown_tunnel(&self, peer_identity: &str) -> Result<(), String> {
        // Remove from active tunnels
        let removed = self.active_tunnels.lock().expect("lock").remove(peer_identity);

        if removed.is_none() {
            return Err(format!(
                "no active tunnel for {}",
                &peer_identity[..12.min(peer_identity.len())]
            ));
        }

        // Tear down WireGuard backend if available
        #[cfg(feature = "wireguard")]
        self.teardown_wireguard(peer_identity).await;

        // Send TUNNEL_TEARDOWN to the remote peer
        if self.wired && self.transport.is_connected() {
            let teardown = TunnelTeardown {
                peer_identity: self.identity_hash.clone(),
                nonce: tunnel_payloads::generate_nonce(),
            };
            if let Ok(payload) = rmp_serde::to_vec(&teardown) {
                let _ = self
                    .send_tunnel_message(
                        peer_identity,
                        StyreneMessageType::TunnelTeardown,
                        &payload,
                    )
                    .await;
            }
        }

        self.emit_state(peer_identity, "torn_down");

        eprintln!(
            "[tunnel] operator-initiated teardown for {}",
            &peer_identity[..12.min(peer_identity.len())]
        );

        Ok(())
    }

    /// Initiate a tunnel to a peer. Sends TUNNEL_OFFER via LXMF.
    pub async fn initiate_tunnel(&self, peer_identity: &str) -> Result<String, String> {
        if !self.wired {
            return Err("tunnel service not wired to transport".into());
        }
        if !self.transport.is_connected() {
            return Err("transport not connected".into());
        }

        let mesh_ip = tunnel_payloads::derive_mesh_ip(&self.identity_hash);
        let endpoint = self.local_endpoint.lock().expect("lock").clone().unwrap_or_default();

        let mut psk = [0u8; 32];
        rand_core::OsRng.fill_bytes(&mut psk);
        let psk_b64 = base64::engine::general_purpose::STANDARD.encode(psk);

        let nonce = tunnel_payloads::generate_nonce();
        let offer = TunnelOffer {
            wg_pubkey: self.local_wg_pubkey.clone(),
            endpoint,
            mesh_ip,
            psk: psk_b64.clone(),
            mtu: 1420,
            nonce: nonce.clone(),
            timestamp: tunnel_payloads::now_ts(),
        };

        self.pending_offers.lock().expect("lock").insert(
            nonce.clone(),
            PendingOffer { peer_identity: peer_identity.to_string(), psk: psk_b64, mtu: 1420 },
        );

        let payload = rmp_serde::to_vec(&offer).map_err(|e| format!("encode: {e}"))?;
        self.send_tunnel_message(peer_identity, StyreneMessageType::TunnelOffer, &payload).await?;

        eprintln!(
            "[tunnel] sent TUNNEL_OFFER to {} nonce={}",
            &peer_identity[..12.min(peer_identity.len())],
            &nonce[..8]
        );
        Ok(nonce)
    }

    async fn handle_offer(&self, source: &str, offer: TunnelOffer) -> HandleResult {
        if !self.check_nonce(&offer.nonce) {
            eprintln!("[tunnel] rejected TUNNEL_OFFER: duplicate nonce");
            return HandleResult::Handled;
        }

        let now = tunnel_payloads::now_ts();
        if (now - offer.timestamp).unsigned_abs() > TIMESTAMP_TOLERANCE_SECS as u64 {
            eprintln!("[tunnel] rejected TUNNEL_OFFER: stale timestamp");
            return HandleResult::Handled;
        }

        if !self.is_peer_allowed(source) {
            eprintln!(
                "[tunnel] rejected TUNNEL_OFFER from {}: not in allowlist",
                &source[..12.min(source.len())]
            );
            // Send reject
            if let Ok(payload) = rmp_serde::to_vec(&TunnelReject {
                reason: "peer not authorized".into(),
                offer_nonce: offer.nonce.clone(),
            }) {
                let _ = self
                    .send_tunnel_message(source, StyreneMessageType::TunnelReject, &payload)
                    .await;
            }
            return HandleResult::Handled;
        }

        eprintln!(
            "[tunnel] received TUNNEL_OFFER from {} endpoint={}",
            &source[..12.min(source.len())],
            offer.endpoint
        );

        let mesh_ip = tunnel_payloads::derive_mesh_ip(&self.identity_hash);
        let endpoint = self.local_endpoint.lock().expect("lock").clone().unwrap_or_default();

        let accept = TunnelAccept {
            wg_pubkey: self.local_wg_pubkey.clone(),
            endpoint,
            mesh_ip,
            offer_nonce: offer.nonce.clone(),
            nonce: tunnel_payloads::generate_nonce(),
            timestamp: tunnel_payloads::now_ts(),
        };

        if let Ok(payload) = rmp_serde::to_vec(&accept) {
            let _ =
                self.send_tunnel_message(source, StyreneMessageType::TunnelAccept, &payload).await;
        }

        // Store full peer state
        let peer_state = TunnelPeerState {
            wg_pubkey: offer.wg_pubkey,
            endpoint: offer.endpoint,
            mesh_ip: offer.mesh_ip,
            psk: offer.psk,
            mtu: offer.mtu,
            established_at: tunnel_payloads::now_ts(),
        };
        self.active_tunnels.lock().expect("lock").insert(source.to_string(), peer_state.clone());

        // Configure WireGuard backend if available
        #[cfg(feature = "wireguard")]
        self.configure_wireguard(source, &peer_state).await;

        self.emit_state(source, "established");

        eprintln!("[tunnel] sent TUNNEL_ACCEPT to {}", &source[..12.min(source.len())]);

        HandleResult::Handled
    }

    async fn handle_accept(&self, source: &str, accept: TunnelAccept) -> HandleResult {
        let pending = self.pending_offers.lock().expect("lock").remove(&accept.offer_nonce);

        let pending = match pending {
            Some(p) => p,
            None => {
                eprintln!("[tunnel] rejected TUNNEL_ACCEPT: unknown offer_nonce");
                return HandleResult::Handled;
            }
        };

        // Verify the accept came from the peer we sent the offer to.
        if pending.peer_identity != source {
            eprintln!(
                "[tunnel] rejected TUNNEL_ACCEPT: source mismatch (expected {}, got {})",
                &pending.peer_identity[..12.min(pending.peer_identity.len())],
                &source[..12.min(source.len())]
            );
            return HandleResult::Handled;
        }

        eprintln!(
            "[tunnel] received TUNNEL_ACCEPT from {} endpoint={}",
            &source[..12.min(source.len())],
            accept.endpoint
        );

        // Store full peer state
        let peer_state = TunnelPeerState {
            wg_pubkey: accept.wg_pubkey,
            endpoint: accept.endpoint,
            mesh_ip: accept.mesh_ip,
            psk: pending.psk,
            mtu: pending.mtu,
            established_at: tunnel_payloads::now_ts(),
        };
        self.active_tunnels.lock().expect("lock").insert(source.to_string(), peer_state.clone());

        // Configure WireGuard backend if available
        #[cfg(feature = "wireguard")]
        self.configure_wireguard(source, &peer_state).await;

        self.emit_state(source, "established");

        HandleResult::Handled
    }

    async fn handle_teardown(&self, source: &str, teardown: TunnelTeardown) -> HandleResult {
        if !self.check_nonce(&teardown.nonce) {
            return HandleResult::Handled;
        }

        // Tear down WireGuard backend if available (before removing state)
        #[cfg(feature = "wireguard")]
        self.teardown_wireguard(source).await;

        self.active_tunnels.lock().expect("lock").remove(source);

        self.emit_state(source, "torn_down");

        eprintln!("[tunnel] received TUNNEL_TEARDOWN from {}", &source[..12.min(source.len())]);

        HandleResult::Handled
    }

    /// Check and record a nonce. Returns true if new, false if duplicate.
    /// Uses time-windowed eviction instead of full cache clear.
    fn check_nonce(&self, nonce: &str) -> bool {
        let now = tunnel_payloads::now_ts();
        let mut seen = self.seen_nonces.lock().expect("lock");

        // Evict expired entries when cache is full
        if seen.len() >= MAX_NONCE_CACHE {
            seen.retain(|_, ts| now - *ts < NONCE_EXPIRY_SECS);
        }

        // Check if nonce was already seen
        if seen.contains_key(nonce) {
            return false;
        }

        seen.insert(nonce.to_string(), now);
        true
    }

    async fn send_tunnel_message(
        &self,
        peer_identity: &str,
        msg_type: StyreneMessageType,
        payload: &[u8],
    ) -> Result<(), String> {
        let msg = StyreneMessage::new(msg_type, payload);
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

        let source_hash = self.transport.identity_hash();
        let mut source_bytes = [0u8; 16];
        source_bytes.copy_from_slice(source_hash.as_slice());
        let mut dest_bytes = [0u8; 16];
        dest_bytes.copy_from_slice(delivery_addr.as_slice());

        let lxmf_payload = crate::lxmf_bridge::build_wire_message(
            source_bytes,
            dest_bytes,
            "",
            "",
            Some(serde_json::json!({"protocol": "tunnel", "custom_data": wire_hex})),
            &self.signer,
        )
        .map_err(|e| format!("wire encode: {e}"))?;

        crate::services::MessagingService::deliver(
            self.transport.as_ref(),
            delivery_addr,
            &lxmf_payload,
        )
        .await
        .map_err(|e| format!("deliver: {e}"))?;

        Ok(())
    }
}

use base64::Engine;
use rand_core::RngCore;

#[async_trait]
impl ProtocolHandler for TunnelService {
    fn name(&self) -> &str {
        "tunnel-handler"
    }

    fn protocol_types(&self) -> Vec<String> {
        vec!["tunnel".to_string()]
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
            StyreneMessageType::TunnelOffer => {
                match rmp_serde::from_slice::<TunnelOffer>(&message.payload) {
                    Ok(offer) => self.handle_offer(source, offer).await,
                    Err(e) => {
                        eprintln!("[tunnel] decode TUNNEL_OFFER failed: {e}");
                        HandleResult::Handled
                    }
                }
            }
            StyreneMessageType::TunnelAccept => {
                match rmp_serde::from_slice::<TunnelAccept>(&message.payload) {
                    Ok(accept) => self.handle_accept(source, accept).await,
                    Err(e) => {
                        eprintln!("[tunnel] decode TUNNEL_ACCEPT failed: {e}");
                        HandleResult::Handled
                    }
                }
            }
            StyreneMessageType::TunnelReject => {
                match rmp_serde::from_slice::<TunnelReject>(&message.payload) {
                    Ok(reject) => {
                        eprintln!(
                            "[tunnel] TUNNEL_REJECT from {}: {}",
                            &source[..12.min(source.len())],
                            reject.reason
                        );
                        self.pending_offers.lock().expect("lock").remove(&reject.offer_nonce);
                        self.emit_state(source, "rejected");
                        HandleResult::Handled
                    }
                    Err(e) => {
                        eprintln!("[tunnel] decode TUNNEL_REJECT failed: {e}");
                        HandleResult::Handled
                    }
                }
            }
            StyreneMessageType::TunnelTeardown => {
                match rmp_serde::from_slice::<TunnelTeardown>(&message.payload) {
                    Ok(teardown) => self.handle_teardown(source, teardown).await,
                    Err(e) => {
                        eprintln!("[tunnel] decode TUNNEL_TEARDOWN failed: {e}");
                        HandleResult::Handled
                    }
                }
            }
            _ => HandleResult::NotHandled,
        }
    }
}
