//! Styrene wire protocol encode/decode.
//!
//! This module must produce byte-identical output to Python's
//! `styrened/src/styrened/models/styrene_wire.py`.

use rand_core::OsRng;
use rand_core::RngCore;
use serde::{Deserialize, Serialize};

use crate::{NAMESPACE, WIRE_VERSION};

/// Wire protocol header size: 10 (namespace) + 1 (version) + 1 (type) + 16 (request_id) = 28
const HEADER_SIZE: usize = 28;

/// Errors from wire protocol operations.
#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("message too short: {0} bytes (minimum {HEADER_SIZE})")]
    TooShort(usize),

    #[error("invalid namespace (expected 'styrene.io')")]
    InvalidNamespace,

    #[error("unsupported wire version: {0}")]
    UnsupportedVersion(u8),

    #[error("unknown message type: 0x{0:02x}")]
    UnknownMessageType(u8),

    #[error("msgpack decode error: {0}")]
    MsgpackDecode(#[from] rmp_serde::decode::Error),

    #[error("msgpack encode error: {0}")]
    MsgpackEncode(#[from] rmp_serde::encode::Error),
}

/// Styrene wire protocol message types.
///
/// Ranges match Python's `StyreneMessageType` enum:
/// - `0x01-0x0F`: Control
/// - `0x10-0x1F`: Status
/// - `0x40-0x5F`: RPC Commands
/// - `0x60-0x7F`: RPC Responses
/// - `0xC0-0xCF`: Terminal Sessions
/// - `0xD0-0xD7`: PQC Sessions (post-quantum cryptography)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum StyreneMessageType {
    // Control (0x01-0x0F)
    Ping = 0x01,
    Pong = 0x02,
    Heartbeat = 0x03,

    // Status (0x10-0x1F)
    StatusRequest = 0x10,
    StatusResponse = 0x11,

    // RPC Commands (0x40-0x5F)
    Exec = 0x40,
    Reboot = 0x41,
    ConfigUpdate = 0x42,

    // RPC Responses (0x60-0x7F)
    ExecResult = 0x60,
    RebootResult = 0x61,
    ConfigUpdateResult = 0x62,

    // Terminal Sessions (0xC0-0xCF)
    TerminalRequest = 0xC0,
    TerminalAccept = 0xC1,
    TerminalData = 0xC2,
    TerminalResize = 0xC3,
    TerminalClose = 0xC4,

    // PQC Sessions (0xD0-0xD7) — post-quantum tunnel establishment
    #[cfg(feature = "pqc")]
    PqcInitiate = 0xD0,
    #[cfg(feature = "pqc")]
    PqcRespond = 0xD1,
    #[cfg(feature = "pqc")]
    PqcConfirm = 0xD2,
    #[cfg(feature = "pqc")]
    PqcRekey = 0xD3,
    #[cfg(feature = "pqc")]
    PqcData = 0xD4,
    #[cfg(feature = "pqc")]
    PqcClose = 0xD5,
    #[cfg(feature = "pqc")]
    PqcCapability = 0xD6,
    #[cfg(feature = "pqc")]
    PqcCapabilityAck = 0xD7,

    // Content Distribution (0xE0-0xE3)
    /// A node announces availability of content chunks.
    ResourceAvailable   = 0xE0,
    /// Request a specific chunk from a seeder.
    ChunkRequest        = 0xE1,
    /// Response carrying a chunk's raw bytes.
    ChunkResponse       = 0xE2,
}

