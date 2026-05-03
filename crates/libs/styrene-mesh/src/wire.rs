//! Styrene wire protocol encode/decode.
//!
//! Payload encoding uses CBOR (RFC 8949) via `ciborium` for deterministic
//! encoding, COSE compatibility, and IETF governance.

use rand_core::OsRng;
use rand_core::RngCore;
use serde::{Deserialize, Serialize};

use crate::{NAMESPACE, WIRE_VERSION};

/// Wire protocol header size: 11 (namespace) + 1 (version) + 1 (type) + 16 (request_id)
const HEADER_SIZE: usize = 29;

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

    #[error("CBOR decode error: {0}")]
    CborDecode(String),

    #[error("CBOR encode error: {0}")]
    CborEncode(String),
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
    CapabilitiesRequest = 0x12,
    CapabilitiesResponse = 0x13,

    // Content (0x20-0x2F)
    Chat = 0x20,
    ChatAck = 0x21,
    FileOffer = 0x22,
    FileAccept = 0x23,
    FileChunk = 0x24,

    // Network (0x30-0x3F)
    Announce = 0x30,
    AnnounceAck = 0x31,
    PeerRequest = 0x32,
    PeerResponse = 0x33,
    VpnHandshakeRequest = 0x34,
    VpnHandshakeResponse = 0x35,

    // RPC Commands (0x40-0x5F)
    Exec = 0x40,
    Reboot = 0x41,
    ConfigUpdate = 0x42,
    SelfUpdate = 0x43,
    InboxQuery = 0x44,
    MessagesQuery = 0x45,
    /// Client sends LXMF payload to hub for offline destination storage.
    PropagationIngest = 0x46,
    /// Client requests queued messages from hub.
    PropagationFetch = 0x47,
    /// Client acknowledges receipt of messages (hub deletes them).
    PropagationDelete = 0x48,
    /// Client requests a Micron page from a remote node.
    PageRequest = 0x49,

    // RPC Responses (0x60-0x7F)
    ExecResult = 0x60,
    RebootResult = 0x61,
    ConfigUpdateResult = 0x62,
    SelfUpdateResult = 0x63,
    InboxResponse = 0x64,
    MessagesResponse = 0x65,
    PropagationIngestResult = 0x66,
    PropagationFetchResult = 0x67,
    PropagationDeleteResult = 0x68,
    PageResponse = 0x69,

    // Hub Services — I2P Proxy (0x84-0x88)
    /// Client requests HTTP fetch through hub's i2pd router.
    I2pProxyRequest = 0x84,
    /// Hub returns HTTP response headers and metadata.
    I2pProxyResponse = 0x85,
    /// Hub sends a chunk of the response body.
    I2pProxyData = 0x86,
    /// Hub reports an error for a proxy request.
    I2pProxyError = 0x87,
    /// Either side aborts an in-flight proxy request.
    I2pProxyClose = 0x88,

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

    // Tunnel Negotiation (0xD8-0xDF) — WireGuard/VPN tunnel lifecycle
    /// Initiator proposes a tunnel to the responder.
    /// Payload: {wg_pubkey, endpoint, mesh_ip, psk, mtu, nonce, timestamp}
    TunnelOffer = 0xD8,
    /// Responder accepts the tunnel offer.
    /// Payload: {wg_pubkey, endpoint, mesh_ip, nonce, timestamp}
    TunnelAccept = 0xD9,
    /// Responder rejects the tunnel offer.
    /// Payload: {reason, nonce}
    TunnelReject = 0xDA,
    /// Either side tears down an established tunnel.
    /// Payload: {tunnel_id, nonce}
    TunnelTeardown = 0xDB,
    /// Either side initiates a rekey with a new PSK.
    /// Payload: {tunnel_id, new_psk, nonce, timestamp}
    TunnelRekey = 0xDC,
    /// Periodic keepalive for the tunnel control channel.
    /// Payload: {tunnel_id, nonce}
    TunnelKeepalive = 0xDD,
    /// Hub broadcasts topology updates to peers.
    /// Payload: {peers: [{identity, endpoint, mesh_ip}], nonce}
    TunnelTopology = 0xDE,

    // Content Distribution (0xE0-0xE3)
    /// A node announces availability of content chunks.
    ResourceAvailable = 0xE0,
    /// Request a specific chunk from a seeder.
    ChunkRequest = 0xE1,
    /// Response carrying a chunk's raw bytes.
    ChunkResponse = 0xE2,

    // Error (0xFF)
    Error = 0xFF,
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
            0x12 => Ok(Self::CapabilitiesRequest),
            0x13 => Ok(Self::CapabilitiesResponse),
            0x20 => Ok(Self::Chat),
            0x21 => Ok(Self::ChatAck),
            0x22 => Ok(Self::FileOffer),
            0x23 => Ok(Self::FileAccept),
            0x24 => Ok(Self::FileChunk),
            0x30 => Ok(Self::Announce),
            0x31 => Ok(Self::AnnounceAck),
            0x32 => Ok(Self::PeerRequest),
            0x33 => Ok(Self::PeerResponse),
            0x34 => Ok(Self::VpnHandshakeRequest),
            0x35 => Ok(Self::VpnHandshakeResponse),
            0x40 => Ok(Self::Exec),
            0x41 => Ok(Self::Reboot),
            0x42 => Ok(Self::ConfigUpdate),
            0x43 => Ok(Self::SelfUpdate),
            0x44 => Ok(Self::InboxQuery),
            0x45 => Ok(Self::MessagesQuery),
            0x46 => Ok(Self::PropagationIngest),
            0x47 => Ok(Self::PropagationFetch),
            0x48 => Ok(Self::PropagationDelete),
            0x49 => Ok(Self::PageRequest),
            0x60 => Ok(Self::ExecResult),
            0x61 => Ok(Self::RebootResult),
            0x62 => Ok(Self::ConfigUpdateResult),
            0x63 => Ok(Self::SelfUpdateResult),
            0x64 => Ok(Self::InboxResponse),
            0x65 => Ok(Self::MessagesResponse),
            0x66 => Ok(Self::PropagationIngestResult),
            0x67 => Ok(Self::PropagationFetchResult),
            0x68 => Ok(Self::PropagationDeleteResult),
            0x69 => Ok(Self::PageResponse),
            0x84 => Ok(Self::I2pProxyRequest),
            0x85 => Ok(Self::I2pProxyResponse),
            0x86 => Ok(Self::I2pProxyData),
            0x87 => Ok(Self::I2pProxyError),
            0x88 => Ok(Self::I2pProxyClose),
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
            0xD8 => Ok(Self::TunnelOffer),
            0xD9 => Ok(Self::TunnelAccept),
            0xDA => Ok(Self::TunnelReject),
            0xDB => Ok(Self::TunnelTeardown),
            0xDC => Ok(Self::TunnelRekey),
            0xDD => Ok(Self::TunnelKeepalive),
            0xDE => Ok(Self::TunnelTopology),
            0xE0 => Ok(Self::ResourceAvailable),
            0xE1 => Ok(Self::ChunkRequest),
            0xE2 => Ok(Self::ChunkResponse),
            0xFF => Ok(Self::Error),
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
    /// Encode to CBOR bytes for embedding in a `StyreneMessage` payload.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf).map_err(|e| WireError::CborEncode(e.to_string()))?;
        Ok(buf)
    }

    /// Decode from CBOR bytes.
    pub fn decode(data: &[u8]) -> Result<Self, WireError> {
        ciborium::from_reader(data).map_err(|e| WireError::CborDecode(e.to_string()))
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
    /// Encode to CBOR bytes for embedding in a `StyreneMessage` payload.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf).map_err(|e| WireError::CborEncode(e.to_string()))?;
        Ok(buf)
    }

    /// Decode from CBOR bytes.
    pub fn decode(data: &[u8]) -> Result<Self, WireError> {
        ciborium::from_reader(data).map_err(|e| WireError::CborDecode(e.to_string()))
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
    /// Encode to CBOR bytes for embedding in a `StyreneMessage` payload.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf).map_err(|e| WireError::CborEncode(e.to_string()))?;
        Ok(buf)
    }

    /// Decode from CBOR bytes.
    pub fn decode(data: &[u8]) -> Result<Self, WireError> {
        ciborium::from_reader(data).map_err(|e| WireError::CborDecode(e.to_string()))
    }
}

