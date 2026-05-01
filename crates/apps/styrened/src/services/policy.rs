//! PolicyService — thread-safe wrapper around styrene-rbac's RbacPolicy.
//!
//! Replaces AuthService as the single authorization check-point.
//! DaemonFacade and RpcRequestHandler call `has_capability()` before
//! delegating to business services.

use std::sync::RwLock;

use styrene_rbac::{RbacPolicy, Role, RosterEntry};

use crate::storage::messages::MessagesStore;

/// Thread-safe RBAC policy wrapper with persistence.
pub struct PolicyService {
    policy: RwLock<RbacPolicy>,
}

impl PolicyService {
    pub fn new(policy: RbacPolicy) -> Self {
        Self { policy: RwLock::new(policy) }
    }

    /// Check whether an identity holds a specific capability.
    pub fn has_capability(&self, identity_hash: &str, cap: &str) -> bool {
        self.policy.read().unwrap().has_capability(identity_hash, cap)
    }

    /// Resolve the effective role for an identity.
    pub fn resolve_role(&self, identity_hash: &str) -> Role {
        self.policy.read().unwrap().resolve_role(identity_hash)
    }

    /// Check whether an identity is blocked.
    pub fn is_blocked(&self, identity_hash: &str) -> bool {
        self.resolve_role(identity_hash) == Role::Blocked
    }

    /// Grant a role to an identity and persist to SQLite.
    ///
    /// Persists to DB first — if the write fails, in-memory state is unchanged.
    pub fn grant(
        &self,
        entry: RosterEntry,
        store: &std::sync::Mutex<MessagesStore>,
    ) -> Result<(), String> {
        // Validate before touching either DB or memory.
        let normalized = entry.identity_hash.to_ascii_lowercase();
        if normalized.len() != 32 || !normalized.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("invalid identity hash".into());
        }
        // Persist first — fail before mutating in-memory state.
        {
            let store = store.lock().unwrap();
            store.upsert_rbac_entry(&entry).map_err(|e| e.to_string())?;
        }
        // DB succeeded — update in-memory (infallible for valid hashes).
        let mut policy = self.policy.write().unwrap();
        policy.add_entry(entry);
        Ok(())
    }

    /// Revoke a role assignment and remove from SQLite.
    ///
    /// Persists to DB first — if the write fails, in-memory state is unchanged.
    pub fn revoke(
        &self,
        identity_hash: &str,
        store: &std::sync::Mutex<MessagesStore>,
    ) -> Result<bool, String> {
        // Persist first.
        let db_removed = {
            let store = store.lock().unwrap();
            store.remove_rbac_entry(identity_hash).map_err(|e| e.to_string())?
        };
        if db_removed {
            let mut policy = self.policy.write().unwrap();
            policy.remove_entry(identity_hash);
        }
        Ok(db_removed)
    }

    /// Block an identity prefix and persist to the blocked_peers table.
    ///
    /// Persists to DB first — if the write fails, in-memory state is unchanged.
    pub fn block(
        &self,
        prefix: &str,
        store: &std::sync::Mutex<MessagesStore>,
    ) -> Result<bool, String> {
        // Persist first.
        let db_added = {
            let store = store.lock().unwrap();
            store.block_peer(prefix).map_err(|e| e.to_string())?
        };
        // Update in-memory regardless — block() is idempotent.
        let mut policy = self.policy.write().unwrap();
        policy.block(prefix);
        Ok(db_added)
    }

    /// Unblock an identity prefix and remove from the blocked_peers table.
    ///
    /// Persists to DB first — if the write fails, in-memory state is unchanged.
    pub fn unblock(
        &self,
        prefix: &str,
        store: &std::sync::Mutex<MessagesStore>,
    ) -> Result<bool, String> {
        // Persist first.
        let db_removed = {
            let store = store.lock().unwrap();
            store.unblock_peer(prefix).map_err(|e| e.to_string())?
        };
        if db_removed {
            let mut policy = self.policy.write().unwrap();
            policy.unblock(prefix);
        }
        Ok(db_removed)
    }

    /// List all roster entries (cloned snapshot).
    pub fn list_roster(&self) -> Vec<RosterEntry> {
        self.policy.read().unwrap().entries().to_vec()
    }

    /// Clone the entire policy (for serialization / debug).
    pub fn policy_snapshot(&self) -> RbacPolicy {
        self.policy.read().unwrap().clone()
    }
}

impl Default for PolicyService {
    fn default() -> Self {
        Self::new(RbacPolicy::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use styrene_rbac::Capability;

    #[test]
    fn default_peer_can_chat() {
        let svc = PolicyService::default();
        assert!(svc.has_capability("aaaa1111bbbb2222cccc3333dddd4444", Capability::CHAT_SEND));
    }

    #[test]
    fn default_peer_cannot_exec() {
        let svc = PolicyService::default();
        assert!(!svc.has_capability("aaaa1111bbbb2222cccc3333dddd4444", Capability::RPC_EXEC));
    }

    #[test]
    fn admin_can_exec() {
        let mut policy = RbacPolicy::default();
        policy.add_entry(RosterEntry::new("aaaa1111bbbb2222cccc3333dddd4444", Role::Admin));
        let svc = PolicyService::new(policy);
        assert!(svc.has_capability("aaaa1111bbbb2222cccc3333dddd4444", Capability::RPC_EXEC));
    }

    #[test]
    fn blocked_has_no_access() {
        let mut policy = RbacPolicy::default();
        policy.block("deadbeef");
        let svc = PolicyService::new(policy);
        assert!(svc.is_blocked("deadbeef11112222333344445555aaaa"));
        assert!(!svc.has_capability("deadbeef11112222333344445555aaaa", Capability::CHAT_SEND));
    }
}
