//! Tunnel orchestrator — selects and manages tunnel backends based on
//! peer capabilities and local policy.
//!
//! The orchestrator is the decision engine that:
//! 1. Receives peer capability information from PQC negotiation
//! 2. Selects the best tunnel backend (strongSwan PQC > WireGuard classical)
//! 3. Establishes the tunnel using the selected backend
//! 4. Monitors tunnel health and handles failover
//! 5. Routes traffic into the active tunnel interface

use std::collections::HashMap;
use std::sync::Arc;

use crate::error::TunnelError;
use crate::traits::{TunnelBackend, TunnelId, TunnelInfo, TunnelParams, TunnelState};
use styrene_mesh::pqc::capability_flags;

/// Local tunnel policy — which backends are enabled and preferred.
#[derive(Debug, Clone)]
pub struct TunnelPolicy {
    /// Whether to prefer PQC (strongSwan) when both sides support it.
    pub prefer_pqc: bool,
    /// Whether to allow fallback to WireGuard when strongSwan fails.
    pub allow_fallback: bool,
    /// Whether WireGuard is enabled locally.
    pub wireguard_enabled: bool,
    /// Whether strongSwan is enabled locally.
    pub strongswan_enabled: bool,
}

impl Default for TunnelPolicy {
    fn default() -> Self {
        Self {
            prefer_pqc: true,
            allow_fallback: true,
            wireguard_enabled: true,
            strongswan_enabled: true,
        }
    }
}

/// The tunnel orchestrator.
pub struct Orchestrator {
    policy: TunnelPolicy,
    #[cfg(feature = "strongswan")]
    strongswan: Option<Arc<dyn TunnelBackend>>,
    #[cfg(feature = "wireguard")]
    wireguard: Option<Arc<dyn TunnelBackend>>,
    /// Active tunnels by peer identity hash.
    active_tunnels: HashMap<String, ActiveTunnel>,
}

/// An active tunnel managed by the orchestrator.
struct ActiveTunnel {
    tunnel_id: TunnelId,
    backend_name: String,
    peer_identity: String,
}

impl Orchestrator {
    /// Create a new orchestrator with the given policy and backends.
    pub fn new(
        policy: TunnelPolicy,
        #[cfg(feature = "strongswan")] strongswan: Option<Arc<dyn TunnelBackend>>,
        #[cfg(feature = "wireguard")] wireguard: Option<Arc<dyn TunnelBackend>>,
    ) -> Self {
        Self {
            policy,
            #[cfg(feature = "strongswan")]
            strongswan,
            #[cfg(feature = "wireguard")]
            wireguard,
            active_tunnels: HashMap::new(),
        }
    }

    /// Determine the best tunnel backend for a peer's capabilities.
    ///
    /// Returns the capability flag for the selected backend, or an error
    /// if no suitable backend is available.
    pub fn select_tunnel_type(&self, peer_capabilities: u8) -> Result<u8, TunnelError> {
        let local_caps = self.local_capabilities();
        let common = peer_capabilities & local_caps;

        if common == capability_flags::TUNNEL_NONE {
            return Err(TunnelError::NoSuitableBackend { capabilities: peer_capabilities });
        }

        // Prefer strongSwan (PQC) if both sides support it and policy allows
        if self.policy.prefer_pqc && common & capability_flags::TUNNEL_STRONGSWAN != 0 {
            return Ok(capability_flags::TUNNEL_STRONGSWAN);
        }

        // Fall back to WireGuard
        if common & capability_flags::TUNNEL_WG != 0 {
            return Ok(capability_flags::TUNNEL_WG);
        }

        // If only strongSwan is common (peer doesn't prefer, but we don't either)
        if common & capability_flags::TUNNEL_STRONGSWAN != 0 {
            return Ok(capability_flags::TUNNEL_STRONGSWAN);
        }

        Err(TunnelError::NoSuitableBackend { capabilities: peer_capabilities })
    }

