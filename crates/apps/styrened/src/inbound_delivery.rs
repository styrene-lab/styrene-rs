use crate::storage::messages::{CanonicalInboundRecord, MessageRecord};
use lxmf::inbound_decode::decode_inbound_message;
pub use lxmf::inbound_decode::InboundPayloadMode;
use lxmf::WireMessage;

use lxmf::wire_fields::rmpv_to_json_redacting_attachments;

pub fn decode_inbound_payload(
    destination: [u8; 16],
    payload: &[u8],
    mode: InboundPayloadMode,
) -> Option<MessageRecord> {
    decode_inbound_payload_with_diagnostics(destination, payload, mode).0
}

#[derive(Clone)]
pub struct DecodedInboundRecord {
    pub projection: MessageRecord,
    pub canonical: CanonicalInboundRecord,
    pub received_ticket: Option<(i64, Vec<u8>)>,
}

impl std::fmt::Debug for DecodedInboundRecord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DecodedInboundRecord")
            .field("message_id", &self.projection.id)
            .field("source", &self.projection.source)
            .field("destination", &self.projection.destination)
            .field("title_len", &self.projection.title.len())
            .field("content_len", &self.projection.content.len())
            .field("timestamp", &self.projection.timestamp)
            .field("canonical", &self.canonical)
            .field(
                "received_ticket",
                &self
                    .received_ticket
                    .as_ref()
                    .map(|(expires_at, ticket)| (expires_at, ticket.len())),
            )
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct DecodeAttempt {
    pub candidate: &'static str,
    pub len: usize,
    pub error: String,
}

#[derive(Debug, Clone, Default)]
pub struct InboundDecodeDiagnostics {
    pub attempts: Vec<DecodeAttempt>,
}

impl InboundDecodeDiagnostics {
    pub fn summary(&self) -> String {
        if self.attempts.is_empty() {
            return "no decode attempts".to_string();
        }
        self.attempts
            .iter()
            .map(|attempt| format!("{}(len={}):{}", attempt.candidate, attempt.len, attempt.error))
            .collect::<Vec<_>>()
            .join(" | ")
    }
}

pub fn decode_inbound_payload_with_diagnostics(
    destination: [u8; 16],
    payload: &[u8],
    mode: InboundPayloadMode,
) -> (Option<MessageRecord>, InboundDecodeDiagnostics) {
    let mut diagnostics = InboundDecodeDiagnostics::default();
    match decode_inbound_payload_mode(destination, payload, mode) {
        Ok(record) => (Some(record), diagnostics),
        Err(error) => {
            diagnostics.attempts.push(DecodeAttempt {
                candidate: inbound_mode_label(mode),
                len: payload.len(),
                error: error.to_string(),
            });
            (None, diagnostics)
        }
    }
}

fn decode_inbound_payload_mode(
    destination: [u8; 16],
    payload: &[u8],
    mode: InboundPayloadMode,
) -> Result<MessageRecord, lxmf::LxmfError> {
    Ok(decode_canonical_inbound_payload(destination, payload, mode, None, None, &[])?.projection)
}

pub fn decode_canonical_inbound_payload(
    destination: [u8; 16],
    payload: &[u8],
    mode: InboundPayloadMode,
    sender_identity: Option<&rns_core::identity::Identity>,
    stamp_target: Option<u32>,
    issued_tickets: &[Vec<u8>],
) -> Result<DecodedInboundRecord, lxmf::LxmfError> {
    let message = decode_inbound_message(destination, payload, mode)?;
    let authentication = message.authentication_state(sender_identity);
    let message_id: [u8; 32] = hex::decode(&message.id)
        .ok()
        .and_then(|value| value.try_into().ok())
        .ok_or_else(|| lxmf::LxmfError::Decode("invalid canonical message id".into()))?;
    let stamp = lxmf::stamps::validate_stamp(
        message.stamp.as_deref(),
        &message_id,
        stamp_target,
        issued_tickets,
    );
    let received_ticket = extract_ticket(message.fields.as_ref())?;
    let timestamp = if message.timestamp >= i64::MAX as f64 {
        i64::MAX
    } else if message.timestamp <= i64::MIN as f64 {
        i64::MIN
    } else {
        message.timestamp.floor() as i64
    };
    let projection = MessageRecord {
        id: message.id.clone(),
        source: hex::encode(message.source),
        destination: hex::encode(message.destination),
        title: String::from_utf8_lossy(&message.title).into_owned(),
        content: String::from_utf8_lossy(&message.content).into_owned(),
        timestamp,
        direction: "in".into(),
        fields: message.fields.as_ref().and_then(rmpv_to_json_redacting_attachments),
        receipt_status: None,
        read: false,
    };
    let canonical = CanonicalInboundRecord {
        message_id: message.id,
        source: message.source,
        destination: message.destination,
        title: message.title,
        content: message.content,
        timestamp: message.timestamp,
        fields_msgpack: Some(message.fields_msgpack),
        signature: message.signature.map(|value| value.to_vec()),
        stamp: message.stamp,
        wire: message.wire,
        authentication_state: authentication.as_str().into(),
        stamp_state: stamp.state.as_str().into(),
        stamp_value: stamp.value,
        stamp_target,
    };
    Ok(DecodedInboundRecord { projection, canonical, received_ticket })
}

