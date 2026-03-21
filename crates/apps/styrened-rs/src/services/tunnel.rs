//! TunnelService — tunnel lifecycle management.
//!
//! Owns: tunnel lifecycle. Wraps styrene-tunnel crate.
//! Package: H
//!
//! This service is DEFER'd in the initial daemon port wave. It provides
//! a minimal skeleton that will be filled when PQC tunnel, WireGuard,
//! and strongSwan integration are ported from the Python daemon.
//!
//! See design-tree node `pqc-tunnel-*` for the deferred design.

/// Placeholder for tunnel lifecycle management.
///
/// Will eventually wrap the `styrene-tunnel` crate (2,249 LOC) for:
/// - PQC session initiation and management
/// - WireGuard tunnel lifecycle
/// - strongSwan SA management
#[derive(Default)]
pub struct TunnelService {
    // Fields will be added when tunnel support is ported
}

impl TunnelService {
    pub fn new() -> Self {
        Self::default()
    }
}
