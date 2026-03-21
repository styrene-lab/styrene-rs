//! `styrene-entropy` — entropy accumulator, HMAC-DRBG, and source abstraction.
//!
//! # Architecture
//!
//! ```text
//! EntropySource(s)  →  EntropyPool (Fortuna-style accumulator)
//!                              │
//!                       seed / reseed
//!                              │
//!                           Drbg  (HMAC-DRBG / SHA-256)
//!                              │
//!                       fill_bytes(buf)   ← callers never see sources
//! ```
//!
//! The [`Drbg`] is the abstraction boundary. Everything above it calls
//! `drbg.fill_bytes()`. Nothing above it knows whether the seed came from a
//! hardware TRNG, the kernel, or the mesh Hub.
//!
//! # Feature flags
//!
//! | Feature | Enables |
//! |---|---|
//! | `kernel` *(default)* | [`source::KernelSource`] via `getrandom` |
//! | `jitter` | [`source::JitterSource`] — CPU timing jitter |
//! | `hardware-trng` | [`source::HardwareSource`] — nRF52840 UART coprocessor |
//! | `mesh-source` | [`source::MeshHubSource`] stub — ENTROPY_REQUEST RPC |

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used)]

pub mod drbg;
pub mod health;
pub mod pool;
pub mod source;

pub use drbg::Drbg;
pub use health::{HealthError, SourceHealth};
pub use pool::EntropyPool;
pub use source::EntropySource;