fn extract_ticket(fields: Option<&rmpv::Value>) -> Result<Option<(i64, Vec<u8>)>, lxmf::LxmfError> {
    let Some(fields) = fields else { return Ok(None) };
    let Some(entries) = fields.as_map() else { return Ok(None) };
    let Some((_, value)) =
        entries.iter().find(|(key, _)| key.as_i64() == Some(lxmf::stamps::FIELD_TICKET))
    else {
        return Ok(None);
    };
    let values = value
        .as_array()
        .filter(|values| values.len() == 2)
        .ok_or_else(|| lxmf::LxmfError::Decode("malformed LXMF ticket field".into()))?;
    let expires_at = ticket_expiry(&values[0])?;
    let ticket = values[1]
        .as_slice()
        .filter(|ticket| ticket.len() == lxmf::stamps::TICKET_LENGTH)
        .ok_or_else(|| lxmf::LxmfError::Decode("malformed LXMF ticket bytes".into()))?;
    Ok(Some((expires_at, ticket.to_vec())))
}

fn ticket_expiry(value: &rmpv::Value) -> Result<i64, lxmf::LxmfError> {
    if let Some(expiry) = value.as_i64() {
        return Ok(expiry);
    }
    let expiry = value
        .as_f64()
        .filter(|expiry| expiry.is_finite())
        .ok_or_else(|| lxmf::LxmfError::Decode("malformed LXMF ticket expiry".into()))?;
    let expiry = expiry.ceil();
    if expiry < i64::MIN as f64 || expiry >= 9_223_372_036_854_775_808.0 {
        return Err(lxmf::LxmfError::Decode("LXMF ticket expiry outside integer range".into()));
    }
    Ok(expiry as i64)
}

/// Verify the Ed25519 signature on an LXMF wire message.
///
/// Returns `true` if the signature is valid for the given identity,
/// `false` if verification fails or the message has no signature.
/// Returns `None` if the wire bytes can't be parsed.
pub fn verify_inbound_signature(
    payload: &[u8],
    mode: InboundPayloadMode,
    fallback_destination: [u8; 16],
    sender_identity: &rns_core::identity::Identity,
) -> Option<bool> {
    let wire = match mode {
        InboundPayloadMode::FullWire => payload.to_vec(),
        InboundPayloadMode::DestinationStripped => {
            let mut with_dest = Vec::with_capacity(16 + payload.len());
            with_dest.extend_from_slice(&fallback_destination);
            with_dest.extend_from_slice(payload);
            with_dest
        }
    };

    let wire_msg = WireMessage::unpack(&wire).ok()?;
    Some(wire_msg.verify(sender_identity).unwrap_or(false))
}

