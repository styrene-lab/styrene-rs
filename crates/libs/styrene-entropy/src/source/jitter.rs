//! CPU jitter entropy source.
//!
//! Harvests entropy from timing variation in CPU execution — differences in
//! cache hit/miss timing, branch prediction, memory access latency, and other
//! micro-architectural non-determinism.
//!
//! This is NOT a certified entropy source and should be used as a supplemental
//! pool contributor alongside kernel or hardware sources, not as a primary source.
//! It is most useful on constrained devices (RP2040, ESP32) where no other
//! source is available.
//!
//! Enabled by the `jitter` feature.

use std::time::Instant;

use crate::{
    health::SourceHealth,
    pool::{EntropyPool, SourceId},
};
use super::EntropySource;

/// Number of timing samples per byte of entropy output.
/// Higher = more entropy per byte but slower. 64 is a reasonable default.
const SAMPLES_PER_BYTE: usize = 64;

/// CPU jitter entropy source.
///
/// Collects timing jitter from a folding loop — each iteration measures
/// nanosecond-resolution elapsed time and XORs the LSBs into an accumulator.
/// Multiple samples are combined per output byte via XOR-folding.
#[derive(Debug)]
pub struct JitterSource {
    samples_per_byte: usize,
}

impl JitterSource {
    /// Create a new jitter source with the default sample count.
    pub fn new() -> Self {
        Self { samples_per_byte: SAMPLES_PER_BYTE }
    }

    /// Create a jitter source with a custom samples-per-byte count.
    /// Higher values are slower but provide more mixing.
    pub fn with_samples(samples_per_byte: usize) -> Self {
        Self { samples_per_byte: samples_per_byte.max(8) }
    }

    /// Collect one byte of jitter entropy.
    fn collect_byte(&self) -> u8 {
        let mut acc: u8 = 0;
        let baseline = Instant::now();

        for i in 0..self.samples_per_byte {
            // Spin on a simple computation — enough to vary with cache state
            let elapsed = baseline.elapsed().subsec_nanos();
            // XOR-fold the low bits of nanosecond timing
            acc = acc.wrapping_add((elapsed as u8).wrapping_mul(i as u8 | 1));
        }
        acc
    }

    /// Collect `n` bytes of jitter entropy into `buf`.
    fn collect_bytes(&self, buf: &mut [u8]) {
        for byte in buf.iter_mut() {
            *byte = self.collect_byte();
        }
    }
}

impl Default for JitterSource {
    fn default() -> Self {
        Self::new()
    }
}

impl EntropySource for JitterSource {
    fn source_id(&self) -> SourceId {
        SourceId::JITTER
    }

    fn health(&self) -> SourceHealth {
        // Jitter source is always "available" — it may be weak but never fails
        SourceHealth::Ok
    }

    fn poll(&mut self, pool: &mut EntropyPool) {
        let mut buf = [0u8; 32];
        self.collect_bytes(&mut buf);
        pool.add(SourceId::JITTER, &buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jitter_produces_nonzero_output() {
        let src = JitterSource::new();
        let mut buf = [0u8; 32];
        src.collect_bytes(&mut buf);
        // Not guaranteed to be non-zero but extremely likely on any real CPU
        let all_zero = buf.iter().all(|&b| b == 0);
        assert!(!all_zero, "jitter output should not be all zeros");
    }

    #[test]
    fn jitter_is_always_healthy() {
        let src = JitterSource::new();
        assert!(src.health().is_ok());
    }

    #[test]
    fn jitter_fills_pool() {
        let mut src = JitterSource::new();
        let mut pool = EntropyPool::new();
        // Poll enough to fill the pool
        for _ in 0..2 {
            src.poll(&mut pool);
        }
        assert!(pool.ready());
    }
}
