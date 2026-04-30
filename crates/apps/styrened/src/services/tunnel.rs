//! TunnelService — tunnel lifecycle management and protocol handling.
//!
//! Handles inbound TUNNEL_OFFER/ACCEPT/REJECT/TEARDOWN messages,
//! orchestrates WireGuard backend operations, and manages tunnel state.
//!
//! Registered as a ProtocolHandler for the "tunnel" protocol type.

use crate::transport::mesh_transport::MeshTransport;
use async_trait::async_trait;
use rns_core::destination::DestinationName;
use rns_core::hash::AddressHash;
use rns_core::identity::PrivateIdentity;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use styrene_mesh::tunnel_payloads::{self, TunnelAccept, TunnelOffer, TunnelReject, TunnelTeardown};
use styrene_mesh::{StyreneMessage, StyreneMessageType};
use styrene_services::protocol_registry::{HandleResult, InboundMessage, ProtocolHandler};

/// Tunnel lifecycle management and protocol handler.
pub struct TunnelService {
    transport: Arc<dyn MeshTransport>,
    signer: Arc<PrivateIdentity>,
    identity_hash: String,
    local_wg_pubkey: String,
    #[allow(dead_code)]
    local_wg_privkey: [u8; 32],
    local_endpoint: Mutex<Option<String>>,
    /// Pending outbound offers: nonce → peer_identity.
    pending_offers: Mutex<HashMap<String, String>>,
    /// Seen nonces for replay protection.
    seen_nonces: Mutex<HashSet<String>>,
    /// Active tunnel peer identities.
    active_tunnels: Mutex<HashSet<String>>,
}

impl TunnelService {
    /// Create a placeholder service (no transport wired yet).
    /// Call `wire_transport()` during bootstrap to enable tunnel operations.
    pub fn new() -> Self {
        Self {
            transport: Arc::new(crate::transport::null_transport::NullTransport::new()),
            signer: Arc::new(PrivateIdentity::new_from_rand(rand_core::OsRng)),
            identity_hash: String::new(),
            local_wg_pubkey: String::new(),
            local_wg_privkey: [0u8; 32],
            local_endpoint: Mutex::new(None),
            pending_offers: Mutex::new(HashMap::new()),
            seen_nonces: Mutex::new(HashSet::new()),
            active_tunnels: Mutex::new(HashSet::new()),
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
            pending_offers: Mutex::new(HashMap::new()),
            seen_nonces: Mutex::new(HashSet::new()),
            active_tunnels: Mutex::new(HashSet::new()),
        }
    }

    pub fn set_endpoint(&self, endpoint: String) {
        *self.local_endpoint.lock().expect("lock") = Some(endpoint);
    }

    /// Initiate a tunnel to a peer. Sends TUNNEL_OFFER via LXMF.
    pub async fn initiate_tunnel(&self, peer_identity: &str) -> Result<String, String> {
        let mesh_ip = tunnel_payloads::derive_mesh_ip(&self.identity_hash);
        let endpoint = self
            .local_endpoint
            .lock()
            .expect("lock")
            .clone()
            .unwrap_or_default();

        let mut psk = [0u8; 32];
        rand_core::OsRng.fill_bytes(&mut psk);
        let psk_b64 = base64::engine::general_purpose::STANDARD.encode(psk);

        let nonce = tunnel_payloads::generate_nonce();
        let offer = TunnelOffer {
            wg_pubkey: self.local_wg_pubkey.clone(),
            endpoint,
            mesh_ip,
            psk: psk_b64,
            mtu: 1420,
            nonce: nonce.clone(),
            timestamp: tunnel_payloads::now_ts(),
        };

        self.pending_offers
            .lock()
            .expect("lock")
            .insert(nonce.clone(), peer_identity.to_string());

        let payload = rmp_serde::to_vec(&offer).map_err(|e| format!("encode: {e}"))?;
        self.send_tunnel_message(peer_identity, StyreneMessageType::TunnelOffer, &payload)
            .await?;

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
        if (now - offer.timestamp).unsigned_abs() > 300 {
            eprintln!("[tunnel] rejected TUNNEL_OFFER: stale timestamp");
            return HandleResult::Handled;
        }

        eprintln!(
            "[tunnel] received TUNNEL_OFFER from {} endpoint={}",
            &source[..12.min(source.len())],
            offer.endpoint
        );

        let mesh_ip = tunnel_payloads::derive_mesh_ip(&self.identity_hash);
        let endpoint = self
            .local_endpoint
            .lock()
            .expect("lock")
            .clone()
            .unwrap_or_default();

        let accept = TunnelAccept {
            wg_pubkey: self.local_wg_pubkey.clone(),
            endpoint,
            mesh_ip,
            offer_nonce: offer.nonce.clone(),
            nonce: tunnel_payloads::generate_nonce(),
            timestamp: tunnel_payloads::now_ts(),
        };

        if let Ok(payload) = rmp_serde::to_vec(&accept) {
            let _ = self
                .send_tunnel_message(source, StyreneMessageType::TunnelAccept, &payload)
                .await;
        }

        self.active_tunnels
            .lock()
            .expect("lock")
            .insert(source.to_string());

        eprintln!(
            "[tunnel] sent TUNNEL_ACCEPT to {}",
            &source[..12.min(source.len())]
        );

        HandleResult::Handled
    }

    async fn handle_accept(&self, source: &str, accept: TunnelAccept) -> HandleResult {
        let peer = self
            .pending_offers
            .lock()
            .expect("lock")
            .remove(&accept.offer_nonce);

        if peer.is_none() {
            eprintln!("[tunnel] rejected TUNNEL_ACCEPT: unknown offer_nonce");
            return HandleResult::Handled;
        }

        eprintln!(
            "[tunnel] received TUNNEL_ACCEPT from {} endpoint={}",
            &source[..12.min(source.len())],
            accept.endpoint
        );

        self.active_tunnels
            .lock()
            .expect("lock")
            .insert(source.to_string());

        HandleResult::Handled
    }

    async fn handle_teardown(&self, source: &str, teardown: TunnelTeardown) -> HandleResult {
        if !self.check_nonce(&teardown.nonce) {
            return HandleResult::Handled;
        }

        self.active_tunnels.lock().expect("lock").remove(source);

        eprintln!(
            "[tunnel] received TUNNEL_TEARDOWN from {}",
            &source[..12.min(source.len())]
        );

        HandleResult::Handled
    }

    fn check_nonce(&self, nonce: &str) -> bool {
        let mut seen = self.seen_nonces.lock().expect("lock");
        if seen.len() > 10_000 {
            seen.clear();
        }
        seen.insert(nonce.to_string())
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

    pub fn active_peers(&self) -> Vec<String> {
        self.active_tunnels
            .lock()
            .expect("lock")
            .iter()
            .cloned()
            .collect()
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
                        self.pending_offers
                            .lock()
                            .expect("lock")
                            .remove(&reject.offer_nonce);
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
