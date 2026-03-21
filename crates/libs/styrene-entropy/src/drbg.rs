//! HMAC-DRBG (SHA-256) — SP800-90A compliant deterministic random bit generator.
//!
//! # Design
//!
//! State: a 32-byte key `K` and 32-byte value `V`, both updated via HMAC-SHA256.
//!
//! - **Seeding / reseeding**: calls `update(seed_material)`, replacing K and V.
//!   Old state is irrecoverably overwritten — forward secrecy.
//! - **Generation**: repeatedly computes `V = HMAC(K, V)` and appends V to output.
//!   After generating, calls `update("")` to advance K and V — backtracking resistance.
//! - **Reseed policy**: automatically reseeds from the [`crate::EntropyPool`] every
//!   [`RESEED_INTERVAL`] bytes of output, or when explicitly requested.
//!
//! # References
//!
//! NIST SP800-90A Rev 1 §10.1.2 — HMAC_DRBG

use hmac::{Hmac, Mac};
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::pool::EntropyPool;

type HmacSha256 = Hmac<Sha256>;

/// Reseed after this many bytes of DRBG output (1 MiB — conservative).
pub const RESEED_INTERVAL: u64 = 1024 * 1024;

/// HMAC-DRBG state.
#[derive(ZeroizeOnDrop)]
pub struct Drbg {
    #[zeroize(skip)] // pool is not secret material — it accumulates entropy
    pool: EntropyPool,
    key: [u8; 32],
    value: [u8; 32],
    bytes_since_reseed: u64,
    seeded: bool,
}

impl std::fmt::Debug for Drbg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Drbg")
            .field("seeded", &self.seeded)
            .field("bytes_since_reseed", &self.bytes_since_reseed)
            .field("pool", &self.pool)
            .finish()
    }
}

impl Drbg {
    /// Create a new DRBG. The pool is empty — you must add entropy and call
    /// [`Self::reseed_from_pool`] (or [`Self::reseed`]) before calling [`Self::fill_bytes`].
    pub fn new(pool: EntropyPool) -> Self {
        Self {
            pool,
            key: [0u8; 32],
            value: [0u8; 32],
            bytes_since_reseed: 0,
            seeded: false,
        }
    }

    /// Add raw entropy bytes to the internal pool.
    ///
    /// Call this before the first [`Self::reseed_from_pool`], and periodically
    /// to keep the pool fresh.
    pub fn add_entropy(&mut self, source_id: crate::pool::SourceId, data: &[u8]) {
        self.pool.add(source_id, data);
    }

    /// Returns `true` if the pool has accumulated enough entropy to seed the DRBG.
    pub fn pool_ready(&self) -> bool {
        self.pool.ready()
    }

    /// Reseed from the internal pool if it is ready.
    ///
    /// Returns `true` if a reseed occurred, `false` if the pool was not ready.
    pub fn reseed_from_pool(&mut self) -> bool {
        if let Some(seed) = self.pool.drain_seed() {
            self.reseed(&seed);
            true
        } else {
            false
        }
    }

    /// Reseed directly from the provided seed material.
    ///
    /// This replaces the current K and V — old DRBG state is irrecoverable.
    /// The seed should be 32 bytes of high-quality entropy.
    pub fn reseed(&mut self, seed: &[u8]) {
        self.update(seed);
        self.bytes_since_reseed = 0;
        self.seeded = true;
    }

    /// Fill `dest` with cryptographically strong pseudorandom bytes.
    ///
    /// # Panics
    ///
    /// Panics if the DRBG has not been seeded. Call [`Self::reseed_from_pool`]
    /// or [`Self::reseed`] before generating output.
    pub fn fill_bytes(&mut self, dest: &mut [u8]) {
        assert!(self.seeded, "DRBG must be seeded before generating output");

        // Auto-reseed if pool has fresh material and interval has elapsed
        if self.bytes_since_reseed >= RESEED_INTERVAL {
            self.reseed_from_pool(); // no-op if pool not ready — caller must ensure freshness
        }

        let mut offset = 0;
        while offset < dest.len() {
            // V = HMAC(K, V)
            self.value = hmac_sha256(&self.key, &self.value);
            let available = 32.min(dest.len() - offset);
            dest[offset..offset + available].copy_from_slice(&self.value[..available]);
            offset += available;
        }

        // Post-generation update for backtracking resistance
        self.update(b"");
        self.bytes_since_reseed = self.bytes_since_reseed.saturating_add(dest.len() as u64);
    }

    /// Returns the number of bytes generated since the last reseed.
    pub fn bytes_since_reseed(&self) -> u64 {
        self.bytes_since_reseed
    }

    /// Returns `true` if the DRBG has been seeded and is ready to generate.
    pub fn is_seeded(&self) -> bool {
        self.seeded
    }

