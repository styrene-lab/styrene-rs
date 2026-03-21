//! Fortuna-style entropy accumulator.
//!
//! Maintains 8 independent pools, each accumulating raw bytes from entropy
//! sources via a SHA-256 running hash. On reseed, pools are drained and
//! combined using the Fortuna schedule: pool `i` contributes to reseed `j`
//! if `2^i` divides `j`. This means pool 0 contributes to every reseed,
//! pool 1 to every other, pool 2 to every 4th, etc.
//!
//! The resulting 32-byte seed is passed to the [`crate::Drbg`] for reseeding.

use sha2::{Digest, Sha256};
use zeroize::Zeroize;

/// Minimum bytes pool 0 must accumulate before a reseed is permitted.
/// 32 bytes = 256 bits — one full SHA-256 block of entropy input.
pub const MIN_POOL_BYTES: usize = 32;

/// Number of independent accumulator pools.
pub const POOL_COUNT: usize = 8;

/// Source identifier — used to label contributions by origin.
/// Values 0x00–0x7F are reserved for built-in sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceId(pub u8);

impl SourceId {
    /// Kernel entropy source (`/dev/random` / getrandom).
    pub const KERNEL: Self = Self(0x00);
    /// CPU jitter entropy source.
    pub const JITTER: Self = Self(0x01);
    /// Hardware TRNG coprocessor (nRF52840).
    pub const HARDWARE: Self = Self(0x02);
    /// Mesh Hub entropy pool (ENTROPY_GRANT RPC response).
    pub const MESH_HUB: Self = Self(0x03);
}

/// A single accumulator pool — SHA-256 running hash + byte count.
#[derive(Debug)]
struct Pool {
    hasher: Sha256,
    bytes_accumulated: usize,
}

impl Pool {
    fn new() -> Self {
        Self { hasher: Sha256::new(), bytes_accumulated: 0 }
    }

    /// Add bytes from a source into this pool.
    fn add(&mut self, source_id: SourceId, data: &[u8]) {
        // Fortuna pool format: source_id (1 byte) || len (1 byte) || data
        self.hasher.update([source_id.0]);
        self.hasher.update([data.len().min(255) as u8]);
        self.hasher.update(data);
        self.bytes_accumulated += data.len();
    }

    /// Drain the pool: finalize hash, reset state, return 32-byte digest.
    fn drain(&mut self) -> [u8; 32] {
        let result = self.hasher.finalize_reset();
        self.bytes_accumulated = 0;
        // finalize_reset re-initializes the hasher — no need to replace
        result.into()
    }

    fn bytes_accumulated(&self) -> usize {
        self.bytes_accumulated
    }
}

/// Fortuna-style entropy accumulator with 8 pools.
///
/// Thread safety: not `Send` by itself — wrap in `Mutex` for shared access.
pub struct EntropyPool {
    pools: [Pool; POOL_COUNT],
    reseed_count: u64,
}

impl std::fmt::Debug for EntropyPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EntropyPool")
            .field("reseed_count", &self.reseed_count)
            .field("pool0_bytes", &self.pools[0].bytes_accumulated())
            .finish()
    }
}

impl EntropyPool {
    /// Create a new, empty entropy pool.
    pub fn new() -> Self {
        Self {
            pools: std::array::from_fn(|_| Pool::new()),
            reseed_count: 0,
        }
    }

    /// Add bytes from a source into the pool.
    ///
    /// Pool 0 always receives the full input — it is the primary trigger pool
    /// and must accumulate [`MIN_POOL_BYTES`] before a reseed is permitted.
    /// For larger inputs (>16 bytes), the data is also distributed across
    /// pools 1–7 for additional diversity in the Fortuna reseed schedule.
    ///
    /// `source_id` labels the origin. `data` is raw entropy bytes.
    pub fn add(&mut self, source_id: SourceId, data: &[u8]) {
        // Pool 0 always gets the full contribution — drives reseed trigger.
        self.pools[0].add(source_id, data);

        // For larger inputs also distribute across pools 1–N for diversity.
        if data.len() > 16 {
            let secondary_count = POOL_COUNT - 1;
            let chunk_size = (data.len() / secondary_count).max(1);
            for (i, chunk) in data.chunks(chunk_size).enumerate() {
                let pool_idx = (i % secondary_count) + 1;
                self.pools[pool_idx].add(source_id, chunk);
            }
        }
    }

