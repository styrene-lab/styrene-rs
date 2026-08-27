use crate::message::Message;
use crate::LxmfError;
use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::ops::Range;

pub const MAX_INBOUND_WIRE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_CANONICAL_TEXT_BYTES: usize = 1024 * 1024;
pub const MAX_FIELDS_BYTES: usize = 1024 * 1024;
const MAX_FIELD_DEPTH: usize = 32;
const MAX_FIELD_VALUES: usize = 16_384;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthenticationState {
    Verified,
    Invalid,
    UnknownIdentity,
    NotApplicable,
}

impl AuthenticationState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Invalid => "invalid",
            Self::UnknownIdentity => "unknown_identity",
            Self::NotApplicable => "not_applicable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboundPayloadMode {
    FullWire,
    DestinationStripped,
}

#[derive(Debug, Clone)]
pub struct DecodedInboundMessage {
    pub id: String,
    pub source: [u8; 16],
    pub destination: [u8; 16],
    pub title: Vec<u8>,
    pub content: Vec<u8>,
    pub timestamp: f64,
    pub fields: Option<rmpv::Value>,
    /// Exact MessagePack bytes occupied by the payload's fields value, including `nil`.
    pub fields_msgpack: Vec<u8>,
    pub signature: Option<[u8; 64]>,
    pub stamp: Option<Vec<u8>>,
    pub wire: Vec<u8>,
    signed_payload: Vec<u8>,
}

impl DecodedInboundMessage {
    pub fn authentication_state(
        &self,
        identity: Option<&rns_core::identity::Identity>,
    ) -> AuthenticationState {
        let Some(identity) = identity else {
            return if self.signature.is_some() {
                AuthenticationState::UnknownIdentity
            } else {
                AuthenticationState::NotApplicable
            };
        };
        let Some(signature) =
            self.signature.and_then(|bytes| ed25519_dalek::Signature::from_slice(&bytes).ok())
        else {
            return AuthenticationState::Invalid;
        };
        let Ok(message_id): Result<[u8; 32], _> = hex::decode(&self.id)
            .and_then(|bytes| bytes.try_into().map_err(|_| hex::FromHexError::InvalidStringLength))
        else {
            return AuthenticationState::Invalid;
        };
        let mut signed = Vec::with_capacity(16 + 16 + self.signed_payload.len() + 32);
        signed.extend_from_slice(&self.destination);
        signed.extend_from_slice(&self.source);
        signed.extend_from_slice(&self.signed_payload);
        signed.extend_from_slice(&message_id);
        if identity.verify(&signed, &signature).is_ok() {
            AuthenticationState::Verified
        } else {
            AuthenticationState::Invalid
        }
    }
}

pub fn decode_inbound_message(
    fallback_destination: [u8; 16],
    payload: &[u8],
    mode: InboundPayloadMode,
) -> Result<DecodedInboundMessage, LxmfError> {
    if payload.len() > MAX_INBOUND_WIRE_BYTES {
        return Err(LxmfError::Decode("inbound LXMF payload exceeds limit".into()));
    }
    let wire = match mode {
        InboundPayloadMode::FullWire => payload.to_vec(),
        InboundPayloadMode::DestinationStripped => {
            let mut with_destination_prefix = Vec::with_capacity(16 + payload.len());
            with_destination_prefix.extend_from_slice(&fallback_destination);
            with_destination_prefix.extend_from_slice(payload);
            with_destination_prefix
        }
    };
    if wire.len() > MAX_INBOUND_WIRE_BYTES {
        return Err(LxmfError::Decode("reconstructed inbound LXMF payload exceeds limit".into()));
    }

    let payload_ranges = scan_payload_structure(&wire)?;
    let message = Message::from_wire(&wire)?;
    reject_duplicate_field_keys(message.fields.as_ref())?;
    let source = message.source_hash.unwrap_or([0u8; 16]);
    let destination = message.destination_hash.unwrap_or(fallback_destination);
    let wire_message = crate::WireMessage::unpack(&wire)?;
    let id = compute_message_id_hex(
        wire_message.destination,
        wire_message.source,
        &payload_ranges.signed_payload,
    );
    if !wire_message.payload.timestamp.is_finite() {
        return Err(LxmfError::Decode("invalid non-finite payload timestamp".into()));
    }
    if message.title.len() > MAX_CANONICAL_TEXT_BYTES
        || message.content.len() > MAX_CANONICAL_TEXT_BYTES
    {
        return Err(LxmfError::Decode("inbound LXMF title or content exceeds limit".into()));
    }
    let fields_msgpack = wire[payload_ranges.fields].to_vec();
    let stamp = message.stamp.clone();
    if stamp.as_ref().is_some_and(|value| value.len() > crate::stamps::MAX_STAMP_LENGTH) {
        return Err(LxmfError::Decode("inbound LXMF stamp exceeds limit".into()));
    }
    Ok(DecodedInboundMessage {
        id,
        source,
        destination,
        title: message.title,
        content: message.content,
        timestamp: message.timestamp.unwrap_or(0.0),
        fields: message.fields,
        fields_msgpack,
        signature: wire_message.signature,
        stamp,
        wire,
        signed_payload: payload_ranges.signed_payload,
    })
}

