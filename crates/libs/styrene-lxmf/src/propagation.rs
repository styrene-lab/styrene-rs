//! Pure helpers for the standard LXMF propagation wire exchange.

use alloc::vec::Vec;
use sha2::{Digest, Sha256};

/// Pinned LXMF's minimum accepted propagated-message length.
pub const MIN_PROPAGATED_LXMF_BYTES: usize = 112;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropagationEnvelopeError {
    TooLarge,
    InvalidShape,
    TooManyMessages,
    PayloadTooLarge,
}

pub fn transient_id(payload: &[u8]) -> [u8; 32] {
    Sha256::digest(payload).into()
}

pub fn propagated_destination(payload: &[u8]) -> Option<[u8; 16]> {
    if payload.len() < MIN_PROPAGATED_LXMF_BYTES {
        return None;
    }
    payload.get(..16)?.try_into().ok()
}

/// Decode `[timebase, [payload...]]` without accepting trailing data or
/// non-binary payload entries. Callers select limits appropriate to their RNS
/// transfer boundary.
pub fn decode_transfer_envelope(
    encoded: &[u8],
    max_encoded_bytes: usize,
    max_messages: usize,
    max_payload_bytes: usize,
) -> Result<Vec<Vec<u8>>, PropagationEnvelopeError> {
    if encoded.is_empty() || encoded.len() > max_encoded_bytes {
        return Err(PropagationEnvelopeError::TooLarge);
    }
    let mut cursor = std::io::Cursor::new(encoded);
    let value = rmpv::decode::read_value(&mut cursor)
        .map_err(|_| PropagationEnvelopeError::InvalidShape)?;
    if cursor.position() != encoded.len() as u64 {
        return Err(PropagationEnvelopeError::InvalidShape);
    }
    let fields = value
        .as_array()
        .filter(|fields| fields.len() == 2)
        .ok_or(PropagationEnvelopeError::InvalidShape)?;
    if !matches!(fields[0], rmpv::Value::Integer(_) | rmpv::Value::F32(_) | rmpv::Value::F64(_)) {
        return Err(PropagationEnvelopeError::InvalidShape);
    }
    let payloads = fields[1].as_array().ok_or(PropagationEnvelopeError::InvalidShape)?;
    if payloads.len() > max_messages {
        return Err(PropagationEnvelopeError::TooManyMessages);
    }
    let mut total = 0usize;
    let mut decoded = Vec::with_capacity(payloads.len());
    for payload in payloads {
        let rmpv::Value::Binary(payload) = payload else {
            return Err(PropagationEnvelopeError::InvalidShape);
        };
        total =
            total.checked_add(payload.len()).ok_or(PropagationEnvelopeError::PayloadTooLarge)?;
        if total > max_payload_bytes {
            return Err(PropagationEnvelopeError::PayloadTooLarge);
        }
        decoded.push(payload.clone());
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_envelope_is_strict_and_bounded() {
        let payload = vec![0x42; MIN_PROPAGATED_LXMF_BYTES];
        let encoded = rmp_serde::to_vec(&rmpv::Value::Array(vec![
            rmpv::Value::F64(1.0),
            rmpv::Value::Array(vec![rmpv::Value::Binary(payload.clone())]),
        ]))
        .unwrap();
        assert_eq!(
            decode_transfer_envelope(&encoded, encoded.len(), 1, payload.len()).unwrap(),
            vec![payload]
        );

        let mut trailing = encoded.clone();
        trailing.push(0xc0);
        assert_eq!(
            decode_transfer_envelope(&trailing, trailing.len(), 1, usize::MAX),
            Err(PropagationEnvelopeError::InvalidShape)
        );
        assert_eq!(
            decode_transfer_envelope(&encoded, encoded.len() - 1, 1, usize::MAX),
            Err(PropagationEnvelopeError::TooLarge)
        );
        assert_eq!(
            decode_transfer_envelope(&encoded, encoded.len(), 0, usize::MAX),
            Err(PropagationEnvelopeError::TooManyMessages)
        );
    }

    #[test]
    fn transient_id_and_destination_use_exact_payload_bytes() {
        let mut payload = vec![0x22; MIN_PROPAGATED_LXMF_BYTES];
        payload[..16].copy_from_slice(&[0x11; 16]);
        assert_eq!(propagated_destination(&payload), Some([0x11; 16]));
        assert_eq!(transient_id(&payload), <[u8; 32]>::from(Sha256::digest(&payload)));
        assert_eq!(propagated_destination(&payload[..MIN_PROPAGATED_LXMF_BYTES - 1]), None);
    }
}
