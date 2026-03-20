/// Network profile governing chunk size selection.
///
/// The publisher selects a profile at manifest creation time based on the
/// expected delivery network. Seeders and leeches use whatever size the
/// manifest specifies — there is no per-transfer negotiation.
///
/// # Memory requirements
///
/// Leeching nodes must be able to buffer a full chunk in memory for
/// Blake3 verification. Selecting a profile incompatible with a node's
/// available RAM is a documented limitation — that node cannot leech.
///
/// | Profile    | Chunk size | Max file (256 chunks) | Target |
/// |------------|------------|----------------------|--------|
/// | `LoRa`     | 4 KB       | 1 MB                 | RP2040, strict LoRa paths |
/// | `Balanced` | 32 KB      | 8 MB                 | Mixed, most ESP32 |
/// | `WiFi`     | 256 KB     | 64 MB                | Hub nodes, ESP32+PSRAM |
#[derive(Debug, Copy, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum ChunkProfile {
    LoRa     = 0,
    Balanced = 1,
    WiFi     = 2,
}

impl ChunkProfile {
    pub const fn chunk_size(self) -> u32 {
        match self {
            Self::LoRa     => 4   * 1024,
            Self::Balanced => 32  * 1024,
            Self::WiFi     => 256 * 1024,
        }
    }

    /// Maximum file size with 256 chunks at this profile's chunk size.
    pub const fn max_file_size(self) -> u64 {
        self.chunk_size() as u64 * 256
    }

    /// Number of chunks required to cover `file_size` bytes.
    pub const fn chunk_count_for(self, file_size: u64) -> u32 {
        let cs = self.chunk_size() as u64;
        ((file_size + cs - 1) / cs) as u32
    }

    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::LoRa),
            1 => Some(Self::Balanced),
            2 => Some(Self::WiFi),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_sizes() {
        assert_eq!(ChunkProfile::LoRa.chunk_size(), 4096);
        assert_eq!(ChunkProfile::Balanced.chunk_size(), 32768);
        assert_eq!(ChunkProfile::WiFi.chunk_size(), 262144);
    }

    #[test]
    fn chunk_count() {
        // Exact multiple
        assert_eq!(ChunkProfile::LoRa.chunk_count_for(4096), 1);
        // Not exact — rounds up
        assert_eq!(ChunkProfile::LoRa.chunk_count_for(4097), 2);
        // Zero
        assert_eq!(ChunkProfile::LoRa.chunk_count_for(0), 0);
    }

    #[test]
    fn max_file_sizes() {
        assert_eq!(ChunkProfile::LoRa.max_file_size(), 1024 * 1024);
        assert_eq!(ChunkProfile::WiFi.max_file_size(), 64 * 1024 * 1024);
    }
}
