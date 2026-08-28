use crate::LxmfError;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use rmpv::Value;

pub const FIELD_ATTACHMENTS: i64 = 5;
pub const MAX_ATTACHMENT_COUNT: usize = 8;
pub const MAX_ATTACHMENT_BYTES: usize = 768 * 1024;
pub const MAX_ATTACHMENT_NAME_BYTES: usize = 255;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentFieldSource {
    CanonicalBinary,
    RustIntegerArray,
}

#[derive(Clone, PartialEq, Eq)]
pub struct AttachmentFieldEntry {
    pub filename: String,
    pub data: Vec<u8>,
    pub source: AttachmentFieldSource,
}

impl core::fmt::Debug for AttachmentFieldEntry {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AttachmentFieldEntry")
            .field("filename", &self.filename)
            .field("byte_len", &self.data.len())
            .field("source", &self.source)
            .finish()
    }
}

impl AttachmentFieldEntry {
    pub fn new(filename: String, data: Vec<u8>) -> Result<Self, LxmfError> {
        validate_name_and_size(&filename, data.len())?;
        Ok(Self { filename, data, source: AttachmentFieldSource::CanonicalBinary })
    }

    pub fn to_rmpv(&self) -> Value {
        Value::Array(vec![
            Value::String(self.filename.as_str().into()),
            Value::Binary(self.data.clone()),
        ])
    }
}

pub fn parse_attachment_field(
    fields: Option<&Value>,
) -> Result<Vec<AttachmentFieldEntry>, LxmfError> {
    let Some(entries) = fields.and_then(Value::as_map) else { return Ok(Vec::new()) };
    let Some((_, value)) = entries.iter().find(|(key, _)| key.as_i64() == Some(FIELD_ATTACHMENTS))
    else {
        return Ok(Vec::new());
    };
    let tuples = value
        .as_array()
        .ok_or_else(|| LxmfError::Decode("LXMF attachment field 5 must be an array".into()))?;
    if tuples.len() > MAX_ATTACHMENT_COUNT {
        return Err(LxmfError::Decode("LXMF attachment count exceeds 8".into()));
    }

    let mut result = Vec::with_capacity(tuples.len());
    let mut aggregate = 0usize;
    for tuple in tuples {
        let tuple = tuple.as_array().filter(|tuple| tuple.len() == 2).ok_or_else(|| {
            LxmfError::Decode("LXMF attachment entries must be two-element tuples".into())
        })?;
        let filename = match &tuple[0] {
            Value::String(value) => value
                .as_str()
                .map(ToString::to_string)
                .ok_or_else(|| LxmfError::Decode("LXMF attachment filename is not UTF-8".into()))?,
            Value::Binary(value) => core::str::from_utf8(value)
                .map(ToString::to_string)
                .map_err(|_| LxmfError::Decode("LXMF attachment filename is not UTF-8".into()))?,
            _ => {
                return Err(LxmfError::Decode(
                    "LXMF attachment filename must be text or binary".into(),
                ));
            }
        };
        let (data, source) = match &tuple[1] {
            Value::Binary(value) => (value.clone(), AttachmentFieldSource::CanonicalBinary),
            Value::Array(values) => {
                let mut bytes = Vec::with_capacity(values.len());
                for value in values {
                    let byte = value
                        .as_u64()
                        .and_then(|value| u8::try_from(value).ok())
                        .ok_or_else(|| {
                            LxmfError::Decode(
                                "LXMF compatibility attachment array contains a non-byte".into(),
                            )
                        })?;
                    bytes.push(byte);
                }
                (bytes, AttachmentFieldSource::RustIntegerArray)
            }
            _ => {
                return Err(LxmfError::Decode(
                    "LXMF attachment data must be MessagePack binary".into(),
                ));
            }
        };
        validate_name_and_size(&filename, data.len())?;
        aggregate = aggregate
            .checked_add(data.len())
            .ok_or_else(|| LxmfError::Decode("LXMF attachment aggregate size overflow".into()))?;
        if aggregate > MAX_ATTACHMENT_BYTES {
            return Err(LxmfError::Decode("LXMF attachment aggregate exceeds 768 KiB".into()));
        }
        result.push(AttachmentFieldEntry { filename, data, source });
    }
    Ok(result)
}

pub fn encode_attachment_field(entries: &[AttachmentFieldEntry]) -> Result<Value, LxmfError> {
    if entries.len() > MAX_ATTACHMENT_COUNT {
        return Err(LxmfError::Encode("LXMF attachment count exceeds 8".into()));
    }
    let mut aggregate = 0usize;
    let mut encoded = Vec::with_capacity(entries.len());
    for entry in entries {
        validate_name_and_size(&entry.filename, entry.data.len())?;
        aggregate = aggregate
            .checked_add(entry.data.len())
            .ok_or_else(|| LxmfError::Encode("LXMF attachment aggregate size overflow".into()))?;
        if aggregate > MAX_ATTACHMENT_BYTES {
            return Err(LxmfError::Encode("LXMF attachment aggregate exceeds 768 KiB".into()));
        }
        encoded.push(entry.to_rmpv());
    }
    Ok(Value::Array(encoded))
}

fn validate_name_and_size(filename: &str, byte_len: usize) -> Result<(), LxmfError> {
    if !(1..=MAX_ATTACHMENT_NAME_BYTES).contains(&filename.len()) {
        return Err(LxmfError::Decode(
            "LXMF attachment filename must be 1..=255 UTF-8 bytes".into(),
        ));
    }
    if byte_len > MAX_ATTACHMENT_BYTES {
        return Err(LxmfError::Decode("LXMF attachment exceeds 768 KiB".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_rmpv_encoding_is_binary_and_accepts_empty_bytes() {
        let field =
            encode_attachment_field(&[
                AttachmentFieldEntry::new("empty.bin".into(), vec![]).unwrap()
            ])
            .unwrap();
        assert!(
            matches!(&field.as_array().unwrap()[0].as_array().unwrap()[1], Value::Binary(value) if value.is_empty())
        );
        let fields = Value::Map(vec![(Value::from(FIELD_ATTACHMENTS), field)]);
        assert_eq!(
            parse_attachment_field(Some(&fields)).unwrap()[0].source,
            AttachmentFieldSource::CanonicalBinary
        );
    }

    #[test]
    fn rolling_integer_array_is_accepted_but_identified() {
        let fields = Value::Map(vec![(
            Value::from(FIELD_ATTACHMENTS),
            Value::Array(vec![Value::Array(vec![
                Value::from("a"),
                Value::Array(vec![Value::from(0), Value::from(255)]),
            ])]),
        )]);
        let parsed = parse_attachment_field(Some(&fields)).unwrap();
        assert_eq!(parsed[0].data, [0, 255]);
        assert_eq!(parsed[0].source, AttachmentFieldSource::RustIntegerArray);
    }

    #[test]
    fn duplicate_names_are_preserved_by_ordinal() {
        let duplicate = Value::Map(vec![(
            Value::from(5),
            Value::Array(vec![
                Value::Array(vec![Value::from("a"), Value::Binary(vec![])]),
                Value::Array(vec![Value::from("a"), Value::Binary(vec![1])]),
            ]),
        )]);
        let entries = parse_attachment_field(Some(&duplicate)).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].filename, entries[1].filename);
        assert_ne!(entries[0].data, entries[1].data);
    }

    #[test]
    fn rejects_non_tuple_entries() {
        let malformed = Value::Map(vec![(
            Value::from(5),
            Value::Array(vec![Value::Array(vec![Value::from("a")])]),
        )]);
        assert!(parse_attachment_field(Some(&malformed)).is_err());
    }
}
