//! Role-based access control for the Styrene mesh.
//!
//! Provides a hierarchical role model with fine-grained capabilities,
//! roster-based identity binding, and policy evaluation. Shared by
//! `styrened` (device-level RBAC) and `aether` (agent-to-agent RBAC).
//!
//! # Design
//!
//! - **Roles** are cumulative: each tier inherits all capabilities from
//!   tiers below it (PEER ⊂ MONITOR ⊂ OPERATOR ⊂ ADMIN).
//! - **Capabilities** are dot-separated strings (`chat.send`, `rpc.exec`).
//! - **Orthogonal grants** (e.g. `vpn.handshake`) sit outside the hierarchy
//!   and must be explicitly assigned regardless of role.
//! - **Policy evaluation** is pure — no I/O, no side effects. Takes a roster
//!   and an identity hash, returns allow/deny.
//!
//! # Operation-scoped authorization
//!
//! The [`authz`] module adds principals, exact and prefix operation grants
//! with deny precedence, role bundles, resource and context constraints,
//! trusted issuer extraction, structured decisions, and policy discovery.
//! Existing coarse roles remain available as data-backed bundles.
//!
//! # Features
//!
//! - `config` — enables serde deserialization from YAML/TOML/JSON config.

pub mod authz;
mod capability;
mod policy;
mod role;
pub mod signed;
mod warning;

pub use capability::{
    ADMIN_CAPS, ALL_CAPABILITIES, Capability, MONITOR_CAPS, OPERATOR_CAPS, PEER_CAPS,
    capabilities_for_role,
};
pub use policy::{MIN_BLOCKED_PREFIX_LEN, RbacPolicy, RosterEntry};
pub use role::Role;
pub use signed::{SignedRosterEntry, TrustedHub};
pub use warning::PolicyWarning;
