//! Entropy source implementations.
//!
//! Each source implements the [`EntropySource`] trait and contributes bytes
//! to an [`crate::EntropyPool`] when polled. Sources are independent — the pool
//! mixes all contributions, so a weak or absent source does not degrade the
//! others.

use crate::{health::SourceHealth, pool::EntropyPool};

#[cfg(feature = "kernel")]
pub mod kernel;

#[cfg(feature = "jitter")]
pub mod jitter;

#[cfg(feature = "hardware-trng")]
pub mod hardware;

#[cfg(feature = "mesh-source")]
pub mod mesh;

// Always re-export concrete types for enabled features
#[cfg(feature = "kernel")]
pub use kernel::KernelSource;

#[cfg(feature = "jitter")]
pub use jitter::JitterSource;

#[cfg(feature = "hardware-trng")]
pub use hardware::HardwareSource;

#[cfg(feature = "mesh-source")]
pub use mesh::MeshHubSource;

/// Trait implemented by all entropy sources.
///
/// Sources contribute raw entropy bytes to an [`EntropyPool`]. They report
/// their health state and optionally a quality hint (used for pool weighting).
pub trait EntropySource: Send + Sync {
    /// Stable identifier for this source type (used for pool labelling).
    fn source_id(&self) -> crate::pool::SourceId;

    /// Returns the current health of this source.
    fn health(&self) -> SourceHealth;

    /// Returns `true` if the source is healthy and should be polled.
    fn available(&self) -> bool {
        self.health().is_ok()
    }

    /// Poll the source: if healthy, add entropy bytes to `pool`.
    ///
    /// Implementations should silently skip contribution when unhealthy
    /// rather than returning an error — the pool handles missing sources.
    fn poll(&mut self, pool: &mut EntropyPool);
}