struct PayloadRanges {
    fields: Range<usize>,
    signed_payload: Vec<u8>,
}

fn scan_payload_structure(wire: &[u8]) -> Result<PayloadRanges, LxmfError> {
    const HEADER_LEN: usize = 16 + 16 + 64;
    let payload =
        wire.get(HEADER_LEN..).ok_or_else(|| LxmfError::Decode("wire message too short".into()))?;
    let (item_count, mut cursor) = array_header(payload)?;
    if !(4..=5).contains(&item_count) {
        return Err(LxmfError::Decode("invalid payload length".into()));
    }
    let mut ranges = Vec::with_capacity(item_count);
    let mut nodes = 0usize;
    for _ in 0..item_count {
        let start = cursor;
        cursor = scan_msgpack_value(payload, cursor, &mut nodes)?;
        ranges.push(start..cursor);
    }
    if cursor != payload.len() {
        return Err(LxmfError::Decode("trailing bytes after LXMF payload".into()));
    }
    enforce_scalar_size(payload, &ranges[1], MAX_CANONICAL_TEXT_BYTES, "title")?;
    enforce_scalar_size(payload, &ranges[2], MAX_CANONICAL_TEXT_BYTES, "content")?;
    if ranges[3].len() > MAX_FIELDS_BYTES {
        return Err(LxmfError::Decode("inbound LXMF fields exceed limit".into()));
    }
    if let Some(stamp) = ranges.get(4) {
        enforce_scalar_size(payload, stamp, crate::stamps::MAX_STAMP_LENGTH, "stamp")?;
    }
    let signed_payload = if item_count == 4 {
        payload.to_vec()
    } else {
        if payload.first() != Some(&0x95) {
            return Err(LxmfError::Decode(
                "stamp-bearing LXMF payload must use Python-compatible fixarray encoding".into(),
            ));
        }
        let mut signed = Vec::with_capacity(1 + ranges[3].end - ranges[0].start);
        signed.push(0x94);
        signed.extend_from_slice(&payload[ranges[0].start..ranges[3].end]);
        signed
    };
    Ok(PayloadRanges {
        fields: (HEADER_LEN + ranges[3].start)..(HEADER_LEN + ranges[3].end),
        signed_payload,
    })
}

pub(crate) fn signed_payload_bytes(wire: &[u8]) -> Result<Vec<u8>, LxmfError> {
    Ok(scan_payload_structure(wire)?.signed_payload)
}

fn array_header(bytes: &[u8]) -> Result<(usize, usize), LxmfError> {
    let marker = *bytes.first().ok_or_else(|| LxmfError::Decode("empty payload".into()))?;
    match marker {
        0x90..=0x9f => Ok(((marker & 0x0f) as usize, 1)),
        0xdc => Ok((read_u16(bytes, 1)? as usize, 3)),
        0xdd => Ok((read_u32(bytes, 1)? as usize, 5)),
        _ => Err(LxmfError::Decode("invalid payload structure".into())),
    }
}