// ── Propagation Payloads (0x46-0x48, 0x66-0x68) ────────────────────────────

/// Payload for `PropagationIngest` (0x46): client sends LXMF message to hub for storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropagationIngestPayload {
    /// Hex-encoded delivery destination hash of the offline recipient.
    pub dest_hash: String,
    /// Raw LXMF wire payload (signed, ready for delivery).
    #[serde(with = "serde_bytes")]
    pub lxmf_bytes: Vec<u8>,
    /// Optional hex-encoded source identity hash.
    pub source_hash: Option<String>,
}

/// Payload for `PropagationFetch` (0x47): client requests queued messages from hub.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropagationFetchPayload {
    /// Hex-encoded delivery destination hash to fetch messages for.
    pub dest_hash: String,
}

/// Payload for `PropagationDelete` (0x48): client acknowledges receipt of messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropagationDeletePayload {
    /// Propagation store IDs to delete (from PropagationFetchResult).
    pub ids: Vec<String>,
}

/// A single queued message in a fetch result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropagationMessageEntry {
    /// Propagation store ID (for later deletion via PropagationDelete).
    pub id: String,
    /// Raw LXMF wire payload.
    #[serde(with = "serde_bytes")]
    pub lxmf_bytes: Vec<u8>,
}

