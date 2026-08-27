use alloc::vec::Vec;
use hkdf::Hkdf;
use rand_core::{OsRng, RngCore as _};
use rns_core::hash::address_hash;
use sha2::{Digest, Sha256};

pub const FIELD_TICKET: i64 = 0x0C;
pub const TICKET_LENGTH: usize = 16;
pub const COST_TICKET: u32 = 0x100;
/// Pinned LXMF accepts positive stamp costs below 255.
pub const MAX_STAMP_COST: u32 = 254;
pub const STAMP_LENGTH: usize = 32;
pub const MAX_STAMP_LENGTH: usize = STAMP_LENGTH;
pub const TICKET_EXPIRY_SECS: i64 = 21 * 24 * 60 * 60;
pub const TICKET_GRACE_SECS: i64 = 5 * 24 * 60 * 60;
pub const TICKET_RENEW_SECS: i64 = 14 * 24 * 60 * 60;
const WORKBLOCK_EXPAND_ROUNDS: usize = 3000;
pub const PROPAGATION_NODE_WORKBLOCK_ROUNDS: usize = 1000;
pub const PEERING_WORKBLOCK_ROUNDS: usize = 25;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StampGenerationError {
    InvalidCost,
    Cancelled,
    Exhausted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedPropagationStamp {
    pub lxmf_data: Vec<u8>,
    pub stamp: [u8; STAMP_LENGTH],
    pub transient_id: [u8; 32],
    pub value: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidatedPeeringKey {
    pub key: [u8; STAMP_LENGTH],
    pub value: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropagationStampError {
    Missing,
    InvalidLength,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StampState {
    Verified,
    Invalid,
    Unknown,
    NotApplicable,
}

impl StampState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Invalid => "invalid",
            Self::Unknown => "unknown",
            Self::NotApplicable => "not_applicable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StampValidation {
    pub state: StampState,
    pub value: Option<u32>,
    pub used_ticket: bool,
}

pub fn ticket_stamp(ticket: &[u8], message_id: &[u8; 32]) -> Option<Vec<u8>> {
    if ticket.len() != TICKET_LENGTH {
        return None;
    }
    let mut material = Vec::with_capacity(ticket.len() + message_id.len());
    material.extend_from_slice(ticket);
    material.extend_from_slice(message_id);
    Some(address_hash(&material).to_vec())
}

pub fn generate_stamp(message_id: &[u8; 32], stamp_cost: u32) -> Option<Vec<u8>> {
    generate_stamp_with_control(message_id, stamp_cost, || false).ok()
}

/// Generate a stamp without changing the set of valid results.
///
/// The caller owns deadline and cancellation policy through `should_cancel`.
pub fn generate_stamp_with_control<F>(
    message_id: &[u8; 32],
    stamp_cost: u32,
    mut should_cancel: F,
) -> Result<Vec<u8>, StampGenerationError>
where
    F: FnMut() -> bool,
{
    if stamp_cost > MAX_STAMP_COST {
        return Err(StampGenerationError::InvalidCost);
    }
    let workblock = stamp_workblock(message_id, WORKBLOCK_EXPAND_ROUNDS);
    loop {
        if should_cancel() {
            return Err(StampGenerationError::Cancelled);
        }
        let mut stamp = vec![0u8; STAMP_LENGTH];
        OsRng.fill_bytes(&mut stamp);
        if stamp_valid(&stamp, stamp_cost, &workblock) {
            return Ok(stamp);
        }
    }
}

pub fn generate_material_stamp_with_control<F>(
    material: &[u8],
    stamp_cost: u32,
    expand_rounds: usize,
    max_attempts: usize,
    mut should_cancel: F,
) -> Result<[u8; STAMP_LENGTH], StampGenerationError>
where
    F: FnMut() -> bool,
{
    if stamp_cost > MAX_STAMP_COST || expand_rounds > WORKBLOCK_EXPAND_ROUNDS {
        return Err(StampGenerationError::InvalidCost);
    }
    let workblock = stamp_workblock(material, expand_rounds);
    for _ in 0..max_attempts {
        if should_cancel() {
            return Err(StampGenerationError::Cancelled);
        }
        let mut stamp = [0u8; STAMP_LENGTH];
        OsRng.fill_bytes(&mut stamp);
        if stamp_valid(&stamp, stamp_cost, &workblock) {
            return Ok(stamp);
        }
    }
    Err(StampGenerationError::Exhausted)
}

pub fn validate_propagation_stamp(
    stamped_payload: &[u8],
    target_cost: u32,
    flexibility: u32,
) -> Result<ValidatedPropagationStamp, PropagationStampError> {
    let minimum_len = crate::propagation::MIN_PROPAGATED_LXMF_BYTES
        .checked_add(STAMP_LENGTH)
        .ok_or(PropagationStampError::InvalidLength)?;
    if stamped_payload.len() <= minimum_len {
        return Err(if stamped_payload.len() < STAMP_LENGTH {
            PropagationStampError::Missing
        } else {
            PropagationStampError::InvalidLength
        });
    }
    let split = stamped_payload.len() - STAMP_LENGTH;
    let lxmf_data = &stamped_payload[..split];
    let stamp: [u8; STAMP_LENGTH] =
        stamped_payload[split..].try_into().map_err(|_| PropagationStampError::Missing)?;
    let transient_id: [u8; 32] = Sha256::digest(lxmf_data).into();
    let workblock = stamp_workblock(&transient_id, PROPAGATION_NODE_WORKBLOCK_ROUNDS);
    let value = stamp_value(&workblock, &stamp);
    let minimum_cost = target_cost.saturating_sub(flexibility);
    if !stamp_valid(&stamp, minimum_cost, &workblock) {
        return Err(PropagationStampError::Invalid);
    }
    Ok(ValidatedPropagationStamp { lxmf_data: lxmf_data.to_vec(), stamp, transient_id, value })
}

pub fn validate_peering_key(
    key: &[u8],
    local_identity_hash: &[u8; 16],
    remote_identity_hash: &[u8; 16],
    target_cost: u32,
) -> Result<ValidatedPeeringKey, PropagationStampError> {
    let key: [u8; STAMP_LENGTH] =
        key.try_into().map_err(|_| PropagationStampError::InvalidLength)?;
    let mut material = [0u8; 32];
    material[..16].copy_from_slice(local_identity_hash);
    material[16..].copy_from_slice(remote_identity_hash);
    let workblock = stamp_workblock(&material, PEERING_WORKBLOCK_ROUNDS);
    let value = stamp_value(&workblock, &key);
    if !stamp_valid(&key, target_cost, &workblock) {
        return Err(PropagationStampError::Invalid);
    }
    Ok(ValidatedPeeringKey { key, value })
}

pub fn validate_stamp(
    stamp: Option<&[u8]>,
    message_id: &[u8; 32],
    target_cost: Option<u32>,
    tickets: &[Vec<u8>],
) -> StampValidation {
    let Some(target_cost) = target_cost else {
        return StampValidation {
            state: if stamp.is_some() { StampState::Unknown } else { StampState::NotApplicable },
            value: None,
            used_ticket: false,
        };
    };
    if target_cost > MAX_STAMP_COST && target_cost != COST_TICKET {
        return StampValidation { state: StampState::Invalid, value: None, used_ticket: false };
    }
    let Some(stamp) = stamp else {
        return StampValidation {
            state: if target_cost == 0 { StampState::NotApplicable } else { StampState::Invalid },
            value: None,
            used_ticket: false,
        };
    };
    if stamp.len() > MAX_STAMP_LENGTH {
        return StampValidation { state: StampState::Invalid, value: None, used_ticket: false };
    }
    for ticket in tickets.iter().take(256) {
        if target_cost <= COST_TICKET && ticket_stamp(ticket, message_id).as_deref() == Some(stamp)
        {
            return StampValidation {
                state: StampState::Verified,
                value: Some(COST_TICKET),
                used_ticket: true,
            };
        }
    }
    if stamp.len() != STAMP_LENGTH {
        return StampValidation { state: StampState::Invalid, value: None, used_ticket: false };
    }
    if target_cost > MAX_STAMP_COST {
        return StampValidation { state: StampState::Invalid, value: None, used_ticket: false };
    }
    let workblock = stamp_workblock(message_id, WORKBLOCK_EXPAND_ROUNDS);
    let value = stamp_value(&workblock, stamp);
    StampValidation {
        state: if value >= target_cost { StampState::Verified } else { StampState::Invalid },
        value: Some(value),
        used_ticket: false,
    }
}

pub fn stamp_workblock(material: &[u8], expand_rounds: usize) -> Vec<u8> {
    if expand_rounds > WORKBLOCK_EXPAND_ROUNDS {
        return Vec::new();
    }
    let mut workblock = Vec::with_capacity(expand_rounds * 256);
    for n in 0..expand_rounds {
        let mut salt_data = Vec::with_capacity(material.len() + 8);
        salt_data.extend_from_slice(material);
        let packed = rmp_serde::to_vec(&n).unwrap_or_default();
        salt_data.extend_from_slice(&packed);
        let salt_hash = Sha256::digest(&salt_data);
        let hk = Hkdf::<Sha256>::new(Some(salt_hash.as_slice()), material);
        let mut okm = [0u8; 256];
        if hk.expand(&[], &mut okm).is_err() {
            return Vec::new();
        }
        workblock.extend_from_slice(&okm);
    }
    workblock
}

pub fn stamp_valid(stamp: &[u8], target_cost: u32, workblock: &[u8]) -> bool {
    if target_cost == 0 {
        return true;
    }
    if target_cost > 256 {
        return false;
    }
    let mut material = Vec::with_capacity(workblock.len() + stamp.len());
    material.extend_from_slice(workblock);
    material.extend_from_slice(stamp);
    let hash = Sha256::digest(&material);
    if hash.iter().all(|byte| *byte == 0) {
        return true;
    }
    let first_one = hash
        .iter()
        .position(|byte| *byte != 0)
        .map(|byte_index| byte_index as u32 * 8 + hash[byte_index].leading_zeros());
    match first_one {
        Some(leading_zeroes) if leading_zeroes >= target_cost => true,
        Some(leading_zeroes) if leading_zeroes + 1 == target_cost => {
            let bit_index = leading_zeroes as usize;
            let byte_index = bit_index / 8;
            let expected = 1 << (7 - bit_index % 8);
            hash[byte_index] == expected
                && hash[..byte_index].iter().all(|byte| *byte == 0)
                && hash[byte_index + 1..].iter().all(|byte| *byte == 0)
        }
        _ => false,
    }
}

pub fn stamp_value(workblock: &[u8], stamp: &[u8]) -> u32 {
    let mut material = Vec::with_capacity(workblock.len() + stamp.len());
    material.extend_from_slice(workblock);
    material.extend_from_slice(stamp);
    let hash = Sha256::digest(&material);
    let mut value = 0u32;
    for byte in hash {
        if byte == 0 {
            value += 8;
        } else {
            value += byte.leading_zeros();
            break;
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticket_stamp_is_bounded_and_validates() {
        let id = [0x33; 32];
        let ticket = vec![0x44; TICKET_LENGTH];
        let stamp = ticket_stamp(&ticket, &id).expect("valid ticket");
        let result = validate_stamp(Some(&stamp), &id, Some(8), &[ticket]);
        assert_eq!(result.state, StampState::Verified);
        assert!(result.used_ticket);
        assert!(ticket_stamp(&[0; TICKET_LENGTH - 1], &id).is_none());
    }

    #[test]
    fn required_missing_and_oversized_stamps_are_invalid() {
        let id = [0; 32];
        assert_eq!(validate_stamp(None, &id, Some(8), &[]).state, StampState::Invalid);
        assert_eq!(
            validate_stamp(Some(&[0; MAX_STAMP_LENGTH + 1]), &id, Some(8), &[]).state,
            StampState::Invalid
        );
    }

    #[test]
    fn full_pinned_cost_range_is_accepted_and_generation_is_cancellable() {
        let id = [0x77; 32];
        let mut checks = 0;
        let result = generate_stamp_with_control(&id, 254, || {
            checks += 1;
            checks > 1
        });
        assert_eq!(result, Err(StampGenerationError::Cancelled));
        assert_eq!(
            generate_stamp_with_control(&id, 255, || false),
            Err(StampGenerationError::InvalidCost)
        );
    }

    #[test]
    fn propagation_stamp_is_strictly_sized_and_uses_inclusive_flexibility_threshold() {
        let mut lxmf_data = vec![0x31; crate::propagation::MIN_PROPAGATED_LXMF_BYTES + 1];
        lxmf_data[..16].copy_from_slice(&[0x32; 16]);
        let transient_id: [u8; 32] = Sha256::digest(&lxmf_data).into();
        let stamp = generate_material_stamp_with_control(
            &transient_id,
            4,
            PROPAGATION_NODE_WORKBLOCK_ROUNDS,
            100_000,
            || false,
        )
        .unwrap();
        let value =
            stamp_value(&stamp_workblock(&transient_id, PROPAGATION_NODE_WORKBLOCK_ROUNDS), &stamp);
        let mut stamped = lxmf_data.clone();
        stamped.extend_from_slice(&stamp);
        let validated = validate_propagation_stamp(&stamped, value + 3, 3).unwrap();
        assert_eq!(validated.lxmf_data, lxmf_data);
        assert_eq!(validated.transient_id, transient_id);
        assert_eq!(validated.stamp, stamp);
        assert_eq!(validated.value, value);
        assert_eq!(
            validate_propagation_stamp(&stamped, value + 4, 3),
            Err(PropagationStampError::Invalid)
        );
        assert_eq!(
            validate_propagation_stamp(
                &[0; crate::propagation::MIN_PROPAGATED_LXMF_BYTES + STAMP_LENGTH],
                0,
                0,
            ),
            Err(PropagationStampError::InvalidLength)
        );
    }

    #[test]
    fn peering_key_uses_local_then_remote_identity_material() {
        let local = [0x41; 16];
        let remote = [0x42; 16];
        let mut material = [0u8; 32];
        material[..16].copy_from_slice(&local);
        material[16..].copy_from_slice(&remote);
        let key = generate_material_stamp_with_control(
            &material,
            8,
            PEERING_WORKBLOCK_ROUNDS,
            1_000_000,
            || false,
        )
        .unwrap();
        let value = validate_peering_key(&key, &local, &remote, 8).unwrap().value;
        assert!(value >= 8);
        assert_eq!(
            validate_peering_key(&key, &remote, &local, 8),
            Err(PropagationStampError::Invalid)
        );
        assert_eq!(
            validate_peering_key(&key[..31], &local, &remote, 8),
            Err(PropagationStampError::InvalidLength)
        );
    }
}
