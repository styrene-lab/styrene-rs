//! strongSwan VICI backend for IPsec + ML-KEM hybrid tunnels.
//!
//! Communicates with the strongSwan charon daemon via the VICI protocol
//! to manage IKEv2 Security Associations with post-quantum key exchange.
//!
//! # System Requirements
//!
//! - strongSwan 6.0+ with `ke-mlkem` plugin
//! - Linux kernel IPsec (XFRM) subsystem
//! - VICI socket accessible to the daemon user

mod sa;
mod vici;

use std::net::IpAddr;

use crate::error::TunnelError;
use crate::traits::{TunnelBackend, TunnelId, TunnelInfo, TunnelParams, TunnelState};

/// strongSwan tunnel backend.
///
/// Manages IPsec Security Associations via the VICI protocol.
pub struct StrongSwanBackend {
    /// Path to the VICI socket (default: /var/run/charon.vici).
    vici_socket: String,
}

impl StrongSwanBackend {
    /// Create a new strongSwan backend with the default VICI socket path.
    pub fn new() -> Self {
        Self { vici_socket: "/var/run/charon.vici".into() }
    }

    /// Create a new strongSwan backend with a custom VICI socket path.
    pub fn with_socket(path: impl Into<String>) -> Self {
        Self { vici_socket: path.into() }
    }
}

#[async_trait::async_trait]
impl TunnelBackend for StrongSwanBackend {
    fn name(&self) -> &str {
        "strongswan"
    }

    async fn is_available(&self) -> bool {
        // Check if VICI socket exists and is accessible
        tokio::fs::metadata(&self.vici_socket).await.is_ok()
    }

    async fn establish(&self, params: TunnelParams) -> Result<TunnelId, TunnelError> {
        // TODO: Implement VICI connection initiation
        //   1. Connect to VICI socket
        //   2. load-shared: inject PSK derived from PQC session
        //   3. load-conn: configure IKEv2 connection with ML-KEM hybrid proposal
        //   4. initiate: start SA negotiation
        //   5. Subscribe to ike-updown events for state tracking
        let _ = params;
        Err(TunnelError::Backend("strongSwan backend not yet implemented".into()))
    }

    async fn teardown(&self, tunnel_id: &str) -> Result<(), TunnelError> {
        // TODO: terminate SA via VICI
        let _ = tunnel_id;
        Err(TunnelError::Backend("strongSwan backend not yet implemented".into()))
    }

    async fn rekey(&self, tunnel_id: &str, new_psk: &[u8; 32]) -> Result<(), TunnelError> {
        // TODO: rekey SA via VICI (load new PSK, trigger reauth)
        let _ = (tunnel_id, new_psk);
        Err(TunnelError::Backend("strongSwan backend not yet implemented".into()))
    }

    async fn status(&self, tunnel_id: &str) -> Result<TunnelInfo, TunnelError> {
        // TODO: list-sas via VICI, filter by tunnel_id
        let _ = tunnel_id;
        Err(TunnelError::Backend("strongSwan backend not yet implemented".into()))
    }

    async fn list_tunnels(&self) -> Result<Vec<TunnelInfo>, TunnelError> {
        // TODO: list-sas via VICI, return all styrene-managed SAs
        Err(TunnelError::Backend("strongSwan backend not yet implemented".into()))
    }
}
