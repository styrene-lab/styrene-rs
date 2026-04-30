//! Tunnel negotiation payloads — msgpack-serializable structures for
//! LXMF tunnel messages (0xD8-0xDE).
//!
//! These payloads are wrapped in StyreneMessage and sent over LXMF
//! between peers during tunnel establishment, teardown, and rekeying.

use serde::{Deserialize, Serialize};

/// TUNNEL_OFFER (0xD8) — initiator proposes a WireGuard tunnel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelOffer {
    /// Initiator's WireGuard public key (base64-encoded).
    pub wg_pubkey: String,
    /// Initiator's reachable endpoint (IP:port) for WireGuard.
    pub endpoint: String,
    /// Initiator's mesh overlay IP (derived from identity hash via BLAKE2b).
    pub mesh_ip: String,
    /// Pre-shared key for the tunnel (base64-encoded, 32 bytes).
    /// Generated per-offer for forward secrecy.
    pub psk: String,
    /// Preferred MTU (0 = use default).
    pub mtu: u16,
    /// Replay protection nonce (16 bytes, hex-encoded).
    pub nonce: String,
    /// Unix timestamp of the offer.
    pub timestamp: i64,
}

/// TUNNEL_ACCEPT (0xD9) — responder accepts the tunnel offer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelAccept {
    /// Responder's WireGuard public key (base64-encoded).
    pub wg_pubkey: String,
    /// Responder's reachable endpoint (IP:port) for WireGuard.
    pub endpoint: String,
    /// Responder's mesh overlay IP.
    pub mesh_ip: String,
    /// Echo back the offer's nonce for correlation.
    pub offer_nonce: String,
    /// Responder's own nonce.
    pub nonce: String,
    /// Unix timestamp of the acceptance.
    pub timestamp: i64,
}

/// TUNNEL_REJECT (0xDA) — responder rejects the tunnel offer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelReject {
    /// Human-readable reason for rejection.
    pub reason: String,
    /// Echo back the offer's nonce for correlation.
    pub offer_nonce: String,
}

/// TUNNEL_TEARDOWN (0xDB) — either side tears down a tunnel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelTeardown {
    /// Identity hash of the peer whose tunnel to tear down.
    pub peer_identity: String,
    /// Replay protection nonce.
    pub nonce: String,
}

/// TUNNEL_REKEY (0xDC) — either side proposes a new PSK.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelRekey {
    /// Identity hash of the tunnel peer.
    pub peer_identity: String,
    /// New pre-shared key (base64-encoded, 32 bytes).
    pub new_psk: String,
    /// Replay protection nonce.
    pub nonce: String,
    /// Unix timestamp.
    pub timestamp: i64,
}

/// TUNNEL_KEEPALIVE (0xDD) — control-channel keepalive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelKeepalive {
    /// Identity hash of the tunnel peer.
    pub peer_identity: String,
    /// Replay protection nonce.
    pub nonce: String,
}

/// TUNNEL_TOPOLOGY (0xDE) — hub broadcasts mesh VPN topology.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelTopology {
    /// List of peers in the mesh VPN.
    pub peers: Vec<TunnelTopologyPeer>,
    /// Replay protection nonce.
    pub nonce: String,
    /// Unix timestamp.
    pub timestamp: i64,
}

/// A peer entry in the topology broadcast.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelTopologyPeer {
    /// Peer's identity hash.
    pub identity: String,
    /// Peer's WireGuard endpoint (IP:port).
    pub endpoint: String,
    /// Peer's mesh overlay IP.
    pub mesh_ip: String,
    /// Peer's WireGuard public key (base64-encoded).
    pub wg_pubkey: String,
}

/// Helper: generate a 16-byte random nonce as hex string.
pub fn generate_nonce() -> String {
    let mut bytes = [0u8; 16];
    rand_core::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Helper: derive a deterministic mesh overlay IPv6 address from an identity hash.
///
/// Uses BLAKE2b-128(identity_hash) to produce the interface ID portion
/// of a ULA IPv6 address in the fd73:7479:7265:6e65::/64 prefix.
pub fn derive_mesh_ip(identity_hash: &str) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(identity_hash.as_bytes());
    format!(
        "fd73:7479:7265:6e65:{:02x}{:02x}:{:02x}{:02x}:{:02x}{:02x}:{:02x}{:02x}",
        hash[0], hash[1], hash[2], hash[3], hash[4], hash[5], hash[6], hash[7]
    )
}

/// Helper: current unix timestamp.
pub fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

use rand_core::RngCore;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonce_is_32_hex_chars() {
        let nonce = generate_nonce();
        assert_eq!(nonce.len(), 32);
        assert!(nonce.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn nonce_is_unique() {
        let a = generate_nonce();
        let b = generate_nonce();
        assert_ne!(a, b);
    }

    #[test]
    fn mesh_ip_is_deterministic() {
        let ip1 = derive_mesh_ip("abc123");
        let ip2 = derive_mesh_ip("abc123");
        assert_eq!(ip1, ip2);
    }

    #[test]
    fn mesh_ip_is_ula_ipv6() {
        let ip = derive_mesh_ip("test-identity-hash");
        assert!(ip.starts_with("fd73:7479:7265:6e65:"));
    }

    #[test]
    fn different_identities_different_ips() {
        let ip1 = derive_mesh_ip("identity-a");
        let ip2 = derive_mesh_ip("identity-b");
        assert_ne!(ip1, ip2);
    }

    #[test]
    fn tunnel_offer_roundtrip() {
        let offer = TunnelOffer {
            wg_pubkey: "base64pubkey==".into(),
            endpoint: "192.168.1.1:51820".into(),
            mesh_ip: derive_mesh_ip("test"),
            psk: "base64psk==".into(),
            mtu: 1420,
            nonce: generate_nonce(),
            timestamp: now_ts(),
        };
        let bytes = rmp_serde::to_vec(&offer).expect("serialize");
        let decoded: TunnelOffer = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(decoded.wg_pubkey, offer.wg_pubkey);
        assert_eq!(decoded.endpoint, offer.endpoint);
        assert_eq!(decoded.mesh_ip, offer.mesh_ip);
    }

    #[test]
    fn tunnel_accept_roundtrip() {
        let accept = TunnelAccept {
            wg_pubkey: "base64pubkey==".into(),
            endpoint: "10.0.0.1:51820".into(),
            mesh_ip: derive_mesh_ip("responder"),
            offer_nonce: generate_nonce(),
            nonce: generate_nonce(),
            timestamp: now_ts(),
        };
        let bytes = rmp_serde::to_vec(&accept).expect("serialize");
        let decoded: TunnelAccept = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(decoded.wg_pubkey, accept.wg_pubkey);
        assert_eq!(decoded.offer_nonce, accept.offer_nonce);
    }
}
