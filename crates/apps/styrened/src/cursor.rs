use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

pub const MAX_CURSOR_LENGTH: usize = 128;

const MAGIC: [u8; 2] = *b"SP";
const VERSION: u8 = 2;
const DIRECTION_BACKWARD: u8 = 1;
const MESSAGE_KIND: u8 = 1;
const CONVERSATION_KIND: u8 = 2;
const HEADER_LENGTH: usize = 30;
const MESSAGE_LENGTH: usize = HEADER_LENGTH + 16 + 8 + 8;
const CONVERSATION_LENGTH: usize = HEADER_LENGTH + 8 + 1 + 8 + 8 + 16;
const AUTH_TAG_LENGTH: usize = 24;

pub type CursorSecret = [u8; 32];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageCursor {
    pub store_id: [u8; 16],
    pub snapshot_seq: i64,
    pub peer: [u8; 16],
    pub sort_timestamp: i64,
    pub ingest_seq: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationCursor {
    pub store_id: [u8; 16],
    pub snapshot_seq: i64,
    pub conversation_epoch: i64,
    pub unread_only: bool,
    pub pinned: bool,
    pub last_sort_timestamp: i64,
    pub last_ingest_seq: i64,
    pub peer: [u8; 16],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CursorError {
    #[error("cursor is malformed")]
    Malformed,
    #[error("cursor has the wrong kind")]
    WrongKind,
}

impl MessageCursor {
    pub fn encode(&self, secret: &CursorSecret) -> String {
        let mut bytes = header(MESSAGE_KIND, 0, self.store_id, self.snapshot_seq);
        bytes.extend_from_slice(&self.peer);
        bytes.extend_from_slice(&self.sort_timestamp.to_be_bytes());
        bytes.extend_from_slice(&self.ingest_seq.to_be_bytes());
        encode_authenticated(bytes, secret)
    }

    pub fn decode(encoded: &str, secret: &CursorSecret) -> Result<Self, CursorError> {
        let bytes = decode_authenticated(encoded, MESSAGE_LENGTH, secret)?;
        let (kind, flags, store_id, snapshot_seq) = parse_header(&bytes)?;
        if kind != MESSAGE_KIND {
            return Err(CursorError::WrongKind);
        }
        if flags != 0 {
            return Err(CursorError::Malformed);
        }
        Ok(Self {
            store_id,
            snapshot_seq,
            peer: array(&bytes[30..46])?,
            sort_timestamp: integer(&bytes[46..54])?,
            ingest_seq: positive_integer(&bytes[54..62])?,
        })
    }
}

impl ConversationCursor {
    pub fn encode(&self, secret: &CursorSecret) -> String {
        let flags = u8::from(self.unread_only);
        let mut bytes = header(CONVERSATION_KIND, flags, self.store_id, self.snapshot_seq);
        bytes.extend_from_slice(&self.conversation_epoch.to_be_bytes());
        bytes.push(u8::from(self.pinned));
        bytes.extend_from_slice(&self.last_sort_timestamp.to_be_bytes());
        bytes.extend_from_slice(&self.last_ingest_seq.to_be_bytes());
        bytes.extend_from_slice(&self.peer);
        encode_authenticated(bytes, secret)
    }

    pub fn decode(encoded: &str, secret: &CursorSecret) -> Result<Self, CursorError> {
        let bytes = decode_authenticated(encoded, CONVERSATION_LENGTH, secret)?;
        let (kind, flags, store_id, snapshot_seq) = parse_header(&bytes)?;
        if kind != CONVERSATION_KIND {
            return Err(CursorError::WrongKind);
        }
        if flags & !1 != 0 || bytes[38] > 1 {
            return Err(CursorError::Malformed);
        }
        Ok(Self {
            store_id,
            snapshot_seq,
            conversation_epoch: nonnegative_integer(&bytes[30..38])?,
            unread_only: flags == 1,
            pinned: bytes[38] == 1,
            last_sort_timestamp: integer(&bytes[39..47])?,
            last_ingest_seq: positive_integer(&bytes[47..55])?,
            peer: array(&bytes[55..71])?,
        })
    }
}

fn header(kind: u8, flags: u8, store_id: [u8; 16], snapshot_seq: i64) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(CONVERSATION_LENGTH);
    bytes.extend_from_slice(&MAGIC);
    bytes.push(VERSION);
    bytes.push(kind);
    bytes.push(DIRECTION_BACKWARD);
    bytes.push(flags);
    bytes.extend_from_slice(&store_id);
    bytes.extend_from_slice(&snapshot_seq.to_be_bytes());
    bytes
}

fn encode_authenticated(mut bytes: Vec<u8>, secret: &CursorSecret) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(&bytes);
    bytes.extend_from_slice(&mac.finalize().into_bytes()[..AUTH_TAG_LENGTH]);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn decode_authenticated(
    encoded: &str,
    payload_length: usize,
    secret: &CursorSecret,
) -> Result<Vec<u8>, CursorError> {
    if encoded.is_empty()
        || encoded.len() > MAX_CURSOR_LENGTH
        || encoded.contains('=')
        || !encoded.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(CursorError::Malformed);
    }
    let bytes = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| CursorError::Malformed)?;
    if bytes.len() != payload_length + AUTH_TAG_LENGTH || URL_SAFE_NO_PAD.encode(&bytes) != encoded
    {
        return Err(CursorError::Malformed);
    }
    let (payload, tag) = bytes.split_at(payload_length);
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).map_err(|_| CursorError::Malformed)?;
    mac.update(payload);
    let expected = mac.finalize().into_bytes();
    if !bool::from(expected[..AUTH_TAG_LENGTH].ct_eq(tag)) {
        return Err(CursorError::Malformed);
    }
    Ok(payload.to_vec())
}