    /// Returns `true` if pool 0 has accumulated enough bytes for a reseed.
    pub fn ready(&self) -> bool {
        self.pools[0].bytes_accumulated() >= MIN_POOL_BYTES
    }

    /// Returns the byte count accumulated in pool 0 (the primary pool).
    pub fn pool0_bytes(&self) -> usize {
        self.pools[0].bytes_accumulated()
    }

    /// Drain the pools participating in this reseed and return a 32-byte seed.
    ///
    /// Uses the Fortuna schedule: pool `i` participates in reseed `j` if
    /// `2^i` divides `j`. After drain, participating pools are reset.
    ///
    /// Returns `None` if the pool is not ready (pool 0 below threshold).
    pub fn drain_seed(&mut self) -> Option<[u8; 32]> {
        if !self.ready() {
            return None;
        }

        self.reseed_count = self.reseed_count.saturating_add(1);
        let reseed = self.reseed_count;

        // Combine digests from participating pools using nested SHA-256
        let mut outer = Sha256::new();
        for i in 0..POOL_COUNT {
            // Pool i participates if 2^i divides the reseed count
            let threshold = 1u64 << i;
            if reseed % threshold == 0 {
                let pool_digest = self.pools[i].drain();
                outer.update(pool_digest);
            }
        }

        let mut seed: [u8; 32] = outer.finalize().into();
        // A second pass adds forward separation between consecutive seeds
        let seed2: [u8; 32] = Sha256::digest(seed).into();
        seed.zeroize();
        Some(seed2)
    }

    /// Total number of reseeds performed.
    pub fn reseed_count(&self) -> u64 {
        self.reseed_count
    }
}

impl Default for EntropyPool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_not_ready_when_empty() {
        let pool = EntropyPool::new();
        assert!(!pool.ready());
    }

    #[test]
    fn pool_ready_after_sufficient_input() {
        let mut pool = EntropyPool::new();
        pool.add(SourceId::KERNEL, &[0u8; MIN_POOL_BYTES]);
        assert!(pool.ready());
    }

    #[test]
    fn drain_returns_none_when_not_ready() {
        let mut pool = EntropyPool::new();
        pool.add(SourceId::KERNEL, &[0u8; MIN_POOL_BYTES - 1]);
        assert!(pool.drain_seed().is_none());
    }

    #[test]
    fn drain_returns_seed_when_ready() {
        let mut pool = EntropyPool::new();
        pool.add(SourceId::KERNEL, &[0xAB; MIN_POOL_BYTES]);
        let seed = pool.drain_seed();
        assert!(seed.is_some(), "should return seed when ready");
    }

    #[test]
    fn drain_resets_pool() {
        let mut pool = EntropyPool::new();
        pool.add(SourceId::KERNEL, &[0xAB; MIN_POOL_BYTES]);
        let _ = pool.drain_seed();
        assert!(!pool.ready(), "pool should not be ready after drain");
    }

    #[test]
    fn different_inputs_produce_different_seeds() {
        let mut pool_a = EntropyPool::new();
        pool_a.add(SourceId::KERNEL, &[0xAA; MIN_POOL_BYTES]);
        let seed_a = pool_a.drain_seed().expect("should be ready");

        let mut pool_b = EntropyPool::new();
        pool_b.add(SourceId::KERNEL, &[0xBB; MIN_POOL_BYTES]);
        let seed_b = pool_b.drain_seed().expect("should be ready");

        assert_ne!(seed_a, seed_b, "different inputs must produce different seeds");
    }

    #[test]
    fn reseed_count_increments() {
        let mut pool = EntropyPool::new();
        assert_eq!(pool.reseed_count(), 0);
        pool.add(SourceId::KERNEL, &[0u8; MIN_POOL_BYTES]);
        pool.drain_seed().expect("should drain");
        assert_eq!(pool.reseed_count(), 1);
    }

    #[test]
    fn multiple_source_ids_accepted() {
        let mut pool = EntropyPool::new();
        pool.add(SourceId::KERNEL, &[0x11; 16]);
        pool.add(SourceId::HARDWARE, &[0x22; 16]);
        assert!(pool.ready());
        let seed = pool.drain_seed().expect("mixed sources should produce seed");
        assert_eq!(seed.len(), 32);
    }
}
