use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

pub fn normalize_display_name(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().any(char::is_control) {
        return None;
    }

    let normalized: String = trimmed.chars().take(64).collect();
    if normalized.is_empty() { None } else { Some(normalized) }
}

pub fn encode_delivery_display_name_app_data(display_name: &str) -> Option<Vec<u8>> {
    let normalized = normalize_display_name(display_name)?;
    let peer_data = rmpv::Value::Array(vec![
        rmpv::Value::Binary(normalized.into_bytes()),
        rmpv::Value::Nil,
        rmpv::Value::Array(Vec::new()),
    ]);
    rmp_serde::to_vec(&peer_data).ok()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryAnnounceMetadata {
    pub display_name: Option<String>,
    pub stamp_cost: Option<u32>,
    pub supported_functionality: Vec<u32>,
}

pub fn delivery_announce_metadata(data: &[u8]) -> Option<DeliveryAnnounceMetadata> {
    if data.is_empty() || data.len() > 4096 {
        return None;
    }
    let decoded: rmpv::Value = rmp_serde::from_slice(data).ok()?;
    let values = decoded.as_array()?;
    if values.len() != 3 {
        return None;
    }
    let display_name = match &values[0] {
        rmpv::Value::Nil => None,
        rmpv::Value::Binary(bytes) => normalize_display_name(core::str::from_utf8(bytes).ok()?),
        rmpv::Value::String(value) => normalize_display_name(value.as_str()?),
        _ => return None,
    };
    let stamp_cost = match &values[1] {
        rmpv::Value::Nil => None,
        value => {
            let cost = u32::try_from(value.as_u64()?).ok()?;
            if cost == 0 {
                None
            } else if cost <= crate::stamps::MAX_STAMP_COST {
                Some(cost)
            } else {
                return None;
            }
        }
    };
    let functions = values[2].as_array()?;
    if functions.len() > 64 {
        return None;
    }
    let mut supported_functionality = Vec::with_capacity(functions.len());
    for function in functions {
        supported_functionality.push(u32::try_from(function.as_u64()?).ok()?);
    }
    Some(DeliveryAnnounceMetadata { display_name, stamp_cost, supported_functionality })
}

pub fn display_name_from_delivery_app_data(data: &[u8]) -> Option<String> {
    if data.is_empty() {
        return None;
    }

    let decoded: rmpv::Value = rmp_serde::from_slice(data).ok()?;
    match decoded {
        rmpv::Value::Array(values) => {
            let first = values.first()?;
            match first {
                rmpv::Value::Binary(bytes) => {
                    let raw = String::from_utf8(bytes.clone()).ok()?;
                    normalize_display_name(raw.as_str())
                }
                rmpv::Value::String(value) => normalize_display_name(value.as_str()?),
                _ => None,
            }
        }
        rmpv::Value::Binary(bytes) => {
            let raw = String::from_utf8(bytes).ok()?;
            normalize_display_name(raw.as_str())
        }
        rmpv::Value::String(value) => normalize_display_name(value.as_str()?),
        _ => None,
    }
}

/// Return the recipient's advertised inbound stamp cost from LXMF delivery
/// announce app data `[display_name, stamp_cost]`.
pub fn stamp_cost_from_delivery_app_data(data: &[u8]) -> Option<u32> {
    delivery_announce_metadata(data)?.stamp_cost
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_and_decode_delivery_display_name_round_trip() {
        let encoded = encode_delivery_display_name_app_data("Alice Router").expect("encoded");
        let decoded = display_name_from_delivery_app_data(encoded.as_slice()).expect("decoded");
        assert_eq!(decoded, "Alice Router");
    }

    #[test]
    fn normalize_display_name_rejects_control_bytes() {
        assert!(normalize_display_name("Alice\nRouter").is_none());
    }

    #[test]
    fn extracts_bounded_stamp_cost() {
        let data = rmp_serde::to_vec(&rmpv::Value::Array(vec![
            rmpv::Value::Binary(b"peer".to_vec()),
            rmpv::Value::from(12),
            rmpv::Value::Array(vec![rmpv::Value::from(0)]),
        ]))
        .expect("app data");
        assert_eq!(stamp_cost_from_delivery_app_data(&data), Some(12));
        let metadata = delivery_announce_metadata(&data).expect("metadata");
        assert_eq!(metadata.supported_functionality, vec![0]);
    }

    #[test]
    fn rejects_deprecated_two_element_metadata_for_cost_learning() {
        let data = rmp_serde::to_vec(&rmpv::Value::Array(vec![
            rmpv::Value::Binary(b"peer".to_vec()),
            rmpv::Value::from(12),
        ]))
        .expect("app data");
        assert!(delivery_announce_metadata(&data).is_none());
    }
}