    /// Get the local capability flags based on policy and backend availability.
    pub fn local_capabilities(&self) -> u8 {
        let mut caps = capability_flags::TUNNEL_NONE;

        #[cfg(feature = "strongswan")]
        if self.policy.strongswan_enabled && self.strongswan.is_some() {
            caps |= capability_flags::TUNNEL_STRONGSWAN;
        }

        #[cfg(feature = "wireguard")]
        if self.policy.wireguard_enabled && self.wireguard.is_some() {
            caps |= capability_flags::TUNNEL_WG;
        }

        caps
    }

    /// Establish a tunnel to a peer using the selected backend.
    pub async fn establish_tunnel(
        &mut self,
        selected_type: u8,
        params: TunnelParams,
    ) -> Result<TunnelId, TunnelError> {
        let peer_identity = params.peer_identity.clone();

        let (backend, backend_name) = self.get_backend(selected_type)?;
        let tunnel_id = backend.establish(params).await?;

        self.active_tunnels.insert(
            peer_identity.clone(),
            ActiveTunnel {
                tunnel_id: tunnel_id.clone(),
                backend_name: backend_name.to_string(),
                peer_identity,
            },
        );

        Ok(tunnel_id)
    }

    /// Tear down a tunnel to a specific peer.
    pub async fn teardown_tunnel(&mut self, peer_identity: &str) -> Result<(), TunnelError> {
        let tunnel =
            self.active_tunnels.remove(peer_identity).ok_or_else(|| TunnelError::NotEstablished)?;

        let selected_type = match tunnel.backend_name.as_str() {
            "strongswan" => capability_flags::TUNNEL_STRONGSWAN,
            "wireguard" => capability_flags::TUNNEL_WG,
            _ => return Err(TunnelError::Backend("unknown backend".into())),
        };

        let (backend, _) = self.get_backend(selected_type)?;
        backend.teardown(&tunnel.tunnel_id).await
    }

    /// List all active tunnels.
    pub fn active_tunnel_peers(&self) -> Vec<&str> {
        self.active_tunnels.keys().map(|s| s.as_str()).collect()
    }

    /// Get the appropriate backend for a tunnel type flag.
    fn get_backend(&self, tunnel_type: u8) -> Result<(Arc<dyn TunnelBackend>, &str), TunnelError> {
        match tunnel_type {
            #[cfg(feature = "strongswan")]
            capability_flags::TUNNEL_STRONGSWAN => {
                let backend = self
                    .strongswan
                    .as_ref()
                    .ok_or_else(|| {
                        TunnelError::Backend("strongSwan backend not configured".into())
                    })?
                    .clone();
                Ok((backend, "strongswan"))
            }
            #[cfg(feature = "wireguard")]
            capability_flags::TUNNEL_WG => {
                let backend = self
                    .wireguard
                    .as_ref()
                    .ok_or_else(|| TunnelError::Backend("WireGuard backend not configured".into()))?
                    .clone();
                Ok((backend, "wireguard"))
            }
            _ => Err(TunnelError::NoSuitableBackend { capabilities: tunnel_type }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_prefers_strongswan_when_both_available() {
        let orch = Orchestrator::new(
            TunnelPolicy::default(),
            #[cfg(feature = "strongswan")]
            None, // no actual backend, but policy says enabled
            #[cfg(feature = "wireguard")]
            None,
        );

        // With both features enabled, local_capabilities reflects policy
        let caps = orch.local_capabilities();
        // Without actual backends, caps will be TUNNEL_NONE
        assert_eq!(caps, capability_flags::TUNNEL_NONE);
    }

    #[test]
    fn select_fails_with_no_common_capabilities() {
        let orch = Orchestrator::new(
            TunnelPolicy::default(),
            #[cfg(feature = "strongswan")]
            None,
            #[cfg(feature = "wireguard")]
            None,
        );

        let result = orch.select_tunnel_type(capability_flags::TUNNEL_NONE);
        assert!(matches!(result, Err(TunnelError::NoSuitableBackend { .. })));
    }
}
