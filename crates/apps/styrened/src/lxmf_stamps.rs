pub use lxmf::stamps::{COST_TICKET, FIELD_TICKET, TICKET_LENGTH};

pub fn decode_ticket_hex(ticket_hex: &str) -> Result<Vec<u8>, String> {
    let bytes = hex::decode(ticket_hex.trim())
        .map_err(|error| format!("invalid outbound ticket hex: {error}"))?;
    if bytes.len() != TICKET_LENGTH {
        return Err(format!(
            "invalid outbound ticket length {}; expected {} bytes",
            bytes.len(),
            TICKET_LENGTH
        ));
    }
    Ok(bytes)
}

pub fn ticket_stamp(ticket: &[u8], message_id: &[u8; 32]) -> Vec<u8> {
    lxmf::stamps::ticket_stamp(ticket, message_id).unwrap_or_default()
}

pub fn generate_stamp(message_id: &[u8; 32], stamp_cost: u32) -> Option<Vec<u8>> {
    lxmf::stamps::generate_stamp(message_id, stamp_cost)
}

pub fn validate_stamp(
    stamp: Option<&[u8]>,
    message_id: &[u8; 32],
    target_cost: u32,
    tickets: &[Vec<u8>],
) -> Option<u32> {
    let validation = lxmf::stamps::validate_stamp(stamp, message_id, Some(target_cost), tickets);
    if validation.state == lxmf::stamps::StampState::Verified {
        validation.value
    } else {
        None
    }
}

pub fn stamp_workblock(material: &[u8], expand_rounds: usize) -> Vec<u8> {
    lxmf::stamps::stamp_workblock(material, expand_rounds)
}

pub fn stamp_valid(stamp: &[u8], target_cost: u32, workblock: &[u8]) -> bool {
    lxmf::stamps::stamp_valid(stamp, target_cost, workblock)
}

pub fn stamp_value(workblock: &[u8], stamp: &[u8]) -> u32 {
    lxmf::stamps::stamp_value(workblock, stamp)
}