fn scan_msgpack_value(bytes: &[u8], offset: usize, nodes: &mut usize) -> Result<usize, LxmfError> {
    let mut cursor = offset;
    let mut pending = Vec::with_capacity(MAX_FIELD_DEPTH + 1);
    pending.push(1usize);
    while !pending.is_empty() {
        if pending.last() == Some(&0) {
            pending.pop();
            continue;
        }
        let remaining = pending
            .last_mut()
            .ok_or_else(|| LxmfError::Decode("invalid MessagePack scanner state".into()))?;
        *remaining -= 1;
        let depth = pending.len() - 1;
        if depth > MAX_FIELD_DEPTH {
            return Err(LxmfError::Decode("MessagePack value exceeds nesting limit".into()));
        }
        *nodes = nodes.saturating_add(1);
        if *nodes > MAX_FIELD_VALUES {
            return Err(LxmfError::Decode("MessagePack value exceeds node limit".into()));
        }
        let (next, children) = scan_msgpack_marker(bytes, cursor)?;
        cursor = next;
        if children > 0 {
            if children > MAX_FIELD_VALUES {
                return Err(LxmfError::Decode("MessagePack collection exceeds limit".into()));
            }
            pending.push(children);
        }
    }
    Ok(cursor)
}

fn scan_msgpack_marker(bytes: &[u8], offset: usize) -> Result<(usize, usize), LxmfError> {
    let marker = *bytes
        .get(offset)
        .ok_or_else(|| LxmfError::Decode("truncated MessagePack value".into()))?;
    let start = offset.checked_add(1).ok_or_else(|| LxmfError::Decode("offset overflow".into()))?;
    let scalar = |end| Ok((end, 0));
    match marker {
        0x00..=0x7f | 0xc0 | 0xc2 | 0xc3 | 0xe0..=0xff => scalar(start),
        0x80..=0x8f => Ok((start, (marker & 0x0f) as usize * 2)),
        0x90..=0x9f => Ok((start, (marker & 0x0f) as usize)),
        0xa0..=0xbf => scalar(advance(bytes, start, (marker & 0x1f) as usize)?),
        0xc4 | 0xd9 => {
            let length = read_u8(bytes, start)? as usize;
            scalar(advance(bytes, start + 1, length)?)
        }
        0xc5 | 0xda => {
            let length = read_u16(bytes, start)? as usize;
            scalar(advance(bytes, start + 2, length)?)
        }
        0xc6 | 0xdb => {
            let length = usize::try_from(read_u32(bytes, start)?)
                .map_err(|_| LxmfError::Decode("MessagePack length exceeds platform".into()))?;
            scalar(advance(bytes, start + 4, length)?)
        }
        0xc7 => {
            let length = read_u8(bytes, start)? as usize;
            scalar(advance(bytes, start + 2, length)?)
        }
        0xc8 => {
            let length = read_u16(bytes, start)? as usize;
            scalar(advance(bytes, start + 3, length)?)
        }
        0xc9 => {
            let length = usize::try_from(read_u32(bytes, start)?)
                .map_err(|_| LxmfError::Decode("MessagePack length exceeds platform".into()))?;
            scalar(advance(bytes, start + 5, length)?)
        }
        0xca => scalar(advance(bytes, start, 4)?),
        0xcb => scalar(advance(bytes, start, 8)?),
        0xcc | 0xd0 => scalar(advance(bytes, start, 1)?),
        0xcd | 0xd1 => scalar(advance(bytes, start, 2)?),
        0xce | 0xd2 => scalar(advance(bytes, start, 4)?),
        0xcf | 0xd3 => scalar(advance(bytes, start, 8)?),
        0xd4 => scalar(advance(bytes, start, 2)?),
        0xd5 => scalar(advance(bytes, start, 3)?),
        0xd6 => scalar(advance(bytes, start, 5)?),
        0xd7 => scalar(advance(bytes, start, 9)?),
        0xd8 => scalar(advance(bytes, start, 17)?),
        0xdc => {
            let count = read_u16(bytes, start)? as usize;
            Ok((start + 2, count))
        }
        0xdd => {
            let count = usize::try_from(read_u32(bytes, start)?)
                .map_err(|_| LxmfError::Decode("MessagePack array exceeds platform".into()))?;
            Ok((start + 4, count))
        }
        0xde => {
            let count = read_u16(bytes, start)? as usize;
            Ok((start + 2, count.saturating_mul(2)))
        }
        0xdf => {
            let count = usize::try_from(read_u32(bytes, start)?)
                .map_err(|_| LxmfError::Decode("MessagePack map exceeds platform".into()))?;
            Ok((start + 4, count.saturating_mul(2)))
        }
        0xc1 => Err(LxmfError::Decode("reserved MessagePack marker".into())),
    }
}