fn inbound_mode_label(mode: InboundPayloadMode) -> &'static str {
    match mode {
        InboundPayloadMode::FullWire => "full_wire",
        InboundPayloadMode::DestinationStripped => "destination_stripped",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decode_canonical_inbound_payload, decode_inbound_payload_with_diagnostics, ticket_expiry,
        DecodedInboundRecord,
    };
    use crate::storage::messages::{CanonicalInboundRecord, MessageRecord};
    use lxmf::inbound_decode::InboundPayloadMode;

    #[test]
    fn decode_inbound_payload_accepts_integer_timestamp_wire() {
        let destination = [0x11; 16];
        let source = [0x22; 16];
        let signature = [0x33; 64];
        let payload = rmp_serde::to_vec(&rmpv::Value::Array(vec![
            rmpv::Value::from(1_770_000_000_i64),
            rmpv::Value::from("title"),
            rmpv::Value::from("hello from python-like payload"),
            rmpv::Value::Nil,
        ]))
        .expect("payload encoding");
        let mut wire = Vec::new();
        wire.extend_from_slice(&destination);
        wire.extend_from_slice(&source);
        wire.extend_from_slice(&signature);
        wire.extend_from_slice(&payload);

        let (record, _) = decode_inbound_payload_with_diagnostics(
            destination,
            &wire,
            InboundPayloadMode::FullWire,
        );
        let record = record.expect("decoded record");
        assert_eq!(record.source, hex::encode(source));
        assert_eq!(record.destination, hex::encode(destination));
        assert_eq!(record.title, "title");
        assert_eq!(record.content, "hello from python-like payload");
        assert_eq!(record.timestamp, 1_770_000_000_i64);
        assert_eq!(record.direction, "in");
    }

    #[test]
    fn malformed_ticket_field_fails_closed() {
        let destination = [0x11; 16];
        let mut wire = Vec::new();
        wire.extend_from_slice(&destination);
        wire.extend_from_slice(&[0x22; 16]);
        wire.extend_from_slice(&[0x33; 64]);
        wire.extend_from_slice(
            &rmp_serde::to_vec(&rmpv::Value::Array(vec![
                rmpv::Value::from(1_770_000_000_i64),
                rmpv::Value::Binary(Vec::new()),
                rmpv::Value::Binary(b"content".to_vec()),
                rmpv::Value::Map(vec![(
                    rmpv::Value::from(lxmf::stamps::FIELD_TICKET),
                    rmpv::Value::Array(vec![
                        rmpv::Value::from(1_800_000_000_i64),
                        rmpv::Value::Binary(vec![0; lxmf::stamps::TICKET_LENGTH - 1]),
                    ]),
                )]),
            ]))
            .expect("payload"),
        );
        let result = decode_canonical_inbound_payload(
            destination,
            &wire,
            InboundPayloadMode::FullWire,
            None,
            None,
            &[],
        );
        assert!(result.is_err());
    }

    #[test]
    fn python_f64_ticket_expiry_is_accepted_without_early_expiration() {
        let destination = [0x11; 16];
        let expiry = 1_800_000_000.25;
        let mut wire = Vec::new();
        wire.extend_from_slice(&destination);
        wire.extend_from_slice(&[0x22; 16]);
        wire.extend_from_slice(&[0x33; 64]);
        wire.extend_from_slice(
            &rmp_serde::to_vec(&rmpv::Value::Array(vec![
                rmpv::Value::F64(1_770_000_000.5),
                rmpv::Value::Binary(Vec::new()),
                rmpv::Value::Binary(b"content".to_vec()),
                rmpv::Value::Map(vec![(
                    rmpv::Value::from(lxmf::stamps::FIELD_TICKET),
                    rmpv::Value::Array(vec![
                        rmpv::Value::F64(expiry),
                        rmpv::Value::Binary(vec![0x44; lxmf::stamps::TICKET_LENGTH]),
                    ]),
                )]),
            ]))
            .expect("Python-shaped payload"),
        );
        let decoded = decode_canonical_inbound_payload(
            destination,
            &wire,
            InboundPayloadMode::FullWire,
            None,
            None,
            &[],
        )
        .expect("decode Python-shaped ticket");
        assert_eq!(decoded.received_ticket.map(|ticket| ticket.0), Some(1_800_000_001));
    }

    #[test]
    fn non_finite_and_out_of_range_ticket_expiry_are_rejected() {
        assert!(ticket_expiry(&rmpv::Value::F64(f64::NAN)).is_err());
        assert!(ticket_expiry(&rmpv::Value::F64(f64::INFINITY)).is_err());
        assert!(ticket_expiry(&rmpv::Value::F64(9_223_372_036_854_775_808.0)).is_err());
    }

    #[test]
    fn decoded_inbound_debug_never_exposes_payload_or_ticket_bytes() {
        let secret = b"TOP_SECRET_INBOUND".to_vec();
        let decoded = DecodedInboundRecord {
            projection: MessageRecord {
                id: "safe-id".into(),
                source: "source".into(),
                destination: "destination".into(),
                title: "TOP_SECRET_INBOUND".into(),
                content: "TOP_SECRET_INBOUND".into(),
                timestamp: 1,
                direction: "in".into(),
                fields: None,
                receipt_status: None,
                read: false,
            },
            canonical: CanonicalInboundRecord {
                message_id: "safe-id".into(),
                source: [0x11; 16],
                destination: [0x22; 16],
                title: secret.clone(),
                content: secret.clone(),
                timestamp: 1.0,
                fields_msgpack: Some(secret.clone()),
                signature: Some(secret.clone()),
                stamp: Some(secret.clone()),
                wire: secret.clone(),
                authentication_state: "verified".into(),
                stamp_state: "valid".into(),
                stamp_value: Some(1),
                stamp_target: Some(2),
            },
            received_ticket: Some((2, secret)),
        };
        let debug = format!("{decoded:?}");
        assert!(!debug.contains("TOP_SECRET_INBOUND"));
        assert!(debug.contains("received_ticket"));
        assert!(debug.contains("content_len"));
    }
}
