//! Standard LXMF propagation-node announce metadata.

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

const MIN_FIELD_COUNT: usize = 7;
const NODE_NAME_KEY: u64 = 1;
/// Reticulum's 464-byte packet MDU minus the 148-byte announce payload overhead.
pub const MAX_APP_DATA_BYTES: usize = 316;
/// Keeps canonical metadata comfortably within the Reticulum announce budget.
pub const MAX_NODE_NAME_BYTES: usize = 256;
const MAX_EMITTED_LIMIT_KB: i64 = 16 * 1024 * 1024;
const MAX_EMITTED_UNIX_SECS: i64 = 253_402_300_799;

pub const DEFAULT_TRANSFER_LIMIT_KB: i64 = 256;
pub const DEFAULT_SYNC_LIMIT_KB: i64 = 10_240;
pub const DEFAULT_STAMP_COST: i64 = 16;
pub const DEFAULT_STAMP_COST_FLEXIBILITY: i64 = 3;
pub const DEFAULT_PEERING_COST: i64 = 18;

/// Parsed pinned-validator values. Numeric fields intentionally retain the full
/// signed range accepted by bounded Python-like `int()` coercion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StandardPropagationAnnounce {
    pub emitted_at: i64,
    pub node_active: bool,
    pub transfer_limit_kb: i64,
    pub sync_limit_kb: i64,
    pub stamp_cost: i64,
    pub stamp_cost_flexibility: i64,
    pub peering_cost: i64,
    pub node_name: Option<String>,
}

impl StandardPropagationAnnounce {
    pub fn inactive(emitted_at: i64, node_name: Option<&str>) -> Result<Self, AnnounceError> {
        let node_name = node_name.map(validate_node_name).transpose()?;
        let value = Self {
            emitted_at,
            node_active: false,
            transfer_limit_kb: DEFAULT_TRANSFER_LIMIT_KB,
            sync_limit_kb: DEFAULT_SYNC_LIMIT_KB,
            stamp_cost: DEFAULT_STAMP_COST,
            stamp_cost_flexibility: DEFAULT_STAMP_COST_FLEXIBILITY,
            peering_cost: DEFAULT_PEERING_COST,
            node_name,
        };
        validate_emission(&value)?;
        Ok(value)
    }

    pub fn active(
        emitted_at: i64,
        node_name: Option<&str>,
        transfer_limit_kb: i64,
        sync_limit_kb: i64,
    ) -> Result<Self, AnnounceError> {
        let mut value = Self::inactive(emitted_at, node_name)?;
        value.node_active = true;
        value.transfer_limit_kb = transfer_limit_kb;
        value.sync_limit_kb = sync_limit_kb;
        validate_emission(&value)?;
        Ok(value)
    }

    /// Emit the canonical seven-element form with integer numeric values.
    pub fn encode(&self) -> Result<Vec<u8>, AnnounceError> {
        validate_emission(self)?;
        let mut metadata = Vec::new();
        if let Some(name) = &self.node_name {
            metadata.push((
                rmpv::Value::from(NODE_NAME_KEY),
                rmpv::Value::Binary(name.as_bytes().to_vec()),
            ));
        }
        let value = rmpv::Value::Array(vec![
            rmpv::Value::Boolean(false),
            rmpv::Value::from(self.emitted_at),
            rmpv::Value::Boolean(self.node_active),
            rmpv::Value::from(self.transfer_limit_kb),
            rmpv::Value::from(self.sync_limit_kb),
            rmpv::Value::Array(vec![
                rmpv::Value::from(self.stamp_cost),
                rmpv::Value::from(self.stamp_cost_flexibility),
                rmpv::Value::from(self.peering_cost),
            ]),
            rmpv::Value::Map(metadata),
        ]);
        let encoded = rmp_serde::to_vec(&value).map_err(|_| AnnounceError::Encoding)?;
        if encoded.len() > MAX_APP_DATA_BYTES {
            return Err(AnnounceError::AppDataTooLarge);
        }
        Ok(encoded)
    }

