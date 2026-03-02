# Incremental Rust Migration via PyO3 FFI

**Date**: 2026-03-02
**Status**: Proposed
**Supersedes**: "No FFI, no PyO3 bindings" stance in CLAUDE.md

## Problem

The current architecture positions `styrened` (Python) and `styrene-rs` (Rust) as **two independent daemons** that communicate over LXMF. This means:

1. **Big bang cutover**: You ship the Rust daemon or you don't. No middle ground.
2. **No incremental validation**: Can't replace `node_store.py` with Rust and run it inside the Python daemon to verify identical behavior under real traffic.
3. **Dual maintenance forever**: Every feature goes into both codebases, or the Rust port falls behind. The parity gaps doc already shows the Rust side missing ~12K LOC of service layer.
4. **Wire-only compatibility is insufficient**: Two implementations can agree on wire format while disagreeing on state machine behavior, timing, error handling, retry logic — all the stuff that makes a mesh network actually work.

The wire protocol is a necessary interop contract, but it's not a migration strategy.

## Proposed Architecture: PyO3 Hybrid Daemon

Replace Python service modules **one at a time** with Rust implementations exposed via PyO3. The Python daemon remains the orchestrator — it imports Rust modules where available and falls back to Python implementations where not.

```
styrened (Python daemon — production)
├── services/
│   ├── node_store.py          → replaced by styrene_native.node_store  (Rust via PyO3)
│   ├── reticulum.py           → replaced by styrene_native.reticulum   (Rust via PyO3)
│   ├── lxmf_service.py        → stays Python (heavy RNS library coupling)
│   ├── conversation_service.py → replaced by styrene_native.conversations (Rust via PyO3)
│   ├── auto_reply.py          → stays Python (simple, low-perf)
│   ├── config.py              → stays Python (YAML, no hot path)
│   └── ...
├── models/
│   ├── styrene_wire.py        → replaced by styrene_native.wire (Rust via PyO3)
│   └── mesh_device.py         → replaced by styrene_native.mesh_device (Rust via PyO3)
├── protocols/
│   └── registry.py            → stays Python (dispatch glue)
└── tui/                        → stays Python (Textual, no Rust benefit)

styrene-native (PyO3 extension module, built with maturin)
├── Cargo.toml
├── pyproject.toml
└── src/
    └── lib.rs                  → Re-exports from styrene-rs crates
```

### How It Works

```python
# styrened/services/node_store.py

try:
    from styrene_native.node_store import NodeStore  # Rust
except ImportError:
    from styrened.services._node_store_py import NodeStore  # Python fallback

# Rest of the module uses NodeStore identically
```

The Python fallback means:
- **pip install styrened** — pure Python, works everywhere, no compiler needed
- **pip install styrened[native]** — pulls in `styrene-native` wheel, Rust hot paths activate
- Gradual: each module migrates independently on its own timeline
- Testable: run the full test suite with Rust module X enabled, compare results to pure Python

## Migration Order

Ordered by **impact × feasibility** — highest-value, lowest-coupling modules first.

### Phase 1: Data Layer (no RNS coupling)

| Module | Python LOC | Why first | Rust crate source |
|--------|-----------|-----------|-------------------|
| `styrene_wire.py` | 1,124 | Wire protocol is the contract. Rust version already exists in `styrene-mesh`. Byte-for-byte parity is trivially verifiable. | `styrene-mesh` |
| `node_store.py` | 986 | Pure SQLite + dataclasses. No RNS imports. Self-contained. Rust version is faster and gives the daemon a real database layer. | New: `styrene-native/src/node_store.rs` |
| `mesh_device.py` | 468 | Data model. No I/O. Direct port. | `styrene-mesh` types |
| `conversation_service.py` | 1,316 | SQLite queries + threading logic. No RNS. CPU-bound message search benefits from Rust. | New module |

**Validation**: Run `just test-unit` with Rust modules enabled. Every existing Python test must pass unchanged. This is the proof that the Rust module is a drop-in replacement.

