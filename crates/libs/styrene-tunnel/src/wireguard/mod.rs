//! WireGuard backend for classical (non-PQC) tunnels.
//!
//! Uses RNS X25519 identity keys directly as WireGuard peer keys,
//! providing zero-configuration tunnel establishment after RNS discovery.
//!
//! # System Requirements
//!
//! - Linux kernel 5.6+ (WireGuard built-in)
//! - `CAP_NET_ADMIN` capability for the daemon process

use crate::error::TunnelError;
use crate::traits::{TunnelBackend, TunnelId, TunnelInfo, TunnelParams, TunnelState};

/// WireGuard tunnel backend.
///
/// Manages WireGuard interfaces and peers via kernel netlink.
pub struct WireGuardBackend {
    /// WireGuard interface name (default: "wg-styrene").
    interface_name: String,
}

impl WireGuardBackend {
    /// Create a new WireGuard backend with the default interface name.
    pub fn new() -> Self {
        Self { interface_name: "wg-styrene".into() }
    }

    /// Create a new WireGuard backend with a custom interface name.
    pub fn with_interface(name: impl Into<String>) -> Self {
        Self { interface_name: name.into() }
    }
}

#[async_trait::async_trait]
impl TunnelBackend for WireGuardBackend {
    fn name(&self) -> &str {
        "wireguard"
    }

    async fn is_available(&self) -> bool {
        // Check if WireGuard kernel module is loaded
        // On Linux 5.6+, it's built-in
        #[cfg(target_os = "linux")]
        {
            tokio::fs::metadata("/sys/module/wireguard").await.is_ok()
                || tokio::fs::metadata("/proc/modules")
                    .await
                    .map(|_| {
                        // Kernel 5.6+ has built-in WireGuard
                        true
                    })
                    .unwrap_or(false)
        }
        #[cfg(not(target_os = "linux"))]
        {
            false
        }
    }

    async fn establish(&self, params: TunnelParams) -> Result<TunnelId, TunnelError> {
        // TODO: Implement WireGuard tunnel establishment
        //   1. Create/configure WireGuard interface via netlink
        //   2. Set private key (derived from RNS identity or generated)
        //   3. Add peer with public key from RNS identity X25519 key
        //   4. Set endpoint, allowed IPs, PSK slot
        //   5. Bring interface up
        let _ = params;
        Err(TunnelError::Backend("WireGuard backend not yet implemented".into()))
    }

    async fn teardown(&self, tunnel_id: &str) -> Result<(), TunnelError> {
        // TODO: Remove peer from WireGuard interface
        let _ = tunnel_id;
        Err(TunnelError::Backend("WireGuard backend not yet implemented".into()))
    }

    async fn rekey(&self, tunnel_id: &str, new_psk: &[u8; 32]) -> Result<(), TunnelError> {
        // TODO: Update PSK for peer
        let _ = (tunnel_id, new_psk);
        Err(TunnelError::Backend("WireGuard backend not yet implemented".into()))
    }

    async fn status(&self, tunnel_id: &str) -> Result<TunnelInfo, TunnelError> {
        // TODO: Query WireGuard interface for peer status
        let _ = tunnel_id;
        Err(TunnelError::Backend("WireGuard backend not yet implemented".into()))
    }

    async fn list_tunnels(&self) -> Result<Vec<TunnelInfo>, TunnelError> {
        // TODO: List all peers on the WireGuard interface
        Err(TunnelError::Backend("WireGuard backend not yet implemented".into()))
    }
}