impl StyreneMessageType {
    /// Convert from raw byte value.
    pub fn from_byte(b: u8) -> Result<Self, WireError> {
        match b {
            0x01 => Ok(Self::Ping),
            0x02 => Ok(Self::Pong),
            0x03 => Ok(Self::Heartbeat),
            0x10 => Ok(Self::StatusRequest),
            0x11 => Ok(Self::StatusResponse),
            0x40 => Ok(Self::Exec),
            0x41 => Ok(Self::Reboot),
            0x42 => Ok(Self::ConfigUpdate),
            0x60 => Ok(Self::ExecResult),
            0x61 => Ok(Self::RebootResult),
            0x62 => Ok(Self::ConfigUpdateResult),
            0xC0 => Ok(Self::TerminalRequest),
            0xC1 => Ok(Self::TerminalAccept),
            0xC2 => Ok(Self::TerminalData),
            0xC3 => Ok(Self::TerminalResize),
            0xC4 => Ok(Self::TerminalClose),
            #[cfg(feature = "pqc")]
            0xD0 => Ok(Self::PqcInitiate),
            #[cfg(feature = "pqc")]
            0xD1 => Ok(Self::PqcRespond),
            #[cfg(feature = "pqc")]
            0xD2 => Ok(Self::PqcConfirm),
            #[cfg(feature = "pqc")]
            0xD3 => Ok(Self::PqcRekey),
            #[cfg(feature = "pqc")]
            0xD4 => Ok(Self::PqcData),
            #[cfg(feature = "pqc")]
            0xD5 => Ok(Self::PqcClose),
            #[cfg(feature = "pqc")]
            0xD6 => Ok(Self::PqcCapability),
            #[cfg(feature = "pqc")]
            0xD7 => Ok(Self::PqcCapabilityAck),
            0xE0 => Ok(Self::ResourceAvailable),
            0xE1 => Ok(Self::ChunkRequest),
            0xE2 => Ok(Self::ChunkResponse),
            _ => Err(WireError::UnknownMessageType(b)),
        }
    }
}

// ── Content Distribution Payloads (0xE0-0xE2) ────────────────────────────────

/// Payload for `ResourceAvailable` (0xE0): announces held chunks for a content item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceAvailablePayload {
    /// Blake3 content hash (32 bytes = ContentId).
    pub content_id: [u8; 32],
    /// First 16 bytes of Blake3(manifest_bytes) — quick integrity check.
    pub manifest_hash: [u8; 16],
    /// 256-bit bitset of chunks currently held (32 bytes = ChunkBitset).
    #[serde(with = "serde_bytes")]
    pub chunks_held: Vec<u8>,
    /// Announcing node's RNS identity_hash (16 bytes).
    pub seeder_hash: [u8; 16],
}

impl ResourceAvailablePayload {
    /// Encode to msgpack bytes for embedding in a `StyreneMessage` payload.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        Ok(rmp_serde::to_vec(self)?)
    }

    /// Decode from msgpack bytes.
    pub fn decode(data: &[u8]) -> Result<Self, WireError> {
        Ok(rmp_serde::from_slice(data)?)
    }
}

/// Payload for `ChunkRequest` (0xE1): asks a seeder for a specific chunk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkRequestPayload {
    /// Content being requested.
    pub content_id: [u8; 32],
    /// Zero-based chunk index.
    pub chunk_index: u32,
}

impl ChunkRequestPayload {
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        Ok(rmp_serde::to_vec(self)?)
    }
    pub fn decode(data: &[u8]) -> Result<Self, WireError> {
        Ok(rmp_serde::from_slice(data)?)
    }
}

/// Payload for `ChunkResponse` (0xE2): carries raw chunk bytes from a seeder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkResponsePayload {
    /// Content this chunk belongs to.
    pub content_id: [u8; 32],
    /// Zero-based chunk index.
    pub chunk_index: u32,
    /// Raw chunk bytes (verified against manifest before use).
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>,
}

impl ChunkResponsePayload {
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        Ok(rmp_serde::to_vec(self)?)
    }
    pub fn decode(data: &[u8]) -> Result<Self, WireError> {
        Ok(rmp_serde::from_slice(data)?)
    }
}

// ─────────────────────────────────────────────────────────────────────────────

/// A Styrene wire protocol message.
#[derive(Debug, Clone)]
pub struct StyreneMessage {
    /// Wire format version (currently always 0x01).
    pub version: u8,
    /// Message type.
    pub message_type: StyreneMessageType,
    /// 16-byte random request ID for correlation.
    pub request_id: [u8; 16],
    /// Raw payload bytes (msgpack-encoded by caller).
    pub payload: Vec<u8>,
}

