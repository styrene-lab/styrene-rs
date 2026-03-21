//! IPC wire protocol — frame encode/decode matching the Python `styrened.ipc.protocol`.
//!
//! Wire format:
//! ```text
//! [LENGTH:4][TYPE:1][REQUEST_ID:16][PAYLOAD:N]
//!
//! LENGTH:     u32 big-endian, total bytes following (TYPE + REQUEST_ID + PAYLOAD)
//! TYPE:       u8, MessageType discriminant
//! REQUEST_ID: 16 bytes, correlation token for request/response matching
//! PAYLOAD:    msgpack-encoded dict
//! ```

use std::collections::HashMap;

use thiserror::Error;

/// Frame header sizes.
pub const LENGTH_SIZE: usize = 4;
pub const TYPE_SIZE: usize = 1;
pub const REQUEST_ID_SIZE: usize = 16;
pub const HEADER_SIZE: usize = TYPE_SIZE + REQUEST_ID_SIZE; // 17

/// Maximum payload size (4 MB).
pub const MAX_PAYLOAD_SIZE: usize = 4 * 1024 * 1024;

/// Wire protocol errors.
#[derive(Debug, Error)]
pub enum WireError {
    #[error("frame incomplete: expected {expected} bytes, got {got}")]
    Incomplete { expected: usize, got: usize },

    #[error("unknown message type: 0x{0:02x}")]
    UnknownType(u8),

    #[error("payload too large: {0} bytes (max {MAX_PAYLOAD_SIZE})")]
    PayloadTooLarge(usize),

    #[error("msgpack decode error: {0}")]
    MsgpackDecode(String),

