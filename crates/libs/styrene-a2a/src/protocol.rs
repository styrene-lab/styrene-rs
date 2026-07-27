use serde::{Deserialize, Serialize};

use crate::RuntimeId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcceptanceDisposition {
    Accepted,
    Duplicate,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptanceReceipt {
    pub message_id: [u8; 16],
    pub runtime_id: RuntimeId,
    pub disposition: AcceptanceDisposition,
    pub accepted_at_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolErrorCode {
    InvalidEnvelope,
    AuthenticationFailed,
    AuthorizationDenied,
    Expired,
    ReplayConflict,
    UnsupportedVersion,
    UnsupportedSchema,
    Internal,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolError {
    pub code: ProtocolErrorCode,
    pub message: String,
    pub retryable: bool,
    pub message_id: Option<[u8; 16]>,
}