/// Payload for `PropagationFetchResult` (0x67): batch of queued messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropagationFetchResultPayload {
    pub messages: Vec<PropagationMessageEntry>,
}

/// Payload for `PropagationIngestResult` (0x66) and `PropagationDeleteResult` (0x68).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropagationStatusPayload {
    pub success: bool,
    pub error: Option<String>,
    pub count: Option<usize>,
}

// ── Page Payloads (0x49, 0x69) ──────────────────────────────────────────────

/// Payload for `PageRequest` (0x49): client requests a Micron page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageRequestPayload {
    /// Request path (e.g. "/", "/status", "/guide/intro").
    pub path: String,
}

/// Payload for `PageResponse` (0x69): node returns page content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageResponsePayload {
    pub success: bool,
    /// Micron source text (empty on error).
    pub source: String,
    /// Error message if success is false.
    pub error: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────

/// A Styrene wire protocol message.
#[derive(Debug, Clone)]
pub struct StyreneMessage {
    /// Wire format version (currently always 0x02).
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

        // Validate namespace (11 bytes: "styrene.io:")
        if &data[..11] != NAMESPACE.as_slice() {
            return Err(WireError::InvalidNamespace);
        }

        // Validate version
        let version = data[11];
        if version != WIRE_VERSION {
            return Err(WireError::UnsupportedVersion(version));
        }

        // Parse message type
        let message_type = StyreneMessageType::from_byte(data[12])?;

        // Extract request ID
        let mut request_id = [0u8; 16];
        request_id.copy_from_slice(&data[13..29]);

        // Remaining bytes are payload
        let payload = data[29..].to_vec();

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
        let mut payload = Vec::new();
        ciborium::into_writer(&serde_json::json!({"hostname": "styrene-node"}), &mut payload)
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
        let mut data = vec![0u8; 29];
        data[..11].copy_from_slice(b"not.styrene");
        assert!(matches!(StyreneMessage::decode(&data), Err(WireError::InvalidNamespace)));
    }

    #[test]
    fn rejects_unknown_version() {
        let mut data = vec![0u8; 29];
        data[..11].copy_from_slice(b"styrene.io:");
        data[11] = 0xFF;
        assert!(matches!(StyreneMessage::decode(&data), Err(WireError::UnsupportedVersion(0xFF))));
    }

    #[test]
    fn header_size_is_29() {
        let msg = StyreneMessage::new(StyreneMessageType::Ping, &[]);
        let encoded = msg.encode();
        assert_eq!(encoded.len(), 29); // 11 (namespace) + 1 (version) + 1 (type) + 16 (request_id)
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
            StyreneMessageType::I2pProxyRequest,
            StyreneMessageType::I2pProxyResponse,
            StyreneMessageType::I2pProxyData,
            StyreneMessageType::I2pProxyError,
            StyreneMessageType::I2pProxyClose,
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
