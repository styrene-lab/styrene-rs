//! Mesh Hub entropy source — ENTROPY_REQUEST RPC stub.
//!
//! Requests entropy from the Hub's multi-source pool via the LXMF
//! ENTROPY_REQUEST (0x50) / ENTROPY_GRANT (0x51) RPC protocol.
//!
//! This is a design stub. The full implementation requires `styrene-ipc`
//! to be wired in (blocked on Hub scaffolding and AppContext / Gap S5).
//! The trait implementation is complete — only the transport is stubbed.
//!
//! Enabled by the `mesh-source` feature.

use crate::{
    health::SourceHealth,
    pool::{EntropyPool, SourceId},
};
use super::EntropySource;

/// Purpose codes for ENTROPY_REQUEST — mirrors the RPC spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EntropyPurpose {
    /// RNS identity creation (X25519 + Ed25519).
    IdentityKeygen = 0x01,
    /// ML-KEM key generation.
    PqcKeygen = 0x02,
    /// IKEv2 / WireGuard PSK derivation.
    TunnelPsk = 0x03,
    /// DRBG reseed (no local hardware source).
    Reseed = 0x04,
}

/// Mesh Hub entropy source.
///
/// Connects to the Styrene Hub's entropy pool via LXMF RPC and requests
/// entropy grants for key generation events.
///
/// **Implementation status:** stub — transport layer not yet wired in.
/// The `poll()` method is a no-op until `styrene-ipc` integration lands.
///
/// See `styrene-hub/docs/entropy-mesh-pool.md` for the full protocol.
#[derive(Debug)]
pub struct MeshHubSource {
    /// Hub LXMF destination hash (hex string).
    hub_destination: String,
    health: SourceHealth,
    pool_depth: u8,
}

impl MeshHubSource {
    /// Create a new mesh hub source targeting the given Hub destination hash.
    pub fn new(hub_destination: impl Into<String>) -> Self {
        Self {
            hub_destination: hub_destination.into(),
            health: SourceHealth::Unavailable,
            pool_depth: 0,
        }
    }

    /// Update connectivity state — called when the Hub LXMF link changes.
    ///
    /// `pool_depth` is the 0–255 coarse signal from the last ENTROPY_GRANT.
    pub fn update_connectivity(&mut self, connected: bool, pool_depth: u8) {
        if connected {
            self.health = SourceHealth::Ok;
            self.pool_depth = pool_depth;
        } else {
            self.health = SourceHealth::Unavailable;
        }
    }

    /// Returns the last known Hub pool depth (0 = empty, 255 = saturated).
    pub fn pool_depth(&self) -> u8 {
        self.pool_depth
    }

    /// Returns the Hub destination hash this source is configured for.
    pub fn hub_destination(&self) -> &str {
        &self.hub_destination
    }
}

impl EntropySource for MeshHubSource {
    fn source_id(&self) -> SourceId {
        SourceId::MESH_HUB
    }

    fn health(&self) -> SourceHealth {
        self.health.clone()
    }

    fn poll(&mut self, _pool: &mut EntropyPool) {
        // Stub: full implementation requires styrene-ipc LXMF RPC transport.
        // When wired in:
        //   1. Send ENTROPY_REQUEST(0x50) to hub_destination
        //   2. Await ENTROPY_GRANT(0x51) response
        //   3. pool.add(SourceId::MESH_HUB, &grant.data)
        //   4. self.pool_depth = grant.pool_depth
        //
        // For now, the MeshHubSource is used by callers who explicitly call
        // `inject_grant()` after receiving an out-of-band ENTROPY_GRANT.
        log::debug!(
            "MeshHubSource::poll stub — hub transport not yet wired in ({})",
            self.hub_destination
        );
    }
}

impl MeshHubSource {
    /// Inject an ENTROPY_GRANT payload received via external transport.
    ///
    /// Called by the daemon's RPC handler when an ENTROPY_GRANT is received.
    /// This is the integration point until `poll()` is fully wired to LXMF.
    pub fn inject_grant(&mut self, pool: &mut EntropyPool, data: &[u8], pool_depth: u8) {
        self.pool_depth = pool_depth;
        if !data.is_empty() {
            pool.add(SourceId::MESH_HUB, data);
            log::debug!(
                "MeshHubSource: injected {} bytes from hub (pool_depth={})",
                data.len(),
                pool_depth
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::{EntropyPool, MIN_POOL_BYTES};

    #[test]
    fn mesh_source_starts_unavailable() {
        let src = MeshHubSource::new("aabbccddeeff00112233445566778899");
        assert_eq!(src.health(), SourceHealth::Unavailable);
        assert!(!src.available());
    }

    #[test]
    fn update_connectivity_changes_health() {
        let mut src = MeshHubSource::new("aabbccddeeff00112233445566778899");
        src.update_connectivity(true, 200);
        assert!(src.health().is_ok());
        assert_eq!(src.pool_depth(), 200);
    }

    #[test]
    fn inject_grant_fills_pool() {
        let mut src = MeshHubSource::new("aabbccddeeff00112233445566778899");
        let mut pool = EntropyPool::new();
        src.inject_grant(&mut pool, &[0xAB; MIN_POOL_BYTES], 180);
        assert!(pool.ready(), "injected grant should fill pool");
        assert_eq!(src.pool_depth(), 180);
    }

    #[test]
    fn inject_empty_grant_is_noop() {
        let mut src = MeshHubSource::new("aabbccddeeff00112233445566778899");
        let mut pool = EntropyPool::new();
        src.inject_grant(&mut pool, &[], 0);
        assert!(!pool.ready(), "empty grant should not advance pool");
    }
}
