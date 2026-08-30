use std::collections::HashSet;
use std::io::Cursor;

use rmpv::Value;

use crate::hash::{ADDRESS_HASH_SIZE, AddressHash};
use crate::transport::error::RnsError;
use crate::transport::iface::{InterfaceKind, InterfaceSnapshot};

pub const DISCOVERY_APP_NAME: &str = "rnstransport";
pub const DISCOVERY_ASPECTS: &str = "discovery.interface";
pub const IMPLEMENTATION_NAME: &str = "Styrene";
pub const IMPLEMENTATION_VERSION: &str = env!("CARGO_PKG_VERSION");

const NAME: u64 = 0xff;
const TRANSPORT_ID: u64 = 0xfe;
const TRANSPORT_IMPL: u64 = 0xfd;
const TRANSPORT_VERSION: u64 = 0xfc;
const OPERATOR_ADDRESS: u64 = 0xf0;
const INTERFACE_TYPE: u64 = 0x00;
const TRANSPORT: u64 = 0x01;
const REACHABLE_ON: u64 = 0x02;
const LATITUDE: u64 = 0x03;
const LONGITUDE: u64 = 0x04;
const HEIGHT: u64 = 0x05;
const PORT: u64 = 0x06;

const DISCOVERABLE_TYPES: [&str; 7] = [
    "BackboneInterface",
    "TCPServerInterface",
    "TCPClientInterface",
    "RNodeInterface",
    "WeaveInterface",
    "I2PInterface",
    "KISSInterface",
];

#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceDiscoveryMetadata {
    pub interface_type: String,
    pub transport: bool,
    pub transport_id: AddressHash,
    pub implementation: String,
    pub version: String,
    pub name: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub height: Option<f64>,
    pub reachable_on: Option<String>,
    pub port: Option<u16>,
    pub operator_lxmf_address: Option<AddressHash>,
}

