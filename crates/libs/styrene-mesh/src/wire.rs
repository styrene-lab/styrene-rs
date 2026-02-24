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
            _ => Err(WireError::UnknownMessageType(b)),
        }
    }
}

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
        Self {
            version: WIRE_VERSION,
            message_type,
            request_id,
            payload: payload.to_vec(),
        }
    }

    /// Create a new message with a specific request ID (for responses).
    pub fn with_request_id(
        message_type: StyreneMessageType,
        request_id: [u8; 16],
        payload: &[u8],
    ) -> Self {
        Self {
            version: WIRE_VERSION,
            message_type,
            request_id,
            payload: payload.to_vec(),
        }
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

        Ok(Self {
            version,
            message_type,
            request_id,
            payload,
        })
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
        let response = StyreneMessage::with_request_id(
            StyreneMessageType::Pong,
            request.request_id,
            &[],
        );
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
        assert!(matches!(
            StyreneMessage::decode(&data),
            Err(WireError::InvalidNamespace)
        ));
    }

    #[test]
    fn rejects_unknown_version() {
        let mut data = vec![0u8; 28];
        data[..10].copy_from_slice(b"styrene.io");
        data[10] = 0xFF;
        assert!(matches!(
            StyreneMessage::decode(&data),
            Err(WireError::UnsupportedVersion(0xFF))
        ));
    }

    #[test]
    fn header_size_is_28() {
        let msg = StyreneMessage::new(StyreneMessageType::Ping, &[]);
        let encoded = msg.encode();
        assert_eq!(encoded.len(), 28); // no payload
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
        ];
        for msg_type in types {
            let msg = StyreneMessage::new(msg_type, &[]);
            let decoded = StyreneMessage::decode(&msg.encode()).expect("roundtrip failed");
            assert_eq!(decoded.message_type, msg_type);
        }
    }
}