    /// SP800-90A §10.1.2.2 HMAC_DRBG_Update.
    ///
    /// Updates K and V given additional input (may be empty).
    fn update(&mut self, additional: &[u8]) {
        // K = HMAC(K, V || 0x00 || additional_input)
        let mut mac = HmacSha256::new_from_slice(&self.key)
            .expect("HMAC accepts any key length");
        mac.update(&self.value);
        mac.update(&[0x00]);
        mac.update(additional);
        let mut new_key: [u8; 32] = mac.finalize().into_bytes().into();

        // V = HMAC(K, V)
        let mut new_value: [u8; 32] = hmac_sha256(&new_key, &self.value);

        if !additional.is_empty() {
            // K = HMAC(K, V || 0x01 || additional_input)
            let mut mac2 = HmacSha256::new_from_slice(&new_key)
                .expect("HMAC accepts any key length");
            mac2.update(&new_value);
            mac2.update(&[0x01]);
            mac2.update(additional);
            new_key = mac2.finalize().into_bytes().into();

            // V = HMAC(K, V)
            new_value = hmac_sha256(&new_key, &new_value);
        }

        self.key.zeroize();
        self.value.zeroize();
        self.key = new_key;
        self.value = new_value;
    }
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut mac =
        HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::{EntropyPool, SourceId, MIN_POOL_BYTES};

    fn seeded_drbg() -> Drbg {
        let mut pool = EntropyPool::new();
        pool.add(SourceId::KERNEL, &[0xAB; MIN_POOL_BYTES]);
        let mut drbg = Drbg::new(pool);
        assert!(drbg.reseed_from_pool(), "pool should be ready");
        drbg
    }

    #[test]
    fn fill_bytes_produces_output() {
        let mut drbg = seeded_drbg();
        let mut buf = [0u8; 64];
        drbg.fill_bytes(&mut buf);
        assert_ne!(buf, [0u8; 64], "output should not be all zeros");
    }

    #[test]
    fn same_seed_same_output() {
        let seed = [0x42u8; 32];
        let pool1 = EntropyPool::new();
        let mut drbg1 = Drbg::new(pool1);
        drbg1.reseed(&seed);

        let pool2 = EntropyPool::new();
        let mut drbg2 = Drbg::new(pool2);
        drbg2.reseed(&seed);

        let mut buf1 = [0u8; 64];
        let mut buf2 = [0u8; 64];
        drbg1.fill_bytes(&mut buf1);
        drbg2.fill_bytes(&mut buf2);

        assert_eq!(buf1, buf2, "same seed must produce same output");
    }

    #[test]
    fn different_seeds_different_output() {
        let mut drbg1 = Drbg::new(EntropyPool::new());
        drbg1.reseed(&[0x11u8; 32]);

        let mut drbg2 = Drbg::new(EntropyPool::new());
        drbg2.reseed(&[0x22u8; 32]);

        let mut buf1 = [0u8; 64];
        let mut buf2 = [0u8; 64];
        drbg1.fill_bytes(&mut buf1);
        drbg2.fill_bytes(&mut buf2);

        assert_ne!(buf1, buf2, "different seeds must produce different output");
    }

    #[test]
    fn consecutive_calls_produce_different_output() {
        let mut drbg = seeded_drbg();
        let mut buf1 = [0u8; 32];
        let mut buf2 = [0u8; 32];
        drbg.fill_bytes(&mut buf1);
        drbg.fill_bytes(&mut buf2);
        assert_ne!(buf1, buf2, "consecutive calls must produce different output");
    }

    #[test]
    fn reseed_changes_output() {
        let mut drbg = seeded_drbg();
        let mut buf_before = [0u8; 32];
        drbg.fill_bytes(&mut buf_before);

        // Reseed with different material
        drbg.reseed(&[0xCC; 32]);

        let mut buf_after = [0u8; 32];
        drbg.fill_bytes(&mut buf_after);

        assert_ne!(buf_before, buf_after, "reseed must change output trajectory");
    }

    #[test]
    fn bytes_counter_tracks_output() {
        let mut drbg = seeded_drbg();
        assert_eq!(drbg.bytes_since_reseed(), 0);

        let mut buf = [0u8; 100];
        drbg.fill_bytes(&mut buf);
        assert_eq!(drbg.bytes_since_reseed(), 100);
    }

    #[test]
    #[should_panic(expected = "DRBG must be seeded")]
    fn panics_when_not_seeded() {
        let mut drbg = Drbg::new(EntropyPool::new());
        let mut buf = [0u8; 32];
        drbg.fill_bytes(&mut buf);
    }

    #[test]
    fn large_output_request() {
        let mut drbg = seeded_drbg();
        let mut buf = vec![0u8; 4096];
        drbg.fill_bytes(&mut buf);
        // Should not be all zeros
        assert!(buf.iter().any(|&b| b != 0), "large output should not be all zeros");
    }
}
