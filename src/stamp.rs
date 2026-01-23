use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    value: [u8; 32],
}

impl Stamp {
    pub fn generate(data: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(data);
        let bytes = hasher.finalize();
        let mut value = [0u8; 32];
        value.copy_from_slice(&bytes);
        Self { value }
    }

    pub fn verify(&self, data: &[u8]) -> bool {
        Self::generate(data).value == self.value
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.value
    }
}