impl InterfaceDiscoveryMetadata {
    pub fn from_snapshot(
        snapshot: &InterfaceSnapshot,
        transport: bool,
        transport_id: AddressHash,
        name: &str,
        operator_lxmf_address: Option<AddressHash>,
    ) -> Option<Self> {
        let interface_type = match snapshot.kind {
            InterfaceKind::TcpServer => "TCPServerInterface",
            InterfaceKind::TcpClient
            | InterfaceKind::Udp
            | InterfaceKind::Serial
            | InterfaceKind::Kiss
            | InterfaceKind::Unknown => return None,
        };
        let (reachable_on, port) = match snapshot.local_endpoint.as_ref() {
            Some(crate::transport::iface::InterfaceEndpoint::Socket(address))
                if snapshot.kind == InterfaceKind::TcpServer =>
            {
                (Some(address.ip().to_string()), Some(address.port()))
            }
            _ => (None, None),
        };
        Some(Self {
            interface_type: interface_type.to_string(),
            transport,
            transport_id,
            implementation: IMPLEMENTATION_NAME.to_string(),
            version: IMPLEMENTATION_VERSION.to_string(),
            name: sanitize_name(name).unwrap_or_else(|| format!("Discovered {interface_type}")),
            latitude: None,
            longitude: None,
            height: None,
            reachable_on,
            port,
            operator_lxmf_address,
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>, RnsError> {
        if !DISCOVERABLE_TYPES.contains(&self.interface_type.as_str())
            || self.implementation.is_empty()
            || self.version.is_empty()
        {
            return Err(RnsError::InvalidArgument);
        }
        let name = sanitize_name(&self.name).ok_or(RnsError::InvalidArgument)?;
        let mut fields = vec![
            field(INTERFACE_TYPE, Value::from(self.interface_type.clone())),
            field(TRANSPORT, Value::Boolean(self.transport)),
            field(TRANSPORT_ID, Value::Binary(self.transport_id.as_slice().to_vec())),
            field(TRANSPORT_IMPL, Value::from(self.implementation.clone())),
            field(TRANSPORT_VERSION, Value::from(self.version.clone())),
            field(NAME, Value::from(name)),
            field(LATITUDE, optional_float(self.latitude)),
            field(LONGITUDE, optional_float(self.longitude)),
            field(HEIGHT, optional_float(self.height)),
        ];
        if let Some(address) = self.operator_lxmf_address {
            fields.push(field(OPERATOR_ADDRESS, Value::Binary(address.as_slice().to_vec())));
        }
        if let Some(reachable_on) = self.reachable_on.as_ref() {
            fields.push(field(REACHABLE_ON, Value::from(reachable_on.clone())));
        }
        if let Some(port) = self.port {
            fields.push(field(PORT, Value::from(port)));
        }
        let mut encoded = Vec::new();
        rmpv::encode::write_value(&mut encoded, &Value::Map(fields))
            .map_err(|_| RnsError::PacketError)?;
        Ok(encoded)
    }

    pub fn decode(encoded: &[u8]) -> Result<Self, RnsError> {
        let mut cursor = Cursor::new(encoded);
        let Value::Map(fields) =
            rmpv::decode::read_value(&mut cursor).map_err(|_| RnsError::PacketError)?
        else {
            return Err(RnsError::PacketError);
        };
        if cursor.position() != encoded.len() as u64 {
            return Err(RnsError::PacketError);
        }
        let mut seen = HashSet::new();
        let mut get = |key: u64| -> Result<Option<&Value>, RnsError> {
            let mut values = fields.iter().filter_map(|(candidate, value)| {
                (candidate.as_u64() == Some(key)).then_some(value)
            });
            let value = values.next();
            if values.next().is_some() || (value.is_some() && !seen.insert(key)) {
                return Err(RnsError::PacketError);
            }
            Ok(value)
        };
        let interface_type = string(get(INTERFACE_TYPE)?.ok_or(RnsError::PacketError)?)?;
        if !DISCOVERABLE_TYPES.contains(&interface_type.as_str()) {
            return Err(RnsError::PacketError);
        }
        let transport = get(TRANSPORT)?.and_then(Value::as_bool).ok_or(RnsError::PacketError)?;
        let transport_id = address(get(TRANSPORT_ID)?.ok_or(RnsError::PacketError)?)?;
        let implementation = string(get(TRANSPORT_IMPL)?.ok_or(RnsError::PacketError)?)?;
        let version = string(get(TRANSPORT_VERSION)?.ok_or(RnsError::PacketError)?)?;
        if implementation.is_empty() || version.is_empty() {
            return Err(RnsError::PacketError);
        }
        let raw_name = string(get(NAME)?.ok_or(RnsError::PacketError)?)?;
        let name = sanitize_name(&raw_name).ok_or(RnsError::PacketError)?;
        let latitude = decode_optional_float(get(LATITUDE)?)?;
        let longitude = decode_optional_float(get(LONGITUDE)?)?;
        let height = decode_optional_float(get(HEIGHT)?)?;
        let reachable_on = get(REACHABLE_ON)?.map(string).transpose()?;
        let port = match get(PORT)? {
            Some(value) => Some(
                value
                    .as_u64()
                    .and_then(|port| u16::try_from(port).ok())
                    .ok_or(RnsError::PacketError)?,
            ),
            None => None,
        };
        let operator_lxmf_address = get(OPERATOR_ADDRESS)?
            .map(|value| match value {
                Value::Nil => Ok(None),
                _ => address(value).map(Some),
            })
            .transpose()?
            .flatten();
        Ok(Self {
            interface_type,
            transport,
            transport_id,
            implementation,
            version,
            name,
            latitude,
            longitude,
            height,
            reachable_on,
            port,
            operator_lxmf_address,
        })
    }
}

pub fn sanitize_name(name: &str) -> Option<String> {
    let sanitized = name
        .chars()
        .filter(|character| character.is_ascii() && !character.is_ascii_control())
        .collect::<String>()
        .split_ascii_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!sanitized.is_empty()).then_some(sanitized)
}

fn field(key: u64, value: Value) -> (Value, Value) {
    (Value::from(key), value)
}

fn optional_float(value: Option<f64>) -> Value {
    value.map(Value::F64).unwrap_or(Value::Nil)
}

fn decode_optional_float(value: Option<&Value>) -> Result<Option<f64>, RnsError> {
    match value {
        None | Some(Value::Nil) => Ok(None),
        Some(Value::F32(value)) => Ok(Some(f64::from(*value))),
        Some(Value::F64(value)) => Ok(Some(*value)),
        Some(_) => Err(RnsError::PacketError),
    }
}

fn string(value: &Value) -> Result<String, RnsError> {
    value.as_str().map(str::to_string).ok_or(RnsError::PacketError)
}

fn address(value: &Value) -> Result<AddressHash, RnsError> {
    let Value::Binary(bytes) = value else { return Err(RnsError::PacketError) };
    let bytes: [u8; ADDRESS_HASH_SIZE] =
        bytes.as_slice().try_into().map_err(|_| RnsError::PacketError)?;
    Ok(AddressHash::new(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata() -> InterfaceDiscoveryMetadata {
        InterfaceDiscoveryMetadata {
            interface_type: "TCPServerInterface".to_string(),
            transport: true,
            transport_id: AddressHash::new([0x11; ADDRESS_HASH_SIZE]),
            implementation: IMPLEMENTATION_NAME.to_string(),
            version: IMPLEMENTATION_VERSION.to_string(),
            name: " Relay\r\n   One ".to_string(),
            latitude: None,
            longitude: None,
            height: None,
            reachable_on: Some("relay.example".to_string()),
            port: Some(4242),
            operator_lxmf_address: Some(AddressHash::new([0x22; ADDRESS_HASH_SIZE])),
        }
    }

    #[test]
    fn roundtrip_preserves_canonical_fields_and_sanitizes_name() {
        let decoded = InterfaceDiscoveryMetadata::decode(&metadata().encode().expect("metadata"))
            .expect("decoded metadata");
        assert_eq!(decoded.name, "Relay One");
        assert_eq!(decoded.implementation, "Styrene");
        assert_eq!(decoded.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(decoded.operator_lxmf_address, Some(AddressHash::new([0x22; 16])));
    }

    #[test]
    fn malformed_operator_address_and_field_types_fail_closed() {
        for value in [Value::from("not bytes"), Value::Binary(vec![0x22; 15])] {
            let mut fields = match rmpv::decode::read_value(&mut Cursor::new(
                metadata().encode().expect("metadata"),
            ))
            .expect("value")
            {
                Value::Map(fields) => fields,
                _ => unreachable!(),
            };
            let operator = fields
                .iter_mut()
                .find(|(key, _)| key.as_u64() == Some(OPERATOR_ADDRESS))
                .expect("operator field");
            operator.1 = value;
            let mut encoded = Vec::new();
            rmpv::encode::write_value(&mut encoded, &Value::Map(fields)).expect("encoded value");
            assert!(InterfaceDiscoveryMetadata::decode(&encoded).is_err());
        }
    }

    #[test]
    fn runtime_snapshot_emits_truthful_observational_metadata() {
        let mut snapshot = InterfaceSnapshot {
            hash: AddressHash::new([0x31; ADDRESS_HASH_SIZE]),
            kind: InterfaceKind::TcpServer,
            mode: crate::transport::iface::InterfaceMode::Gateway,
            state: crate::transport::iface::InterfaceState::Listening,
            local_endpoint: Some(crate::transport::iface::InterfaceEndpoint::Socket(
                "192.0.2.10:4242".parse().expect("socket"),
            )),
            remote_endpoint: None,
            parent: None,
            tx_bytes: 0,
            rx_bytes: 0,
            violations: Default::default(),
            filters: Default::default(),
            connected_peers: 0,
            generation: 1,
        };
        let metadata = InterfaceDiscoveryMetadata::from_snapshot(
            &snapshot,
            true,
            AddressHash::new([0x32; ADDRESS_HASH_SIZE]),
            " Gateway\n  One ",
            None,
        )
        .expect("discoverable TCP server");
        assert_eq!(metadata.interface_type, "TCPServerInterface");
        assert_eq!(metadata.name, "Gateway One");
        assert_eq!(metadata.reachable_on.as_deref(), Some("192.0.2.10"));
        assert_eq!(metadata.port, Some(4242));
        assert_eq!(metadata.implementation, IMPLEMENTATION_NAME);

        snapshot.kind = InterfaceKind::Udp;
        assert!(
            InterfaceDiscoveryMetadata::from_snapshot(
                &snapshot,
                true,
                AddressHash::new([0x32; ADDRESS_HASH_SIZE]),
                "UDP",
                None,
            )
            .is_none()
        );
    }
}
