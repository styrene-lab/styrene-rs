//! # styrene-native
//!
//! PyO3 bindings exposing styrene-rs crates to Python.
//!
//! This extension module is imported by the Python `styrened` daemon as a
//! drop-in replacement for pure-Python modules. When `styrene-native` is
//! not installed, `styrened` falls back to its own Python implementations.
//!
//! ## Usage from Python
//!
//! ```python
//! from styrene_native import StyreneMessage, StyreneMessageType
//!
//! msg = StyreneMessage(StyreneMessageType.Ping, b"")
//! encoded = msg.encode()
//! decoded = StyreneMessage.decode(encoded)
//! assert decoded.message_type == StyreneMessageType.Ping
//! ```

mod content;
mod wire;

use pyo3::prelude::*;

/// styrene_native — Rust-accelerated internals for styrened.
#[pymodule]
fn styrene_native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // Wire protocol types
    m.add_class::<wire::PyStyreneMessage>()?;
    m.add_class::<wire::PyStyreneMessageType>()?;

    // Content distribution types
    m.add_class::<content::PyContentId>()?;
    m.add_class::<content::PyChunkProfile>()?;
    m.add_class::<content::PyStyreneManifest>()?;
    m.add_class::<content::PyResourceAvailableAnnounce>()?;

    // Module metadata
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add("WIRE_VERSION", styrene_mesh::WIRE_VERSION)?;
    m.add("NAMESPACE", styrene_mesh::NAMESPACE.as_slice())?;

    Ok(())
}
