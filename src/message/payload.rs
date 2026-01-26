use serde::{Deserialize, Serialize};
use serde_bytes::ByteBuf;

use crate::error::LxmfError;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Payload {
    pub timestamp: f64,
    pub content: Option<ByteBuf>,
    pub title: Option<ByteBuf>,
    pub fields: Option<rmpv::Value>,
}

impl Payload {
    pub fn new(
        timestamp: f64,
        content: Option<Vec<u8>>,
        title: Option<Vec<u8>>,
        fields: Option<rmpv::Value>,
    ) -> Self {
        Self {
            timestamp,
            content: content.map(ByteBuf::from),
            title: title.map(ByteBuf::from),
            fields,
        }
    }

    pub fn to_msgpack(&self) -> Result<Vec<u8>, LxmfError> {
        let list = (
            self.timestamp,
            self.content.clone(),
            self.title.clone(),
            self.fields.clone(),
        );
        rmp_serde::to_vec(&list).map_err(|e| LxmfError::Encode(e.to_string()))
    }

    pub fn from_msgpack(bytes: &[u8]) -> Result<Self, LxmfError> {
        let (timestamp, content, title, fields): (
            f64,
            Option<ByteBuf>,
            Option<ByteBuf>,
            Option<rmpv::Value>,
        ) = rmp_serde::from_slice(bytes)
            .map_err(|e| LxmfError::Decode(e.to_string()))?;
        Ok(Self {
            timestamp,
            content,
            title,
            fields,
        })
    }
}