    /// Parse the permissive shape accepted by pinned LXMF revision
    /// `795fdaa2b0777c13033787d933d1afc94a2377cb`.
    pub fn parse(data: &[u8]) -> Result<Self, AnnounceError> {
        if data.is_empty() || data.len() > MAX_APP_DATA_BYTES {
            return Err(AnnounceError::InvalidShape);
        }
        let value: rmpv::Value =
            rmp_serde::from_slice(data).map_err(|_| AnnounceError::InvalidShape)?;
        let fields = value.as_array().ok_or(AnnounceError::InvalidShape)?;
        if fields.len() < MIN_FIELD_COUNT {
            return Err(AnnounceError::InvalidShape);
        }
        let costs = fields[5]
            .as_array()
            .filter(|values| values.len() >= 3)
            .ok_or(AnnounceError::InvalidShape)?;
        let metadata = fields[6].as_map().ok_or(AnnounceError::InvalidShape)?;
        let mut node_name = None;
        for (key, value) in metadata {
            if key.as_u64() != Some(NODE_NAME_KEY) {
                continue;
            }
            let rmpv::Value::Binary(bytes) = value else {
                node_name = None;
                continue;
            };
            let Ok(raw) = core::str::from_utf8(bytes) else {
                node_name = None;
                continue;
            };
            // Python MessagePack maps retain one value per key. If a non-canonical
            // encoder supplies duplicates, matching last-value behavior is safest.
            node_name = Some(raw.to_string());
        }
        Ok(Self {
            emitted_at: python_int(&fields[1])?,
            node_active: python_bool(&fields[2])?,
            transfer_limit_kb: python_int(&fields[3])?,
            sync_limit_kb: python_int(&fields[4])?,
            stamp_cost: python_int(&costs[0])?,
            stamp_cost_flexibility: python_int(&costs[1])?,
            peering_cost: python_int(&costs[2])?,
            node_name,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnnounceError {
    InvalidShape,
    InvalidNumber,
    InvalidActive,
    InvalidEmission,
    EmptyName,
    ControlCharacterName,
    NameTooLong,
    InvalidNameType,
    InvalidNameUtf8,
    AppDataTooLarge,
    Encoding,
}

fn validate_node_name(value: &str) -> Result<String, AnnounceError> {
    if value.is_empty() {
        return Err(AnnounceError::EmptyName);
    }
    if value.chars().any(char::is_control) {
        return Err(AnnounceError::ControlCharacterName);
    }
    if value.len() > MAX_NODE_NAME_BYTES {
        return Err(AnnounceError::NameTooLong);
    }
    Ok(value.to_string())
}

fn validate_emission(value: &StandardPropagationAnnounce) -> Result<(), AnnounceError> {
    if value.emitted_at < 0
        || value.emitted_at > MAX_EMITTED_UNIX_SECS
        || value.transfer_limit_kb <= 0
        || value.transfer_limit_kb > MAX_EMITTED_LIMIT_KB
        || value.sync_limit_kb < value.transfer_limit_kb
        || value.sync_limit_kb > MAX_EMITTED_LIMIT_KB
        || !(0..=i64::from(crate::stamps::MAX_STAMP_COST)).contains(&value.stamp_cost)
        || !(0..=i64::from(crate::stamps::MAX_STAMP_COST)).contains(&value.stamp_cost_flexibility)
        || !(0..=i64::from(crate::stamps::MAX_STAMP_COST)).contains(&value.peering_cost)
    {
        return Err(AnnounceError::InvalidEmission);
    }
    if let Some(name) = &value.node_name {
        validate_node_name(name)?;
    }
    Ok(())
}

fn python_bool(value: &rmpv::Value) -> Result<bool, AnnounceError> {
    if let Some(value) = value.as_bool() {
        return Ok(value);
    }
    match value {
        rmpv::Value::Integer(value) if value.as_i64() == Some(0) || value.as_u64() == Some(0) => {
            Ok(false)
        }
        rmpv::Value::Integer(value) if value.as_i64() == Some(1) || value.as_u64() == Some(1) => {
            Ok(true)
        }
        rmpv::Value::F32(value) if *value == 0.0 => Ok(false),
        rmpv::Value::F32(value) if *value == 1.0 => Ok(true),
        rmpv::Value::F64(value) if *value == 0.0 => Ok(false),
        rmpv::Value::F64(value) if *value == 1.0 => Ok(true),
        _ => Err(AnnounceError::InvalidActive),
    }
}

fn python_int(value: &rmpv::Value) -> Result<i64, AnnounceError> {
    match value {
        rmpv::Value::Boolean(value) => Ok(i64::from(*value)),
        rmpv::Value::Integer(value) => value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
            .ok_or(AnnounceError::InvalidNumber),
        rmpv::Value::F32(value) => float_to_i64(f64::from(*value)),
        rmpv::Value::F64(value) => float_to_i64(*value),
        rmpv::Value::String(value) => {
            value.as_str().ok_or(AnnounceError::InvalidNumber).and_then(decimal_text_to_i64)
        }
        rmpv::Value::Binary(value) => core::str::from_utf8(value)
            .map_err(|_| AnnounceError::InvalidNumber)
            .and_then(decimal_text_to_i64),
        _ => Err(AnnounceError::InvalidNumber),
    }
}

fn float_to_i64(value: f64) -> Result<i64, AnnounceError> {
    const I64_UPPER_EXCLUSIVE: f64 = 9_223_372_036_854_775_808.0;
    const I64_LOWER_INCLUSIVE: f64 = -9_223_372_036_854_775_808.0;
    if !value.is_finite() {
        return Err(AnnounceError::InvalidNumber);
    }
    let truncated = value.trunc();
    if !(I64_LOWER_INCLUSIVE..I64_UPPER_EXCLUSIVE).contains(&truncated) {
        return Err(AnnounceError::InvalidNumber);
    }
    Ok(truncated as i64)
}

fn decimal_text_to_i64(value: &str) -> Result<i64, AnnounceError> {
    let value = value.trim();
    let digits = value.strip_prefix(['+', '-']).unwrap_or(value);
    if digits.is_empty() {
        return Err(AnnounceError::InvalidNumber);
    }
    let bytes = digits.as_bytes();
    if bytes.iter().enumerate().any(|(index, byte)| {
        if *byte == b'_' {
            index == 0
                || index + 1 == bytes.len()
                || !bytes[index - 1].is_ascii_digit()
                || !bytes[index + 1].is_ascii_digit()
        } else {
            !byte.is_ascii_digit()
        }
    }) {
        return Err(AnnounceError::InvalidNumber);
    }
    let normalized: String = value.chars().filter(|character| *character != '_').collect();
    normalized.parse().map_err(|_| AnnounceError::InvalidNumber)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canonical_value() -> rmpv::Value {
        rmp_serde::from_slice(
            &StandardPropagationAnnounce::inactive(1_700_000_000, Some("Node One"))
                .unwrap()
                .encode()
                .unwrap(),
        )
        .unwrap()
    }

    fn parse_value(fields: Vec<rmpv::Value>) -> StandardPropagationAnnounce {
        StandardPropagationAnnounce::parse(&rmp_serde::to_vec(&rmpv::Value::Array(fields)).unwrap())
            .unwrap()
    }

    #[test]
    fn emitted_shape_uses_exact_integer_binary_defaults_and_preserved_name() {
        let encoded = StandardPropagationAnnounce::inactive(1_700_000_000, Some("  Node One  "))
            .unwrap()
            .encode()
            .unwrap();
        let value: rmpv::Value = rmp_serde::from_slice(&encoded).unwrap();
        let fields = value.as_array().unwrap();
        assert_eq!(fields.len(), 7);
        assert_eq!(fields[0], rmpv::Value::Boolean(false));
        assert_eq!(fields[1].as_i64(), Some(1_700_000_000));
        assert_eq!(fields[2], rmpv::Value::Boolean(false));
        assert_eq!(fields[3].as_i64(), Some(256));
        assert_eq!(fields[4].as_i64(), Some(10_240));
        assert_eq!(
            fields[5].as_array().unwrap().iter().map(rmpv::Value::as_i64).collect::<Vec<_>>(),
            vec![Some(16), Some(3), Some(18)]
        );
        assert_eq!(
            fields[6].as_map().unwrap(),
            &[(rmpv::Value::from(1), rmpv::Value::Binary(b"  Node One  ".to_vec()))]
        );
    }

    #[test]
    fn configured_name_rejections_are_explicit_and_value_preserving() {
        assert_eq!(
            StandardPropagationAnnounce::inactive(1, Some("")),
            Err(AnnounceError::EmptyName)
        );
        assert_eq!(
            StandardPropagationAnnounce::inactive(1, Some("bad\nname")),
            Err(AnnounceError::ControlCharacterName)
        );
        let oversized = "x".repeat(MAX_NODE_NAME_BYTES + 1);
        assert_eq!(
            StandardPropagationAnnounce::inactive(1, Some(&oversized)),
            Err(AnnounceError::NameTooLong)
        );
        let maximum = "x".repeat(MAX_NODE_NAME_BYTES);
        let encoded =
            StandardPropagationAnnounce::inactive(1, Some(&maximum)).unwrap().encode().unwrap();
        assert!(encoded.len() <= MAX_APP_DATA_BYTES);
    }

    #[test]
    fn parser_accepts_pinned_extensions_ignored_slot_zero_and_extra_costs() {
        let rmpv::Value::Array(mut fields) = canonical_value() else { panic!("array") };
        fields[0] = rmpv::Value::Map(vec![("ignored".into(), true.into())]);
        fields.push("bounded extension".into());
        fields[5].as_array().unwrap();
        let rmpv::Value::Array(costs) = &mut fields[5] else { panic!("costs") };
        costs.push(i64::MAX.into());
        let parsed = parse_value(fields);
        assert_eq!(parsed.transfer_limit_kb, 256);
        assert_eq!(parsed.peering_cost, 18);
    }

    #[test]
    fn parser_matches_bounded_python_int_coercions() {
        let rmpv::Value::Array(mut fields) = canonical_value() else { panic!("array") };
        fields[1] = rmpv::Value::F64(-42.9);
        fields[2] = rmpv::Value::F64(1.0);
        fields[3] = rmpv::Value::String("-2_048".into());
        fields[4] = rmpv::Value::F32(3.9);
        let rmpv::Value::Array(costs) = &mut fields[5] else { panic!("costs") };
        costs[0] = i64::MIN.into();
        costs[1] = i64::MAX.into();
        costs[2] = rmpv::Value::Binary(b"  +0  ".to_vec());
        let parsed = parse_value(fields);
        assert_eq!(parsed.emitted_at, -42);
        assert!(parsed.node_active);
        assert_eq!(parsed.transfer_limit_kb, -2048);
        assert_eq!(parsed.sync_limit_kb, 3);
        assert_eq!(parsed.stamp_cost, i64::MIN);
        assert_eq!(parsed.stamp_cost_flexibility, i64::MAX);
        assert_eq!(parsed.peering_cost, 0);
    }

    #[test]
    fn inbound_name_is_utf8_preserving_and_duplicate_key_uses_last_value() {
        let rmpv::Value::Array(mut fields) = canonical_value() else { panic!("array") };
        fields[6] = rmpv::Value::Map(vec![
            (1.into(), rmpv::Value::Binary(b"first".to_vec())),
            (1.into(), rmpv::Value::Binary(b"\n last \n".to_vec())),
        ]);
        assert_eq!(parse_value(fields).node_name.as_deref(), Some("\n last \n"));
    }

    #[test]
    fn invalid_optional_name_is_ignored_like_pinned_lxmf() {
        for invalid_name in [rmpv::Value::from("text"), rmpv::Value::Binary(vec![0xff])] {
            let rmpv::Value::Array(mut fields) = canonical_value() else { panic!("array") };
            fields[6] = rmpv::Value::Map(vec![(1.into(), invalid_name)]);
            assert_eq!(parse_value(fields).node_name, None);
        }
    }

    #[test]
    fn parser_accepts_zero_negative_and_large_values_but_emission_rejects_them() {
        let rmpv::Value::Array(mut fields) = canonical_value() else { panic!("array") };
        fields[1] = i64::MIN.into();
        fields[3] = 0.into();
        fields[4] = i64::MAX.into();
        let parsed = parse_value(fields);
        assert_eq!(parsed.emitted_at, i64::MIN);
        assert_eq!(parsed.transfer_limit_kb, 0);
        assert_eq!(parsed.sync_limit_kb, i64::MAX);
        assert_eq!(parsed.encode(), Err(AnnounceError::InvalidEmission));
    }

    #[test]
    fn parser_rejects_nonfinite_overflow_bad_active_and_malformed_shape() {
        for replacement in [
            rmpv::Value::F64(f64::NAN),
            rmpv::Value::F64(f64::INFINITY),
            rmpv::Value::String("9223372036854775808".into()),
            rmpv::Value::Binary(b"1.5".to_vec()),
        ] {
            let rmpv::Value::Array(mut fields) = canonical_value() else { panic!("array") };
            fields[3] = replacement;
            assert!(
                StandardPropagationAnnounce::parse(
                    &rmp_serde::to_vec(&rmpv::Value::Array(fields)).unwrap()
                )
                .is_err()
            );
        }
        let rmpv::Value::Array(mut fields) = canonical_value() else { panic!("array") };
        fields[2] = 2.into();
        assert_eq!(
            StandardPropagationAnnounce::parse(
                &rmp_serde::to_vec(&rmpv::Value::Array(fields)).unwrap()
            ),
            Err(AnnounceError::InvalidActive)
        );
        for invalid_active in [
            rmpv::Value::Binary(b"1".to_vec()),
            rmpv::Value::String("1".into()),
            rmpv::Value::F64(0.5),
        ] {
            let rmpv::Value::Array(mut fields) = canonical_value() else { panic!("array") };
            fields[2] = invalid_active;
            assert_eq!(
                StandardPropagationAnnounce::parse(
                    &rmp_serde::to_vec(&rmpv::Value::Array(fields)).unwrap()
                ),
                Err(AnnounceError::InvalidActive)
            );
        }
        for case in [vec![], vec![0xc1], rmp_serde::to_vec(&rmpv::Value::Array(vec![])).unwrap()] {
            assert!(StandardPropagationAnnounce::parse(&case).is_err());
        }
        for byte in 0_u8..=255 {
            let _ = StandardPropagationAnnounce::parse(&[byte]);
        }
    }
}
