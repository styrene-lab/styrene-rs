//! Kernel entropy source — `/dev/random` via `getrandom`.
//!
//! Available on Linux, macOS, and any platform with `getrandom` support.
//! Enabled by the `kernel` feature (on by default).

use crate::{
    health::SourceHealth,
    pool::{EntropyPool, SourceId},
};
use super::EntropySource;

/// Entropy source drawing from the OS kernel's CSPRNG (`getrandom` syscall).
///
/// On Linux this is `/dev/urandom` post-initialization (blocks only until the
/// kernel's entropy pool is initialized at boot). On macOS it uses `getentropy`.
///
/// This source is always `Ok` once the kernel is initialized — it never degrades.
#[derive(Debug, Default)]
pub struct KernelSource {
    health: SourceHealth,
}

impl KernelSource {
    /// Create a new kernel entropy source.
    pub fn new() -> Self {
        Self { health: SourceHealth::Ok }
    }

    /// Attempt to fill `buf` from the kernel. Updates health on failure.
    fn try_fill(&mut self, buf: &mut [u8]) -> bool {
        match getrandom::getrandom(buf) {
            Ok(()) => {
                self.health = SourceHealth::Ok;
                true
            }
            Err(e) => {
                self.health = SourceHealth::Degraded(format!("getrandom failed: {e}"));
                false
            }
        }
    }
}

impl EntropySource for KernelSource {
    fn source_id(&self) -> SourceId {
        SourceId::KERNEL
    }

    fn health(&self) -> SourceHealth {
        self.health.clone()
    }

    fn poll(&mut self, pool: &mut EntropyPool) {
        let mut buf = [0u8; 64];
        if self.try_fill(&mut buf) {
            pool.add(SourceId::KERNEL, &buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::{EntropyPool, MIN_POOL_BYTES};

    #[test]
    fn kernel_source_is_initially_ok() {
        let src = KernelSource::new();
        assert!(src.health().is_ok());
    }

    #[test]
    fn kernel_source_fills_pool() {
        let mut src = KernelSource::new();
        let mut pool = EntropyPool::new();

        // Poll enough times to fill the pool
        for _ in 0..(MIN_POOL_BYTES / 64 + 1) {
            src.poll(&mut pool);
        }
        assert!(pool.ready(), "kernel source should fill pool to ready");
    }

    #[test]
    fn kernel_output_is_not_all_zeros() {
        let mut buf = [0u8; 64];
        getrandom::getrandom(&mut buf).expect("getrandom should succeed");
        assert!(buf.iter().any(|&b| b != 0), "kernel entropy should not be all zeros");
    }
}
