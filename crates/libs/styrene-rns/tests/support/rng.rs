//! Synthetic providers for tests only. These are not production entropy sources.

#![allow(dead_code)]

use rand_core::{CryptoRng, RngCore};
use sha2::{Digest, Sha256};

/// Deliberately neither Copy nor Clone; records every request and its output.
#[derive(Default)]
pub struct RecordingRng {
    counter: u64,
    pub requests: Vec<Vec<u8>>,
}

impl RngCore for RecordingRng {
    fn next_u32(&mut self) -> u32 {
        let mut bytes = [0; 4];
        self.fill_bytes(&mut bytes);
        u32::from_le_bytes(bytes)
    }

    fn next_u64(&mut self) -> u64 {
        let mut bytes = [0; 8];
        self.fill_bytes(&mut bytes);
        u64::from_le_bytes(bytes)
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        for chunk in dest.chunks_mut(32) {
            let block = Sha256::digest(self.counter.to_be_bytes());
            self.counter += 1;
            chunk.copy_from_slice(&block[..chunk.len()]);
        }
        self.requests.push(dest.to_vec());
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}

impl CryptoRng for RecordingRng {}

impl RecordingRng {
    pub fn lengths(&self) -> Vec<usize> {
        self.requests.iter().map(Vec::len).collect()
    }
}
