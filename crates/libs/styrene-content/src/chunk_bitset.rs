/// A fixed 256-bit bitset tracking which chunks a node holds.
///
/// `[u8; 32]` = 256 bits = one bit per chunk slot. `Copy`, no alloc.
///
/// # Limits
///
/// Supports up to 256 chunk indices (0–255). This covers:
/// - LoRa profile: 256 × 4 KB = 1 MB max
/// - WiFi profile: 256 × 256 KB = 64 MB max
///
/// Manifests declaring `chunk_count > 256` are valid but nodes can only
/// track the first 256 chunks in their bitset; partial seeding beyond
/// index 255 is not announced.
#[derive(Debug, Copy, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ChunkBitset(pub [u8; 32]);

impl ChunkBitset {
    pub const fn new() -> Self {
        Self([0u8; 32])
    }

    /// Set bit at `index` (0-based). Panics if `index >= 256`.
    pub fn set(&mut self, index: u32) {
        assert!(index < 256, "chunk index out of range");
        self.0[(index / 8) as usize] |= 1 << (index % 8);
    }

    /// Clear bit at `index`.
    pub fn clear(&mut self, index: u32) {
        if index < 256 {
            self.0[(index / 8) as usize] &= !(1 << (index % 8));
        }
    }

    /// Test bit at `index`. Returns false for any index >= 256.
    pub fn get(&self, index: u32) -> bool {
        if index >= 256 {
            return false;
        }
        self.0[(index / 8) as usize] & (1 << (index % 8)) != 0
    }

    /// Count of set bits (popcount).
    pub fn count(&self) -> u32 {
        self.0.iter().map(|b| b.count_ones()).sum()
    }

    /// Returns true if the first `total_chunks` bits are all set.
    pub fn is_complete(&self, total_chunks: u32) -> bool {
        if total_chunks == 0 {
            return true;
        }
        let chunks = total_chunks.min(256) as usize;
        let full_bytes = chunks / 8;
        let remainder = chunks % 8;

        // Check full bytes
        if self.0[..full_bytes].iter().any(|&b| b != 0xFF) {
            return false;
        }
        // Check partial last byte
        if remainder > 0 {
            let mask = (1u8 << remainder) - 1;
            if self.0[full_bytes] & mask != mask {
                return false;
            }
        }
        true
    }

    pub const fn empty() -> Self {
        Self::new()
    }
}

impl Default for ChunkBitset {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_and_get() {
        let mut bs = ChunkBitset::new();
        assert!(!bs.get(0));
        bs.set(0);
        assert!(bs.get(0));
        assert!(!bs.get(1));
    }

    #[test]
    fn boundaries() {
        let mut bs = ChunkBitset::new();
        bs.set(0);
        bs.set(255);
        assert!(bs.get(0));
        assert!(bs.get(255));
        assert!(!bs.get(127));
        // Out of range returns false
        assert!(!bs.get(256));
    }

    #[test]
    fn popcount() {
        let mut bs = ChunkBitset::new();
        assert_eq!(bs.count(), 0);
        bs.set(0);
        bs.set(7);
        bs.set(8);
        assert_eq!(bs.count(), 3);
    }

    #[test]
    fn is_complete() {
        let mut bs = ChunkBitset::new();
        assert!(!bs.is_complete(3));
        bs.set(0);
        bs.set(1);
        bs.set(2);
        assert!(bs.is_complete(3));
        assert!(!bs.is_complete(4));
    }

    #[test]
    fn is_complete_byte_boundary() {
        let mut bs = ChunkBitset::new();
        for i in 0..8 {
            bs.set(i);
        }
        assert!(bs.is_complete(8));
        assert!(!bs.is_complete(9));
    }

    #[test]
    fn clear() {
        let mut bs = ChunkBitset::new();
        bs.set(5);
        assert!(bs.get(5));
        bs.clear(5);
        assert!(!bs.get(5));
    }
}
