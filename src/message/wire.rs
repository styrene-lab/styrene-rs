use crate::message::Payload;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct WireMessage {
    pub destination: [u8; 16],
    pub source: [u8; 16],
    pub payload: Payload,
}

impl WireMessage {
    pub fn new(destination: [u8; 16], source: [u8; 16], payload: Payload) -> Self {
        Self {
            destination,
            source,
            payload,
        }
    }

    pub fn message_id(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(self.destination);
        hasher.update(self.source);
        hasher.update(self.payload.to_msgpack().unwrap_or_default());
        let bytes = hasher.finalize();
        let mut out = [0u8; 32];
        out.copy_from_slice(&bytes);
        out
    }
}
