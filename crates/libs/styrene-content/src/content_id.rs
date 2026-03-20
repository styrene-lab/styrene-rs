use core::fmt;

/// Content identifier — Blake3 hash of the full content.
///
/// Stable regardless of chunk size or profile. Two nodes with the same
/// content will always produce the same `ContentId`.
#[derive(Copy, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ContentId(pub [u8; 32]);

impl ContentId {
    /// Hash arbitrary byte content.
    pub fn from_bytes(data: &[u8]) -> Self {
        let hash = blake3::hash(data);
        Self(*hash.as_bytes())
    }

    /// Wrap a raw 32-byte hash directly (e.g. from a stored manifest).
    pub const fn from_raw(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for ContentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ContentId(")?;
        for b in &self.0 {
            write!(f, "{b:02x}")?;
        }
        write!(f, ")")
    }
}

impl fmt::LowerHex for ContentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in &self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        let a = ContentId::from_bytes(b"hello mesh");
        let b = ContentId::from_bytes(b"hello mesh");
        assert_eq!(a, b);
    }

    #[test]
    fn different_content_different_id() {
        let a = ContentId::from_bytes(b"firmware v1");
        let b = ContentId::from_bytes(b"firmware v2");
        assert_ne!(a, b);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn hex_display() {
        let id = ContentId::from_raw([0xde, 0xad, 0xbe, 0xef,
                                      0x00, 0x00, 0x00, 0x00,
                                      0x00, 0x00, 0x00, 0x00,
                                      0x00, 0x00, 0x00, 0x00,
                                      0x00, 0x00, 0x00, 0x00,
                                      0x00, 0x00, 0x00, 0x00,
                                      0x00, 0x00, 0x00, 0x00,
                                      0x00, 0x00, 0x00, 0x00]);
        let s = alloc::format!("{id:x}");
        assert!(s.starts_with("deadbeef"));
    }
}
