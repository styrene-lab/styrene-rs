//! AuthService — RBAC policy and blocklist management.
//!
//! Owns: 4.3 RBAC policy+roster, 4.4 blocklist.
//! Exposes check(identity, capability) and is_blocked(identity).
//! Package: E
//!
//! AuthService owns policy data and check methods. DaemonFacade (Package I)
//! owns enforcement — it calls auth.check() before delegating to services.
//! Services trust their caller.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

/// RBAC capability — what a peer is allowed to do.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Capability {
    /// Can send chat messages
    Chat,
    /// Can execute remote commands
    Exec,
    /// Can trigger reboot
    Reboot,
    /// Can update config
    UpdateConfig,
    /// Can request status
    Status,
    /// Custom capability string
    Custom(String),
}

/// RBAC role level — higher includes lower.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Blocked = 0,
    #[default]
    Peer = 1,
    Operator = 2,
    Admin = 3,
}

/// Service managing RBAC policy and peer blocklist.
#[derive(Default)]
pub struct AuthService {
    /// Identity hash → role mapping.
    roster: Mutex<HashMap<String, Role>>,
    /// Blocked identity hashes.
    blocked: Mutex<HashSet<String>>,
    /// Default role for unknown peers.
    default_role: Mutex<Role>,
}

impl AuthService {
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if an identity has a specific capability.
    ///
    /// Called by DaemonFacade before delegating to services.
    pub fn check(&self, identity_hash: &str, capability: &Capability) -> bool {
        if self.is_blocked(identity_hash) {
            return false;
        }
        let role = self.role_for(identity_hash);
        Self::role_has_capability(role, capability)
    }

    /// Check if an identity is explicitly blocked.
    ///
    /// Also used by MessagingService for inbound message filtering
    /// (a data query, not an enforcement gate).
    pub fn is_blocked(&self, identity_hash: &str) -> bool {
        self.blocked.lock().unwrap().contains(identity_hash)
    }

    /// Get the role for an identity (falls back to default role).
    pub fn role_for(&self, identity_hash: &str) -> Role {
        self.roster
            .lock()
            .unwrap()
            .get(identity_hash)
            .copied()
            .unwrap_or_else(|| *self.default_role.lock().unwrap())
    }

    /// Set the role for an identity.
    pub fn set_role(&self, identity_hash: &str, role: Role) {
        self.roster
            .lock()
            .unwrap()
            .insert(identity_hash.to_string(), role);
    }

    /// Block an identity.
    pub fn block(&self, identity_hash: &str) {
        self.blocked
            .lock()
            .unwrap()
            .insert(identity_hash.to_string());
    }

    /// Unblock an identity.
    pub fn unblock(&self, identity_hash: &str) {
        self.blocked.lock().unwrap().remove(identity_hash);
    }

    /// Set the default role for unknown peers.
    pub fn set_default_role(&self, role: Role) {
        *self.default_role.lock().unwrap() = role;
    }

    /// Check if a role grants a capability.
    fn role_has_capability(role: Role, capability: &Capability) -> bool {
        match capability {
            Capability::Status | Capability::Chat => role >= Role::Peer,
            Capability::Exec | Capability::Reboot | Capability::UpdateConfig => {
                role >= Role::Operator
            }
            Capability::Custom(_) => role >= Role::Admin,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_peer_gets_default_role() {
        let svc = AuthService::new();
        assert_eq!(svc.role_for("unknown"), Role::Peer);
    }

    #[test]
    fn peer_can_chat_and_status() {
        let svc = AuthService::new();
        assert!(svc.check("peer1", &Capability::Chat));
        assert!(svc.check("peer1", &Capability::Status));
    }

    #[test]
    fn peer_cannot_exec() {
        let svc = AuthService::new();
        assert!(!svc.check("peer1", &Capability::Exec));
    }

    #[test]
    fn operator_can_exec() {
        let svc = AuthService::new();
        svc.set_role("op1", Role::Operator);
        assert!(svc.check("op1", &Capability::Exec));
        assert!(svc.check("op1", &Capability::Chat));
    }

    #[test]
    fn blocked_peer_cannot_do_anything() {
        let svc = AuthService::new();
        svc.set_role("bad", Role::Admin);
        svc.block("bad");
        assert!(!svc.check("bad", &Capability::Chat));
        assert!(!svc.check("bad", &Capability::Status));
    }

    #[test]
    fn unblock_restores_access() {
        let svc = AuthService::new();
        svc.block("peer");
        assert!(svc.is_blocked("peer"));
        svc.unblock("peer");
        assert!(!svc.is_blocked("peer"));
        assert!(svc.check("peer", &Capability::Chat));
    }

    #[test]
    fn set_default_role_affects_unknown_peers() {
        let svc = AuthService::new();
        svc.set_default_role(Role::Blocked);
        assert!(!svc.check("anyone", &Capability::Chat));
    }
}