impl StyreneMessage {
    /// Create a new message with a random request ID.
    pub fn new(message_type: StyreneMessageType, payload: &[u8]) -> Self {
        let mut request_id = [0u8; 16];
        OsRng.fill_bytes(&mut request_id);
        Self { version: WIRE_VERSION, message_type, request_id, payload: payload.to_vec() }
    }

    /// Create a new message with a specific request ID (for responses).
    pub fn with_request_id(
        message_type: StyreneMessageType,
        request_id: [u8; 16],
        payload: &[u8],
    ) -> Self {
        Self { version: WIRE_VERSION, message_type, request_id, payload: payload.to_vec() }
    }

    /// Encode to wire format bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(HEADER_SIZE + self.payload.len());
        buf.extend_from_slice(NAMESPACE);
        buf.push(self.version);
        buf.push(self.message_type as u8);
        buf.extend_from_slice(&self.request_id);
        buf.extend_from_slice(&self.payload);
        buf
    }

    /// Decode from wire format bytes.
    pub fn decode(data: &[u8]) -> Result<Self, WireError> {
        if data.len() < HEADER_SIZE {
            return Err(WireError::TooShort(data.len()));
        }

        // Validate namespace
        if &data[..10] != NAMESPACE.as_slice() {
            return Err(WireError::InvalidNamespace);
        }

        // Validate version
        let version = data[10];
        if version != WIRE_VERSION {
            return Err(WireError::UnsupportedVersion(version));
        }

        // Parse message type
        let message_type = StyreneMessageType::from_byte(data[11])?;

        // Extract request ID
        let mut request_id = [0u8; 16];
        request_id.copy_from_slice(&data[12..28]);

        // Remaining bytes are payload
        let payload = data[28..].to_vec();

        Ok(Self { version, message_type, request_id, payload })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_ping() {
        let msg = StyreneMessage::new(StyreneMessageType::Ping, &[]);
        let encoded = msg.encode();
        let decoded = StyreneMessage::decode(&encoded).expect("decode failed");
        assert_eq!(decoded.version, WIRE_VERSION);
        assert_eq!(decoded.message_type, StyreneMessageType::Ping);
        assert_eq!(decoded.request_id, msg.request_id);
        assert!(decoded.payload.is_empty());
    }

    #[test]
    fn roundtrip_with_payload() {
        let payload = rmp_serde::to_vec(&serde_json::json!({"hostname": "styrene-node"}))
            .expect("encode payload");
        let msg = StyreneMessage::new(StyreneMessageType::StatusResponse, &payload);
        let encoded = msg.encode();
        let decoded = StyreneMessage::decode(&encoded).expect("decode failed");
        assert_eq!(decoded.message_type, StyreneMessageType::StatusResponse);
        assert_eq!(decoded.payload, payload);
    }

    #[test]
    fn response_preserves_request_id() {
        let request = StyreneMessage::new(StyreneMessageType::Ping, &[]);
        let response =
            StyreneMessage::with_request_id(StyreneMessageType::Pong, request.request_id, &[]);
        assert_eq!(request.request_id, response.request_id);
    }

    #[test]
    fn rejects_short_message() {
        assert!(StyreneMessage::decode(&[0; 10]).is_err());
    }

    #[test]
    fn rejects_wrong_namespace() {
        let mut data = vec![0u8; 28];
        data[..10].copy_from_slice(b"not.styren");
        assert!(matches!(StyreneMessage::decode(&data), Err(WireError::InvalidNamespace)));
    }

    #[test]
    fn rejects_unknown_version() {
        let mut data = vec![0u8; 28];
        data[..10].copy_from_slice(b"styrene.io");
        data[10] = 0xFF;
        assert!(matches!(StyreneMessage::decode(&data), Err(WireError::UnsupportedVersion(0xFF))));
    }

    #[test]
    fn header_size_is_28() {
        let msg = StyreneMessage::new(StyreneMessageType::Ping, &[]);
        let encoded = msg.encode();
        assert_eq!(encoded.len(), 28); // no payload
    }

    #[test]
    fn resource_available_payload_roundtrip() {
        let p = ResourceAvailablePayload {
            content_id: [0xABu8; 32],
            manifest_hash: [0x12u8; 16],
            chunks_held: vec![0xFFu8; 32],
            seeder_hash: [0x99u8; 16],
        };
        let encoded = p.encode().unwrap();
        let decoded = ResourceAvailablePayload::decode(&encoded).unwrap();
        assert_eq!(decoded.content_id, p.content_id);
        assert_eq!(decoded.manifest_hash, p.manifest_hash);
        assert_eq!(decoded.chunks_held, p.chunks_held);
        assert_eq!(decoded.seeder_hash, p.seeder_hash);
    }

    #[test]
    fn chunk_request_payload_roundtrip() {
        let p = ChunkRequestPayload { content_id: [0x01u8; 32], chunk_index: 42 };
        let decoded = ChunkRequestPayload::decode(&p.encode().unwrap()).unwrap();
        assert_eq!(decoded.content_id, p.content_id);
        assert_eq!(decoded.chunk_index, 42);
    }

    #[test]
    fn chunk_response_payload_roundtrip() {
        let p = ChunkResponsePayload {
            content_id: [0x02u8; 32],
            chunk_index: 7,
            data: b"hello world chunk data".to_vec(),
        };
        let decoded = ChunkResponsePayload::decode(&p.encode().unwrap()).unwrap();
        assert_eq!(decoded.chunk_index, 7);
        assert_eq!(decoded.data, p.data);
    }

    #[test]
    fn content_message_types_roundtrip() {
        for msg_type in [
            StyreneMessageType::ResourceAvailable,
            StyreneMessageType::ChunkRequest,
            StyreneMessageType::ChunkResponse,
        ] {
            let msg = StyreneMessage::new(msg_type, &[]);
            let decoded = StyreneMessage::decode(&msg.encode()).expect("roundtrip failed");
            assert_eq!(decoded.message_type, msg_type);
        }
    }

    #[test]
    fn all_message_types_roundtrip() {
        let types = [
            StyreneMessageType::Ping,
            StyreneMessageType::Pong,
            StyreneMessageType::Heartbeat,
            StyreneMessageType::StatusRequest,
            StyreneMessageType::StatusResponse,
            StyreneMessageType::Exec,
            StyreneMessageType::Reboot,
            StyreneMessageType::ConfigUpdate,
            StyreneMessageType::ExecResult,
            StyreneMessageType::RebootResult,
            StyreneMessageType::ConfigUpdateResult,
            StyreneMessageType::TerminalRequest,
            StyreneMessageType::TerminalAccept,
            StyreneMessageType::TerminalData,
            StyreneMessageType::TerminalResize,
            StyreneMessageType::TerminalClose,
            #[cfg(feature = "pqc")]
            StyreneMessageType::PqcInitiate,
            #[cfg(feature = "pqc")]
            StyreneMessageType::PqcRespond,
            #[cfg(feature = "pqc")]
            StyreneMessageType::PqcConfirm,
            #[cfg(feature = "pqc")]
            StyreneMessageType::PqcRekey,
            #[cfg(feature = "pqc")]
            StyreneMessageType::PqcData,
            #[cfg(feature = "pqc")]
            StyreneMessageType::PqcClose,
            #[cfg(feature = "pqc")]
            StyreneMessageType::PqcCapability,
            #[cfg(feature = "pqc")]
            StyreneMessageType::PqcCapabilityAck,
        ];
        for msg_type in types {
            let msg = StyreneMessage::new(msg_type, &[]);
            let decoded = StyreneMessage::decode(&msg.encode()).expect("roundtrip failed");
            assert_eq!(decoded.message_type, msg_type);
        }
    }
}
