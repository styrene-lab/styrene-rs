//! AuthService — RBAC policy and blocklist management.
//!
//! Owns: 4.3 RBAC policy+roster, 4.4 blocklist. Exposes check(identity, capability) and is_blocked(identity).
//! Package: E

pub struct AuthService {
    // Fields will be added in Package E
}

impl AuthService {
    pub fn new() -> Self {
        Self {}
    }
}