### Phase 2: Crypto & Protocol

| Module | Python LOC | Notes |
|--------|-----------|-------|
| `pqc_session.py` | 424 | ML-KEM + X25519 hybrid. `styrene-rns` already has the crypto primitives. PyO3 exposes them. |
| `file_transfer.py` | 637 | Chunked transfer state machine. Benefits from Rust's zero-copy buffer management. |
| `rns_service.py` | 653 | RNS destination cache. Depends on how tightly it calls into the `RNS` Python library. May need an adapter. |

### Phase 3: RNS Integration Layer

| Module | Python LOC | Notes |
|--------|-----------|-------|
| `reticulum.py` | 1,407 | Deepest RNS coupling. Announce handling, path table lookups, interface management. This is the hardest module to port because it's tightly bound to `RNS.Transport` Python singletons. |
| `lxmf_service.py` | 1,114 | LXMF router. Same issue — deep coupling to `LXMF.LXMRouter` Python class. |

Phase 3 is where you'd evaluate whether to keep using Python RNS or switch to the Rust RNS transport (`styrene-rns`). That's a bigger decision — it means the daemon's network stack itself is Rust.

### Never Port

| Module | Why |
|--------|-----|
| `tui/` (all) | Textual is Python-native. No Rust benefit. If a Rust TUI is wanted, it's Ratatui in `styrened-rs`, not PyO3. |
| `config.py` | YAML parsing, file I/O, validation. Python is fine. Not a hot path. |
| `doctor.py` | Diagnostic tool. Runs once. Python is fine. |
| `cli.py` | Click/Typer CLI. Python is the right tool. |
| `auto_reply.py` | Simple pattern matching. 634 LOC. Not worth the complexity. |

## PyO3 Integration Pattern

### Crate Structure

```toml
# crates/bindings/styrene-native/Cargo.toml
[package]
name = "styrene-native"
version = "0.1.0"

[lib]
name = "styrene_native"
crate-type = ["cdylib"]

[dependencies]
styrene-mesh = { path = "../../libs/styrene-mesh" }
styrene-rns = { path = "../../libs/styrene-rns" }
styrene-lxmf = { path = "../../libs/styrene-lxmf" }
pyo3 = { version = "0.23", features = ["extension-module"] }
```

```toml
# crates/bindings/styrene-native/pyproject.toml
[build-system]
requires = ["maturin>=1.0"]
build-backend = "maturin"

[project]
name = "styrene-native"
requires-python = ">=3.11"

[tool.maturin]
features = ["pyo3/extension-module"]
```

### Example: Wire Protocol

```rust
// crates/bindings/styrene-native/src/wire.rs
use pyo3::prelude::*;
use styrene_mesh::wire::StyreneEnvelope;

#[pyclass]
#[derive(Clone)]
pub struct PyStyreneEnvelope {
    inner: StyreneEnvelope,
}

#[pymethods]
impl PyStyreneEnvelope {
    #[staticmethod]
    fn from_bytes(data: &[u8]) -> PyResult<Self> {
        let inner = StyreneEnvelope::from_bytes(data)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
        Ok(Self { inner })
    }

    fn to_bytes(&self) -> PyResult<Vec<u8>> {
        self.inner.to_bytes()
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))
    }

    #[getter]
    fn protocol(&self) -> &str {
        self.inner.protocol()
    }

    // ... expose all fields
}
```

### Example: Node Store

```rust
// crates/bindings/styrene-native/src/node_store.rs
use pyo3::prelude::*;

#[pyclass]
pub struct PyNodeStore {
    // rusqlite connection, same schema as Python's SQLAlchemy
    conn: rusqlite::Connection,
}

#[pymethods]
impl PyNodeStore {
    #[new]
    fn new(db_path: &str) -> PyResult<Self> { /* ... */ }

    fn upsert_node(&self, identity: &str, display_name: &str,
                   discovered_via: Option<&str>) -> PyResult<()> { /* ... */ }

    fn get_all_nodes(&self) -> PyResult<Vec<PyMeshDevice>> { /* ... */ }

    fn search(&self, query: &str) -> PyResult<Vec<PyMeshDevice>> { /* ... */ }
}
```