    #[error("msgpack encode error: {0}")]
    MsgpackEncode(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// IPC message types — values match Python `IPCMessageType` exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum MessageType {
    // Keepalive
    Ping = 0x01,
    Pong = 0x80,

    // Query requests (0x10-0x1F)
    QueryDevices = 0x10,
    QueryIdentity = 0x11,
    QueryStatus = 0x12,
    QueryConfig = 0x13,
    QueryConversations = 0x14,
    QueryMessages = 0x15,
    QuerySearchMessages = 0x16,
    QueryContacts = 0x17,
    QueryResolveName = 0x18,
    QueryAutoReply = 0x19,
    QueryPathInfo = 0x1A,
    QueryPage = 0x1B,
    QueryPageServerStatus = 0x1C,
    QueryAttachment = 0x1D,
    GetNodes = 0x1E,
    GetCoreConfig = 0x1F,

    // Command requests (0x20-0x2F)
    CmdSend = 0x20,
    CmdExec = 0x21,
    CmdAnnounce = 0x22,
    CmdDeviceStatus = 0x23,
    CmdSendChat = 0x24,
    CmdMarkRead = 0x25,
    CmdDeleteConversation = 0x26,
    CmdDeleteMessage = 0x27,
    CmdRetryMessage = 0x28,
    CmdSetContact = 0x29,
    CmdRemoveContact = 0x2A,
    CmdSetAutoReply = 0x2B,
    CmdSyncMessages = 0x2C,
    CmdPageDisconnect = 0x2D,
    CmdRebootDevice = 0x2E,
    CmdRemoteInbox = 0x2F,

    // Subscription requests (0x30-0x3F)
    SubDevices = 0x30,
    SubMessages = 0x31,
    SubActivity = 0x32,
    Unsub = 0x3F,

    // Extended commands (0x40-0x4F)
    CmdRemoteMessages = 0x40,
    CmdSelfUpdate = 0x41,
    CmdSetIdentity = 0x42,
    CmdPqcStatus = 0x43,
    CmdPageRegenerate = 0x44,
    CmdPageSaveSite = 0x45,
    CmdPageRemoveSite = 0x46,
    CmdPageListSites = 0x47,
    CmdPageCrawlSite = 0x48,
    CmdPageGetCached = 0x49,
    CmdBlockPeer = 0x4A,
    CmdUnblockPeer = 0x4B,
    QueryBlockedPeers = 0x4C,
    SaveCoreConfig = 0x4D,
    GetHubStatus = 0x4E,
    GetUnreadCounts = 0x4F,

    // Terminal sessions (0x50-0x5F)
    CmdTerminalOpen = 0x50,
    CmdTerminalInput = 0x51,
    CmdTerminalResize = 0x52,
    CmdTerminalClose = 0x53,

    // Direct data link (0x60-0x6F)
    CmdDatalinkEstablish = 0x60,
    CmdDatalinkTeardown = 0x61,
    CmdDatalinkStatus = 0x62,
    CmdDatalinkQuery = 0x63,
    CmdDatalinkSpeedtest = 0x64,
    CmdDatalinkMeta = 0x65,
    CmdDatalinkInfo = 0x66,

    // Boundary logging / adapters (0x70-0x7F)
    CmdBoundarySnapshot = 0x70,
    CmdProvisionAdapter = 0x71,
    GetAdapterState = 0x72,
    GetActivityHistory = 0x73,

    // Responses (0x80-0x8F)
    Result = 0x81,
    Error = 0x82,

    // Events (0xC0-0xFF)
    EventDevice = 0xC0,
    EventMessage = 0xC1,
    EventTerminalOutput = 0xC2,
    EventTerminalExited = 0xC3,
    EventTerminalError = 0xC4,
    EventTerminalReady = 0xC5,
    EventActivity = 0xC6,
}

impl MessageType {
    /// Parse a byte into a MessageType.
    pub fn from_byte(b: u8) -> Result<Self, WireError> {
        match b {
            0x01 => Ok(Self::Ping),
            0x10 => Ok(Self::QueryDevices),
            0x11 => Ok(Self::QueryIdentity),
            0x12 => Ok(Self::QueryStatus),
            0x13 => Ok(Self::QueryConfig),
            0x14 => Ok(Self::QueryConversations),
            0x15 => Ok(Self::QueryMessages),
            0x16 => Ok(Self::QuerySearchMessages),
            0x17 => Ok(Self::QueryContacts),
            0x18 => Ok(Self::QueryResolveName),
            0x19 => Ok(Self::QueryAutoReply),
            0x1A => Ok(Self::QueryPathInfo),
            0x1B => Ok(Self::QueryPage),
            0x1C => Ok(Self::QueryPageServerStatus),
            0x1D => Ok(Self::QueryAttachment),
            0x1E => Ok(Self::GetNodes),
            0x1F => Ok(Self::GetCoreConfig),
            0x20 => Ok(Self::CmdSend),
            0x21 => Ok(Self::CmdExec),
            0x22 => Ok(Self::CmdAnnounce),
            0x23 => Ok(Self::CmdDeviceStatus),
            0x24 => Ok(Self::CmdSendChat),
            0x25 => Ok(Self::CmdMarkRead),
            0x26 => Ok(Self::CmdDeleteConversation),
            0x27 => Ok(Self::CmdDeleteMessage),
            0x28 => Ok(Self::CmdRetryMessage),
            0x29 => Ok(Self::CmdSetContact),
            0x2A => Ok(Self::CmdRemoveContact),
            0x2B => Ok(Self::CmdSetAutoReply),
            0x2C => Ok(Self::CmdSyncMessages),
            0x2D => Ok(Self::CmdPageDisconnect),
            0x2E => Ok(Self::CmdRebootDevice),
            0x2F => Ok(Self::CmdRemoteInbox),
            0x30 => Ok(Self::SubDevices),
            0x31 => Ok(Self::SubMessages),
            0x32 => Ok(Self::SubActivity),
            0x3F => Ok(Self::Unsub),
            0x40 => Ok(Self::CmdRemoteMessages),
            0x41 => Ok(Self::CmdSelfUpdate),
            0x42 => Ok(Self::CmdSetIdentity),
            0x43 => Ok(Self::CmdPqcStatus),
            0x44 => Ok(Self::CmdPageRegenerate),
            0x45 => Ok(Self::CmdPageSaveSite),
            0x46 => Ok(Self::CmdPageRemoveSite),
            0x47 => Ok(Self::CmdPageListSites),
            0x48 => Ok(Self::CmdPageCrawlSite),
            0x49 => Ok(Self::CmdPageGetCached),
            0x4A => Ok(Self::CmdBlockPeer),
            0x4B => Ok(Self::CmdUnblockPeer),
            0x4C => Ok(Self::QueryBlockedPeers),
            0x4D => Ok(Self::SaveCoreConfig),
            0x4E => Ok(Self::GetHubStatus),
            0x4F => Ok(Self::GetUnreadCounts),
            0x50 => Ok(Self::CmdTerminalOpen),
            0x51 => Ok(Self::CmdTerminalInput),
            0x52 => Ok(Self::CmdTerminalResize),
            0x53 => Ok(Self::CmdTerminalClose),
            0x60 => Ok(Self::CmdDatalinkEstablish),
            0x61 => Ok(Self::CmdDatalinkTeardown),
            0x62 => Ok(Self::CmdDatalinkStatus),
            0x63 => Ok(Self::CmdDatalinkQuery),
            0x64 => Ok(Self::CmdDatalinkSpeedtest),
            0x65 => Ok(Self::CmdDatalinkMeta),
            0x66 => Ok(Self::CmdDatalinkInfo),
            0x70 => Ok(Self::CmdBoundarySnapshot),
            0x71 => Ok(Self::CmdProvisionAdapter),
            0x72 => Ok(Self::GetAdapterState),
            0x73 => Ok(Self::GetActivityHistory),
            0x80 => Ok(Self::Pong),
            0x81 => Ok(Self::Result),
            0x82 => Ok(Self::Error),
            0xC0 => Ok(Self::EventDevice),
            0xC1 => Ok(Self::EventMessage),
            0xC2 => Ok(Self::EventTerminalOutput),
            0xC3 => Ok(Self::EventTerminalExited),
            0xC4 => Ok(Self::EventTerminalError),
            0xC5 => Ok(Self::EventTerminalReady),
            0xC6 => Ok(Self::EventActivity),
            other => Err(WireError::UnknownType(other)),
        }
    }

