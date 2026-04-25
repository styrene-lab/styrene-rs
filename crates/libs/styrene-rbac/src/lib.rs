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
//! # Features
//!
//! - `config` — enables serde deserialization from YAML/TOML/JSON config.

mod capability;
mod policy;
mod role;
mod warning;

pub use capability::{Capability, ADMIN_CAPS, MONITOR_CAPS, OPERATOR_CAPS, PEER_CAPS};
pub use policy::{RbacPolicy, RosterEntry, MIN_BLOCKED_PREFIX_LEN};
pub use role::Role;
pub use warning::PolicyWarning;
