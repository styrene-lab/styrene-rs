pub use lxmf_legacy::errors;
pub use lxmf_legacy::identity;
pub use lxmf_legacy::inbound_decode;
pub use lxmf_legacy::message;
pub use lxmf_legacy::payload_fields;
#[cfg(feature = "json-interop")]
pub use lxmf_legacy::wire_fields;

pub use lxmf_legacy::LxmfError;
pub use lxmf_legacy::{Message, Payload, WireMessage};
