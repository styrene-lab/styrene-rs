use serde::{Deserialize, Serialize};
use serde_bytes::ByteBuf;

use crate::error::LxmfError;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageContainer {
    pub state: u8,
    pub lxmf_bytes: ByteBuf,
    pub transport_encrypted: bool,
    pub transport_encryption: Option<String>,
    pub method: u8,
}

impl MessageContainer {
    pub fn from_msgpack(bytes: &[u8]) -> Result<Self, LxmfError> {
        rmp_serde::from_slice(bytes).map_err(|e| LxmfError::Decode(e.to_string()))
    }

    pub fn to_msgpack(&self) -> Result<Vec<u8>, LxmfError> {
        let mut out = Vec::new();
        let mut serializer = rmp_serde::Serializer::new(&mut out).with_struct_map();
        self.serialize(&mut serializer)
            .map_err(|e| LxmfError::Encode(e.to_string()))?;
        Ok(out)
    }
}