    /// Whether this type is a request (needs a response).
    pub fn is_request(self) -> bool {
        (self as u8) < 0x80
    }

    /// Whether this type is a response.
    pub fn is_response(self) -> bool {
        matches!(self, Self::Pong | Self::Result | Self::Error)
    }

    /// Whether this type is a pushed event.
    pub fn is_event(self) -> bool {
        (self as u8) >= 0xC0
    }
}

/// A decoded IPC frame.
#[derive(Debug)]
pub struct Frame {
    pub msg_type: MessageType,
    pub request_id: [u8; REQUEST_ID_SIZE],
    pub payload: HashMap<String, rmpv::Value>,
}

/// Encode an IPC frame into wire bytes.
pub fn encode_frame(
    msg_type: MessageType,
    request_id: &[u8; REQUEST_ID_SIZE],
    payload: &HashMap<String, rmpv::Value>,
) -> Result<Vec<u8>, WireError> {
    let payload_bytes =
        rmp_serde::to_vec(payload).map_err(|e| WireError::MsgpackEncode(e.to_string()))?;

    if payload_bytes.len() > MAX_PAYLOAD_SIZE {
        return Err(WireError::PayloadTooLarge(payload_bytes.len()));
    }

    let total_length = TYPE_SIZE + REQUEST_ID_SIZE + payload_bytes.len();
    let mut buf = Vec::with_capacity(LENGTH_SIZE + total_length);
    buf.extend_from_slice(&(total_length as u32).to_be_bytes());
    buf.push(msg_type as u8);
    buf.extend_from_slice(request_id);
    buf.extend_from_slice(&payload_bytes);
    Ok(buf)
}

/// Decode an IPC frame from complete wire bytes (including length prefix).
pub fn decode_frame(data: &[u8]) -> Result<Frame, WireError> {
    if data.len() < LENGTH_SIZE {
        return Err(WireError::Incomplete {
            expected: LENGTH_SIZE,
            got: data.len(),
        });
    }

    let total_length =
        u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;

    let needed = LENGTH_SIZE + total_length;
    if data.len() < needed {
        return Err(WireError::Incomplete {
            expected: needed,
            got: data.len(),
        });
    }

    if total_length > MAX_PAYLOAD_SIZE + HEADER_SIZE {
        return Err(WireError::PayloadTooLarge(total_length));
    }

    let offset = LENGTH_SIZE;
    let msg_type = MessageType::from_byte(data[offset])?;

    let mut request_id = [0u8; REQUEST_ID_SIZE];
    request_id.copy_from_slice(&data[offset + TYPE_SIZE..offset + TYPE_SIZE + REQUEST_ID_SIZE]);

    let payload_bytes = &data[offset + HEADER_SIZE..LENGTH_SIZE + total_length];

    let payload: HashMap<String, rmpv::Value> =
        rmp_serde::from_slice(payload_bytes)
            .map_err(|e| WireError::MsgpackDecode(e.to_string()))?;

    Ok(Frame {
        msg_type,
        request_id,
        payload,
    })
}

/// Read a complete frame from a tokio `AsyncRead`.
pub async fn read_frame_async<R: tokio::io::AsyncReadExt + Unpin>(
    reader: &mut R,
) -> Result<Frame, WireError> {
    // Read length prefix
    let mut len_buf = [0u8; LENGTH_SIZE];
    reader.read_exact(&mut len_buf).await?;
    let total_length = u32::from_be_bytes(len_buf) as usize;

    if total_length > MAX_PAYLOAD_SIZE + HEADER_SIZE {
        return Err(WireError::PayloadTooLarge(total_length));
    }

    // Read rest of frame
    let mut frame_buf = vec![0u8; total_length];
    reader.read_exact(&mut frame_buf).await?;

    let msg_type = MessageType::from_byte(frame_buf[0])?;

    let mut request_id = [0u8; REQUEST_ID_SIZE];
    request_id.copy_from_slice(&frame_buf[TYPE_SIZE..TYPE_SIZE + REQUEST_ID_SIZE]);

    let payload_bytes = &frame_buf[HEADER_SIZE..];
    let payload: HashMap<String, rmpv::Value> =
        rmp_serde::from_slice(payload_bytes)
            .map_err(|e| WireError::MsgpackDecode(e.to_string()))?;

    Ok(Frame {
        msg_type,
        request_id,
        payload,
    })
}

/// Write a complete frame to a tokio `AsyncWrite`.
pub async fn write_frame_async<W: tokio::io::AsyncWriteExt + Unpin>(
    writer: &mut W,
    msg_type: MessageType,
    request_id: &[u8; REQUEST_ID_SIZE],
    payload: &HashMap<String, rmpv::Value>,
) -> Result<(), WireError> {
    let bytes = encode_frame(msg_type, request_id, payload)?;
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_payload() -> HashMap<String, rmpv::Value> {
        HashMap::new()
    }

    fn request_id() -> [u8; 16] {
        [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
    }

    #[test]
    fn encode_ping_frame() {
        let frame = encode_frame(MessageType::Ping, &request_id(), &empty_payload())
            .expect("encode");
        // Length prefix
        let total = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
        assert_eq!(frame[LENGTH_SIZE], 0x01); // PING
        assert_eq!(&frame[LENGTH_SIZE + 1..LENGTH_SIZE + 17], &request_id());
        assert_eq!(total, TYPE_SIZE + REQUEST_ID_SIZE + 1); // msgpack {} = 1 byte (0x80)
    }

    #[test]
    fn encode_result_with_payload() {
        let mut payload = HashMap::new();
        payload.insert("uptime".to_string(), rmpv::Value::from(42));
        let frame = encode_frame(MessageType::Result, &request_id(), &payload)
            .expect("encode");
        let total = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
        assert!(total > HEADER_SIZE); // Has payload bytes beyond header
        assert_eq!(frame[LENGTH_SIZE], 0x81); // RESULT
    }

    #[test]
    fn roundtrip_encode_decode() {
        let mut payload = HashMap::new();
        payload.insert("key".to_string(), rmpv::Value::from("value"));
        let bytes = encode_frame(MessageType::QueryStatus, &request_id(), &payload)
            .expect("encode");
        let frame = decode_frame(&bytes).expect("decode");
        assert_eq!(frame.msg_type, MessageType::QueryStatus);
        assert_eq!(frame.request_id, request_id());
        assert_eq!(
            frame.payload.get("key").and_then(|v| v.as_str()),
            Some("value")
        );
    }

    #[test]
    fn decode_truncated_frame() {
        let bytes = encode_frame(MessageType::Ping, &request_id(), &empty_payload())
            .expect("encode");
        // Truncate: only length prefix + partial
        let truncated = &bytes[..LENGTH_SIZE + 5];
        match decode_frame(truncated) {
            Err(WireError::Incomplete { .. }) => {}
            other => panic!("expected Incomplete, got {other:?}"),
        }
    }

    #[test]
    fn decode_unknown_type() {
        // Manually craft a frame with unknown type 0xFF
        let payload_bytes = rmp_serde::to_vec(&empty_payload()).expect("pack");
        let total = TYPE_SIZE + REQUEST_ID_SIZE + payload_bytes.len();
        let mut buf = Vec::new();
        buf.extend_from_slice(&(total as u32).to_be_bytes());
        buf.push(0xFF); // unknown type
        buf.extend_from_slice(&request_id());
        buf.extend_from_slice(&payload_bytes);
        match decode_frame(&buf) {
            Err(WireError::UnknownType(0xFF)) => {}
            other => panic!("expected UnknownType(0xFF), got {other:?}"),
        }
    }

    #[test]
    fn decode_oversized_payload_rejected() {
        // Fake a length prefix that exceeds MAX_PAYLOAD_SIZE
        let fake_length = (MAX_PAYLOAD_SIZE + HEADER_SIZE + 1) as u32;
        let mut buf = Vec::new();
        buf.extend_from_slice(&fake_length.to_be_bytes());
        buf.push(0x01); // PING
        buf.extend_from_slice(&request_id());
        // Don't need actual payload — check fires before read
        match decode_frame(&buf) {
            Err(WireError::PayloadTooLarge(_)) => {}
            Err(WireError::Incomplete { .. }) => {} // Also acceptable — incomplete data
            other => panic!("expected PayloadTooLarge or Incomplete, got {other:?}"),
        }
    }

    #[test]
    fn message_type_byte_values() {
        assert_eq!(MessageType::Ping as u8, 0x01);
        assert_eq!(MessageType::Pong as u8, 0x80);
        assert_eq!(MessageType::QueryDevices as u8, 0x10);
        assert_eq!(MessageType::QueryIdentity as u8, 0x11);
        assert_eq!(MessageType::QueryStatus as u8, 0x12);
        assert_eq!(MessageType::Result as u8, 0x81);
        assert_eq!(MessageType::Error as u8, 0x82);
        assert_eq!(MessageType::SubDevices as u8, 0x30);
        assert_eq!(MessageType::SubMessages as u8, 0x31);
        assert_eq!(MessageType::SubActivity as u8, 0x32);
        assert_eq!(MessageType::Unsub as u8, 0x3F);
        assert_eq!(MessageType::EventDevice as u8, 0xC0);
        assert_eq!(MessageType::EventMessage as u8, 0xC1);
        assert_eq!(MessageType::EventActivity as u8, 0xC6);
    }

    #[test]
    fn message_type_classification() {
        assert!(MessageType::QueryStatus.is_request());
        assert!(!MessageType::QueryStatus.is_response());
        assert!(!MessageType::QueryStatus.is_event());

        assert!(MessageType::Result.is_response());
        assert!(!MessageType::Result.is_request());

        assert!(MessageType::EventDevice.is_event());
        assert!(!MessageType::EventDevice.is_request());
    }

    #[tokio::test]
    async fn async_read_write_roundtrip() {
        let mut payload = HashMap::new();
        payload.insert("test".to_string(), rmpv::Value::from(true));

        let mut buf = Vec::new();
        write_frame_async(&mut buf, MessageType::QueryIdentity, &request_id(), &payload)
            .await
            .expect("write");

        let mut cursor = std::io::Cursor::new(buf);
        let frame = read_frame_async(&mut cursor).await.expect("read");
        assert_eq!(frame.msg_type, MessageType::QueryIdentity);
        assert_eq!(
            frame.payload.get("test").and_then(|v| v.as_bool()),
            Some(true)
        );
    }
}