fn enforce_scalar_size(
    bytes: &[u8],
    range: &Range<usize>,
    limit: usize,
    name: &str,
) -> Result<(), LxmfError> {
    let encoded_overhead = match bytes.get(range.start).copied() {
        Some(0xa0..=0xbf) => 1,
        Some(0xc4 | 0xd9) => 2,
        Some(0xc5 | 0xda) => 3,
        Some(0xc6 | 0xdb) => 5,
        Some(0xc0) => return Ok(()),
        _ => return Err(LxmfError::Decode(format!("invalid inbound LXMF {name} encoding"))),
    };
    if range.len().saturating_sub(encoded_overhead) > limit {
        return Err(LxmfError::Decode(format!("inbound LXMF {name} exceeds limit")));
    }
    Ok(())
}

fn reject_duplicate_field_keys(fields: Option<&rmpv::Value>) -> Result<(), LxmfError> {
    let Some(entries) = fields.and_then(rmpv::Value::as_map) else {
        return Ok(());
    };
    let mut keys = BTreeSet::new();
    for (key, _) in entries {
        let encoded = rmp_serde::to_vec(key)
            .map_err(|error| LxmfError::Decode(format!("invalid LXMF field key: {error}")))?;
        if !keys.insert(encoded) {
            return Err(LxmfError::Decode("duplicate LXMF field key".into()));
        }
    }
    Ok(())
}

fn advance(bytes: &[u8], offset: usize, length: usize) -> Result<usize, LxmfError> {
    let end =
        offset.checked_add(length).ok_or_else(|| LxmfError::Decode("offset overflow".into()))?;
    if end > bytes.len() {
        return Err(LxmfError::Decode("truncated MessagePack value".into()));
    }
    Ok(end)
}