fn parse_header(bytes: &[u8]) -> Result<(u8, u8, [u8; 16], i64), CursorError> {
    if bytes[..2] != MAGIC || bytes[2] != VERSION || bytes[4] != DIRECTION_BACKWARD {
        return Err(CursorError::Malformed);
    }
    Ok((bytes[3], bytes[5], array(&bytes[6..22])?, nonnegative_integer(&bytes[22..30])?))
}

fn array<const N: usize>(bytes: &[u8]) -> Result<[u8; N], CursorError> {
    bytes.try_into().map_err(|_| CursorError::Malformed)
}

fn integer(bytes: &[u8]) -> Result<i64, CursorError> {
    Ok(i64::from_be_bytes(array(bytes)?))
}

fn nonnegative_integer(bytes: &[u8]) -> Result<i64, CursorError> {
    let value = integer(bytes)?;
    (value >= 0).then_some(value).ok_or(CursorError::Malformed)
}

fn positive_integer(bytes: &[u8]) -> Result<i64, CursorError> {
    let value = integer(bytes)?;
    (value > 0).then_some(value).ok_or(CursorError::Malformed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message() -> MessageCursor {
        MessageCursor {
            store_id: [7; 16],
            snapshot_seq: 19,
            peer: [8; 16],
            sort_timestamp: -4,
            ingest_seq: 3,
        }
    }

    const SECRET: CursorSecret = [9; 32];

    #[test]
    fn message_cursor_round_trips_in_bounded_unpadded_form() {
        let cursor = message();
        let encoded = cursor.encode(&SECRET);
        assert!(encoded.len() <= MAX_CURSOR_LENGTH);
        assert!(!encoded.contains('='));
        assert_eq!(MessageCursor::decode(&encoded, &SECRET), Ok(cursor));
    }

    #[test]
    fn decoder_rejects_padding_trailing_bytes_reserved_bits_and_wrong_kind() {
        let encoded = message().encode(&SECRET);
        assert_eq!(
            MessageCursor::decode(&(encoded.clone() + "="), &SECRET),
            Err(CursorError::Malformed)
        );
        assert_eq!(
            MessageCursor::decode(&(encoded.clone() + "A"), &SECRET),
            Err(CursorError::Malformed)
        );
        let mut bytes = URL_SAFE_NO_PAD.decode(&encoded).unwrap();
        bytes[5] = 0x80;
        assert_eq!(
            MessageCursor::decode(&URL_SAFE_NO_PAD.encode(bytes), &SECRET),
            Err(CursorError::Malformed)
        );

        let conversation = ConversationCursor {
            store_id: [1; 16],
            snapshot_seq: 2,
            conversation_epoch: 3,
            unread_only: false,
            pinned: false,
            last_sort_timestamp: 4,
            last_ingest_seq: 5,
            peer: [6; 16],
        };
        assert_eq!(
            MessageCursor::decode(&conversation.encode(&SECRET), &SECRET),
            Err(CursorError::Malformed)
        );
    }

    #[test]
    fn decoder_rejects_oversized_and_noncanonical_input() {
        assert_eq!(
            MessageCursor::decode(&"A".repeat(MAX_CURSOR_LENGTH + 1), &SECRET),
            Err(CursorError::Malformed)
        );
        assert_eq!(MessageCursor::decode("+///", &SECRET), Err(CursorError::Malformed));
        assert_eq!(MessageCursor::decode("", &SECRET), Err(CursorError::Malformed));
    }

    #[test]
    fn conversation_cursor_round_trips_and_strict_fields_fail_closed() {
        let cursor = ConversationCursor {
            store_id: [1; 16],
            snapshot_seq: 2,
            conversation_epoch: 3,
            unread_only: true,
            pinned: true,
            last_sort_timestamp: -4,
            last_ingest_seq: 5,
            peer: [6; 16],
        };
        let encoded = cursor.encode(&SECRET);
        assert!(encoded.len() <= MAX_CURSOR_LENGTH);
        assert_eq!(ConversationCursor::decode(&encoded, &SECRET), Ok(cursor));

        for (index, value) in [(0, b'X'), (2, VERSION + 1), (4, DIRECTION_BACKWARD + 1), (38, 2)] {
            let mut bytes = URL_SAFE_NO_PAD.decode(&encoded).unwrap();
            bytes[index] = value;
            assert_eq!(
                ConversationCursor::decode(&URL_SAFE_NO_PAD.encode(bytes), &SECRET),
                Err(CursorError::Malformed)
            );
        }
        let mut bytes = URL_SAFE_NO_PAD.decode(&encoded).unwrap();
        bytes[47..55].fill(0);
        assert_eq!(
            ConversationCursor::decode(&URL_SAFE_NO_PAD.encode(bytes), &SECRET),
            Err(CursorError::Malformed)
        );
    }

    #[test]
    fn every_authenticated_field_and_wrong_secret_fail_closed() {
        let encoded = message().encode(&SECRET);
        for index in [3, 5, 6, 22, 30, 46, 54] {
            let mut bytes = URL_SAFE_NO_PAD.decode(&encoded).unwrap();
            bytes[index] ^= 1;
            assert_eq!(
                MessageCursor::decode(&URL_SAFE_NO_PAD.encode(bytes), &SECRET),
                Err(CursorError::Malformed)
            );
        }
        assert_eq!(MessageCursor::decode(&encoded, &[8; 32]), Err(CursorError::Malformed));

        let conversation = ConversationCursor {
            store_id: [1; 16],
            snapshot_seq: 2,
            conversation_epoch: 3,
            unread_only: true,
            pinned: true,
            last_sort_timestamp: 4,
            last_ingest_seq: 5,
            peer: [6; 16],
        };
        let encoded = conversation.encode(&SECRET);
        for index in [3, 5, 6, 22, 30, 38, 39, 47, 55] {
            let mut bytes = URL_SAFE_NO_PAD.decode(&encoded).unwrap();
            bytes[index] ^= 1;
            assert_eq!(
                ConversationCursor::decode(&URL_SAFE_NO_PAD.encode(bytes), &SECRET),
                Err(CursorError::Malformed)
            );
        }
    }
}
