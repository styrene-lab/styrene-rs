use std::fmt;
use std::str::FromStr;

use minicbor::{Decode, Decoder, Encode, Encoder};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// Canonical 16-byte identifier for a Styrene Identity authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IdentityId([u8; 16]);

impl IdentityId {
    /// Derive an identifier from an Identity Ed25519 public key.
    pub fn from_public_key(public_key: &[u8; 32]) -> Self {
        let digest = Sha256::digest(public_key);
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&digest[..16]);
        Self(bytes)
    }

    /// Construct an identifier from its exact binary representation.
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Return the exact binary representation.
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    /// Check the identifier-to-public-key binding in constant time.
    pub fn matches_public_key(&self, public_key: &[u8; 32]) -> bool {
        self.0.ct_eq(&Self::from_public_key(public_key).0).into()
    }
}

impl fmt::Display for IdentityId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", hex::encode(self.0))
    }
}

impl FromStr for IdentityId {
    type Err = IdentityIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != 32
            || !value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(IdentityIdError);
        }
        let decoded = hex::decode(value).map_err(|_| IdentityIdError)?;
        let bytes = decoded.try_into().map_err(|_| IdentityIdError)?;
        Ok(Self(bytes))
    }
}

impl<Context> Encode<Context> for IdentityId {
    fn encode<Writer: minicbor::encode::Write>(
        &self,
        encoder: &mut Encoder<Writer>,
        _context: &mut Context,
    ) -> Result<(), minicbor::encode::Error<Writer::Error>> {
        encoder.bytes(&self.0)?;
        Ok(())
    }
}

impl<'bytes, Context> Decode<'bytes, Context> for IdentityId {
    fn decode(
        decoder: &mut Decoder<'bytes>,
        _context: &mut Context,
    ) -> Result<Self, minicbor::decode::Error> {
        let bytes = decoder.bytes()?;
        let bytes = bytes
            .try_into()
            .map_err(|_| minicbor::decode::Error::message("Identity ID must be 16 bytes"))?;
        Ok(Self(bytes))
    }
}

/// Canonical Identity ID text was malformed or non-canonical.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("Identity ID must be exactly 32 lowercase hexadecimal characters")]
pub struct IdentityIdError;