fn read_u8(bytes: &[u8], offset: usize) -> Result<u8, LxmfError> {
    bytes
        .get(offset)
        .copied()
        .ok_or_else(|| LxmfError::Decode("truncated MessagePack length".into()))
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, LxmfError> {
    let value: [u8; 2] = bytes
        .get(offset..offset + 2)
        .and_then(|value| value.try_into().ok())
        .ok_or_else(|| LxmfError::Decode("truncated MessagePack length".into()))?;
    Ok(u16::from_be_bytes(value))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, LxmfError> {
    let value: [u8; 4] = bytes
        .get(offset..offset + 4)
        .and_then(|value| value.try_into().ok())
        .ok_or_else(|| LxmfError::Decode("truncated MessagePack length".into()))?;
    Ok(u32::from_be_bytes(value))
}

/// Compute the canonical message ID for an outbound LXMF wire payload.
///
/// Uses the same derivation as the inbound decoder: SHA-256 of
/// destination + source + payload (without stamp). This ensures
/// sender and receiver agree on the message ID.
pub fn outbound_message_id_hex(candidate: &[u8]) -> Option<String> {
    wire_message_id_hex(candidate)
}

fn wire_message_id_hex(candidate: &[u8]) -> Option<String> {
    const HEADER_LEN: usize = 16 + 16 + 64;
    if candidate.len() <= HEADER_LEN || candidate.len() > MAX_INBOUND_WIRE_BYTES {
        return None;
    }
    let mut destination = [0u8; 16];
    destination.copy_from_slice(&candidate[..16]);
    let mut source = [0u8; 16];
    source.copy_from_slice(&candidate[16..32]);
    let ranges = scan_payload_structure(candidate).ok()?;
    Some(compute_message_id_hex(destination, source, &ranges.signed_payload))
}

fn compute_message_id_hex(
    destination: [u8; 16],
    source: [u8; 16],
    payload_without_stamp: &[u8],
) -> String {
    use sha2::Digest as _;
    let mut hasher = sha2::Sha256::new();
    hasher.update(destination);
    hasher.update(source);
    hasher.update(payload_without_stamp);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(payload: rmpv::Value) -> Vec<u8> {
        let mut wire = vec![0x11; 16];
        wire.extend_from_slice(&[0x22; 16]);
        wire.extend_from_slice(&[0x33; 64]);
        wire.extend_from_slice(&rmp_serde::to_vec(&payload).expect("payload"));
        wire
    }

    #[test]
    fn preserves_binary_content_fractional_timestamp_and_fields() {
        let fields = rmpv::Value::Map(vec![(
            rmpv::Value::from(7),
            rmpv::Value::Array(vec![rmpv::Value::Binary(vec![0xff]), rmpv::Value::from(true)]),
        )]);
        let decoded = decode_inbound_message(
            [0x11; 16],
            &wire(rmpv::Value::Array(vec![
                rmpv::Value::F64(1_700_000_000.125),
                rmpv::Value::Binary(vec![0xfe]),
                rmpv::Value::Binary(vec![0xff, 0x00]),
                fields.clone(),
            ])),
            InboundPayloadMode::FullWire,
        )
        .expect("canonical decode");
        assert_eq!(decoded.timestamp, 1_700_000_000.125);
        assert_eq!(decoded.title, vec![0xfe]);
        assert_eq!(decoded.content, vec![0xff, 0x00]);
        let roundtrip: rmpv::Value =
            rmp_serde::from_slice(&decoded.fields_msgpack).expect("fields decode");
        assert_eq!(roundtrip, fields);
    }

    #[test]
    fn rejects_non_finite_timestamp_and_oversized_input() {
        let non_finite = wire(rmpv::Value::Array(vec![
            rmpv::Value::F64(f64::NAN),
            rmpv::Value::Nil,
            rmpv::Value::Nil,
            rmpv::Value::Nil,
        ]));
        assert!(decode_inbound_message([0; 16], &non_finite, InboundPayloadMode::FullWire).is_err());
        assert!(decode_inbound_message(
            [0; 16],
            &vec![0; MAX_INBOUND_WIRE_BYTES + 1],
            InboundPayloadMode::FullWire
        )
        .is_err());
        assert!(decode_inbound_message(
            [0; 16],
            &vec![0; MAX_INBOUND_WIRE_BYTES],
            InboundPayloadMode::DestinationStripped
        )
        .is_err());
    }

    #[test]
    fn preserves_original_noncanonical_field_encoding() {
        let mut payload = vec![0x94, 0xcb];
        payload.extend_from_slice(&1_700_000_000.5_f64.to_bits().to_be_bytes());
        payload.extend_from_slice(&[0xc4, 0x00, 0xc4, 0x00]);
        let raw_fields = [0x81, 0xcc, 0x01, 0xcd, 0x00, 0x02];
        payload.extend_from_slice(&raw_fields);
        let mut full_wire = vec![0x11; 16];
        full_wire.extend_from_slice(&[0x22; 16]);
        full_wire.extend_from_slice(&[0x33; 64]);
        full_wire.extend_from_slice(&payload);
        let decoded = decode_inbound_message([0x11; 16], &full_wire, InboundPayloadMode::FullWire)
            .expect("decode noncanonical fields");
        assert_eq!(decoded.fields_msgpack, raw_fields);
    }

    #[test]
    fn rejects_structural_limits_before_value_deserialization() {
        let mut deeply_nested = vec![0x94, 0xcb];
        deeply_nested.extend_from_slice(&1_700_000_000.5_f64.to_bits().to_be_bytes());
        deeply_nested.extend_from_slice(&[0xc4, 0x00, 0xc4, 0x00]);
        deeply_nested.extend(std::iter::repeat_n(0x91, MAX_FIELD_DEPTH + 2));
        deeply_nested.push(0xc0);
        assert!(decode_inbound_message(
            [0x11; 16],
            &wire_bytes(&deeply_nested),
            InboundPayloadMode::FullWire,
        )
        .is_err());

        let mut excessive_nodes = vec![0x94, 0xcb];
        excessive_nodes.extend_from_slice(&1_700_000_000.5_f64.to_bits().to_be_bytes());
        excessive_nodes.extend_from_slice(&[0xc4, 0x00, 0xc4, 0x00, 0xdd]);
        excessive_nodes.extend_from_slice(&((MAX_FIELD_VALUES + 1) as u32).to_be_bytes());
        assert!(decode_inbound_message(
            [0x11; 16],
            &wire_bytes(&excessive_nodes),
            InboundPayloadMode::FullWire,
        )
        .is_err());
    }

    #[test]
    fn semantic_messagepack_rewrite_changes_id_and_invalidates_signature() {
        let signer = rns_core::identity::PrivateIdentity::new_from_name("exact-lxmf-payload");
        let source = signer.as_identity().address_hash.as_slice().try_into().expect("source hash");
        let payload = crate::Payload::new(
            1_700_000_000.5,
            Some(Vec::new()),
            Some(Vec::new()),
            Some(rmpv::Value::Map(vec![(rmpv::Value::from(1), rmpv::Value::from(2))])),
            None,
        );
        let mut message = crate::WireMessage::new([0x11; 16], source, payload);
        message.sign(&signer).expect("sign canonical payload");
        let canonical = message.pack().expect("pack canonical payload");
        let canonical_decoded =
            decode_inbound_message([0x11; 16], &canonical, InboundPayloadMode::FullWire)
                .expect("decode canonical payload");
        assert_eq!(
            canonical_decoded.authentication_state(Some(signer.as_identity())),
            AuthenticationState::Verified
        );

        let canonical_fields = [0x81, 0x01, 0x02];
        assert!(canonical.ends_with(&canonical_fields));
        let mut rewritten = canonical[..canonical.len() - canonical_fields.len()].to_vec();
        rewritten.extend_from_slice(&[0x81, 0xcc, 0x01, 0xcd, 0x00, 0x02]);
        let rewritten_decoded =
            decode_inbound_message([0x11; 16], &rewritten, InboundPayloadMode::FullWire)
                .expect("decode rewritten payload");
        assert_ne!(rewritten_decoded.id, canonical_decoded.id);
        assert_eq!(
            rewritten_decoded.authentication_state(Some(signer.as_identity())),
            AuthenticationState::Invalid
        );
        assert!(!crate::WireMessage::unpack(&rewritten)
            .expect("unpack rewritten payload")
            .verify(signer.as_identity())
            .expect("verify rewritten payload"));
    }

    #[test]
    fn stamp_bearing_payload_uses_exact_four_element_signature_input() {
        let signer = rns_core::identity::PrivateIdentity::new_from_name("stamped-lxmf-payload");
        let source = signer.as_identity().address_hash.as_slice().try_into().expect("source hash");
        let payload = crate::Payload::new(
            1_700_000_000.5,
            Some(vec![0xaa]),
            Some(vec![0xbb]),
            None,
            Some(vec![0xcc; crate::stamps::STAMP_LENGTH]),
        );
        let mut message = crate::WireMessage::new([0x11; 16], source, payload);
        message.sign(&signer).expect("sign stamped payload");
        let wire = message.pack().expect("pack stamped payload");
        assert_eq!(wire[96], 0x95);
        let decoded = decode_inbound_message([0x11; 16], &wire, InboundPayloadMode::FullWire)
            .expect("decode stamped payload");
        assert_eq!(
            decoded.authentication_state(Some(signer.as_identity())),
            AuthenticationState::Verified
        );
    }

    #[test]
    fn duplicate_field_keys_are_rejected() {
        let mut payload = vec![0x94, 0xcb];
        payload.extend_from_slice(&1_700_000_000.5_f64.to_bits().to_be_bytes());
        payload.extend_from_slice(&[0xc4, 0x00, 0xc4, 0x00]);
        payload.extend_from_slice(&[0x82, 0x01, 0x02, 0xcc, 0x01, 0x03]);
        assert!(decode_inbound_message(
            [0x11; 16],
            &wire_bytes(&payload),
            InboundPayloadMode::FullWire,
        )
        .is_err());
    }

    fn wire_bytes(payload: &[u8]) -> Vec<u8> {
        let mut wire = vec![0x11; 16];
        wire.extend_from_slice(&[0x22; 16]);
        wire.extend_from_slice(&[0x33; 64]);
        wire.extend_from_slice(payload);
        wire
    }
}