### Python Consumer

```python
# styrened/services/node_store.py
try:
    from styrene_native import PyNodeStore as NodeStore
    _NATIVE = True
except ImportError:
    from styrened.services._node_store_py import NodeStore
    _NATIVE = False

import logging
log = logging.getLogger(__name__)
if _NATIVE:
    log.info("Using native (Rust) NodeStore")
```

## Build & Distribution

```bash
# Development (editable install)
cd crates/bindings/styrene-native
maturin develop --release

# Build wheels for distribution
maturin build --release  # produces styrene_native-0.1.0-cp311-*.whl

# In styrened's pyproject.toml:
[project.optional-dependencies]
native = ["styrene-native>=0.1.0"]
```

Maturin handles cross-compilation and produces manylinux/macOS/Windows wheels. CI builds wheels for all targets. Users on platforms without a prebuilt wheel fall back to pure Python automatically.

## Compatibility Verification Strategy

The key advantage of FFI over wire-only interop: **you can run the same test suite against both implementations in the same process.**

```python
# tests/conftest.py
import pytest

@pytest.fixture(params=["python", "rust"])
def node_store(request, tmp_path):
    db = str(tmp_path / "test.db")
    if request.param == "rust":
        pytest.importorskip("styrene_native")
        from styrene_native import PyNodeStore
        return PyNodeStore(db)
    else:
        from styrened.services._node_store_py import NodeStore
        return NodeStore(db)
```

Every test runs twice — once with Python, once with Rust. Any behavioral divergence is caught immediately. This is impossible with the wire-only approach.

## CLAUDE.md Update

The `styrene-rs` CLAUDE.md should be updated:

```diff
- This is a **parallel implementation** alongside the Python `styrened` daemon.
- The wire protocol is the shared contract — no FFI, no PyO3 bindings.
- Both implementations communicate over LXMF like any two Reticulum nodes.
+ This is a **parallel implementation** alongside the Python `styrened` daemon,
+ with an incremental FFI migration path via PyO3.
+
+ Two integration modes:
+ 1. **Wire interop**: Independent Rust daemon communicates with Python daemon
+    over LXMF mesh (the `styrened-rs` binary).
+ 2. **FFI hybrid**: Rust modules exposed via PyO3 (`styrene-native` package),
+    imported by the Python daemon as drop-in replacements for Python modules.
+    Enables incremental migration with per-module compatibility verification.
```

## Risks

| Risk | Mitigation |
|------|-----------|
| PyO3 GIL contention on hot paths | Release GIL for CPU-bound work (`py.allow_threads`). SQLite calls, crypto, wire encode/decode all release GIL. |
| Async bridge complexity | Start with sync PyO3 functions. `pyo3-asyncio` only needed for Phase 3 (transport layer). Phase 1-2 modules are all synchronous. |
| Two codepaths in production | Feature flag `STYRENE_NATIVE=0` to force pure Python. CI matrix tests both paths. |
| Wheel build matrix explosion | Maturin + GitHub Actions / Argo builds wheels for linux-x86_64, linux-aarch64, macos-arm64. Three targets covers 99% of users. |
| SQLite schema drift | Single schema migration system (in Python). Rust `NodeStore` reads the same DB file, same schema. Migrations stay in Python. |

## Timeline Sketch

| Quarter | Milestone |
|---------|-----------|
| Q2 2026 | `styrene-native` crate scaffolded. Wire protocol (`StyreneEnvelope`) exposed via PyO3. Interop test vectors pass. |
| Q3 2026 | `NodeStore` + `MeshDevice` in Rust. Dual-path test fixture running in CI. |
| Q4 2026 | `ConversationService` + `FileTransfer` in Rust. Benchmark pure-Python vs hybrid daemon. |
| 2027 | Evaluate Phase 3 (RNS transport in Rust). Decision point: keep Python RNS or go full Rust network stack. |
