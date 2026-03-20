//! PyO3 bindings for the Styrene wire protocol.
//!
//! Wraps `styrene_mesh::wire::StyreneMessage` and `StyreneMessageType`
//! so Python can encode/decode wire-format messages using the exact same
//! Rust implementation that `styrened-rs` uses.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use styrene_mesh::wire::{StyreneMessage, StyreneMessageType, WireError};

/// Message type enum — mirrors Python's `StyreneMessageType`.
///
/// Exposed as `styrene_native.StyreneMessageType` with the same variant
/// names and integer values as the Python enum.
#[pyclass(name = "StyreneMessageType", eq, eq_int)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PyStyreneMessageType {
    // Control
    Ping = 0x01,
    Pong = 0x02,
    Heartbeat = 0x03,

    // Status
    StatusRequest = 0x10,
    StatusResponse = 0x11,

    // RPC Commands
    Exec = 0x40,
    Reboot = 0x41,
    ConfigUpdate = 0x42,

    // RPC Responses
    ExecResult = 0x60,
    RebootResult = 0x61,
    ConfigUpdateResult = 0x62,

    // Terminal Sessions
    TerminalRequest = 0xC0,
    TerminalAccept = 0xC1,
    TerminalData = 0xC2,
    TerminalResize = 0xC3,
    TerminalClose = 0xC4,
}

impl PyStyreneMessageType {
    fn to_rust(self) -> StyreneMessageType {
        match self {
            Self::Ping => StyreneMessageType::Ping,
            Self::Pong => StyreneMessageType::Pong,
            Self::Heartbeat => StyreneMessageType::Heartbeat,
            Self::StatusRequest => StyreneMessageType::StatusRequest,
            Self::StatusResponse => StyreneMessageType::StatusResponse,
            Self::Exec => StyreneMessageType::Exec,
            Self::Reboot => StyreneMessageType::Reboot,
            Self::ConfigUpdate => StyreneMessageType::ConfigUpdate,
            Self::ExecResult => StyreneMessageType::ExecResult,
            Self::RebootResult => StyreneMessageType::RebootResult,
            Self::ConfigUpdateResult => StyreneMessageType::ConfigUpdateResult,
            Self::TerminalRequest => StyreneMessageType::TerminalRequest,
            Self::TerminalAccept => StyreneMessageType::TerminalAccept,
            Self::TerminalData => StyreneMessageType::TerminalData,
            Self::TerminalResize => StyreneMessageType::TerminalResize,
            Self::TerminalClose => StyreneMessageType::TerminalClose,
        }
    }

    fn from_rust(t: StyreneMessageType) -> Self {
        match t {
            StyreneMessageType::Ping => Self::Ping,
            StyreneMessageType::Pong => Self::Pong,
            StyreneMessageType::Heartbeat => Self::Heartbeat,
            StyreneMessageType::StatusRequest => Self::StatusRequest,
            StyreneMessageType::StatusResponse => Self::StatusResponse,
            StyreneMessageType::Exec => Self::Exec,
            StyreneMessageType::Reboot => Self::Reboot,
            StyreneMessageType::ConfigUpdate => Self::ConfigUpdate,
            StyreneMessageType::ExecResult => Self::ExecResult,
            StyreneMessageType::RebootResult => Self::RebootResult,
            StyreneMessageType::ConfigUpdateResult => Self::ConfigUpdateResult,
            StyreneMessageType::TerminalRequest => Self::TerminalRequest,
            StyreneMessageType::TerminalAccept => Self::TerminalAccept,
            StyreneMessageType::TerminalData => Self::TerminalData,
            StyreneMessageType::TerminalResize => Self::TerminalResize,
            StyreneMessageType::TerminalClose => Self::TerminalClose,
            // PQC variants are feature-gated in styrene-mesh; if we don't
            // compile with pqc, this arm is unreachable. If we do, add
            // them to the enum above.
            #[allow(unreachable_patterns)]
            _ => Self::Ping, // fallback — should not happen
        }
    }
}

/// Wire protocol message — encode/decode Styrene envelope format.
///
/// This is the Rust implementation of Python's `StyreneEnvelope`.
/// Both produce byte-identical output for the same inputs.
///
/// ```python
/// from styrene_native import StyreneMessage, StyreneMessageType
///
/// # Create and encode
/// msg = StyreneMessage(StyreneMessageType.Ping, b"")
/// data = msg.encode()
///
/// # Decode
/// msg2 = StyreneMessage.decode(data)
/// assert msg2.message_type == StyreneMessageType.Ping
/// assert msg2.version == 1
///
/// # Access request ID for correlation
/// print(msg.request_id.hex())
/// ```
#[pyclass(name = "StyreneMessage")]
#[derive(Clone)]
pub struct PyStyreneMessage {
    inner: StyreneMessage,
}

#[pymethods]
impl PyStyreneMessage {
    /// Create a new message with a random request ID.
    #[new]
    fn new(message_type: PyStyreneMessageType, payload: &[u8]) -> Self {
        Self {
            inner: StyreneMessage::new(message_type.to_rust(), payload),
        }
    }

    /// Create a new message with a specific request ID (for responses).
    #[staticmethod]
    fn with_request_id(
        message_type: PyStyreneMessageType,
        request_id: &[u8],
        payload: &[u8],
    ) -> PyResult<Self> {
        let rid: [u8; 16] = request_id
            .try_into()
            .map_err(|_| PyValueError::new_err("request_id must be exactly 16 bytes"))?;
        Ok(Self {
            inner: StyreneMessage::with_request_id(message_type.to_rust(), rid, payload),
        })
    }

    /// Encode to wire format bytes.
    fn encode<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.encode())
    }

    /// Decode from wire format bytes.
    #[staticmethod]
    fn decode(data: &[u8]) -> PyResult<Self> {
        StyreneMessage::decode(data)
            .map(|inner| Self { inner })
            .map_err(wire_err_to_py)
    }

    /// Wire format version (currently 1).
    #[getter]
    fn version(&self) -> u8 {
        self.inner.version
    }

    /// Message type.
    #[getter]
    fn message_type(&self) -> PyStyreneMessageType {
        PyStyreneMessageType::from_rust(self.inner.message_type)
    }

    /// 16-byte request ID for request/response correlation.
    #[getter]
    fn request_id<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.request_id)
    }

    /// Raw payload bytes.
    #[getter]
    fn payload<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.payload)
    }

    fn __repr__(&self) -> String {
        format!(
            "StyreneMessage(type={:?}, request_id={}, payload_len={})",
            self.inner.message_type,
            hex::encode(self.inner.request_id),
            self.inner.payload.len(),
        )
    }
}

fn wire_err_to_py(e: WireError) -> PyErr {
    PyValueError::new_err(e.to_string())
}
