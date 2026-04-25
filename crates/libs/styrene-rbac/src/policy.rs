//! RBAC policy evaluation — roster-based identity → role → capability checks.
//!
//! Pure evaluation logic with no I/O. Policy is constructed from a roster
//! (identity hash → role + grants) and a default role for unknown identities.

use crate::capability::{capabilities_for_role, is_valid_capability};
use crate::role::Role;

#[cfg(feature = "config")]
use serde::{Deserialize, Serialize};

/// Minimum length for blocked prefixes (4 bytes = 8 hex chars).
/// Shorter prefixes would block unacceptably large portions of the identity space.
pub const MIN_BLOCKED_PREFIX_LEN: usize = 8;

/// A single identity's role assignment with optional explicit grants.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "config", derive(Serialize, Deserialize))]
pub struct RosterEntry {
    /// 32-char hex identity hash (lowercase).
    #[cfg_attr(feature = "config", serde(alias = "identity"))]
    pub identity_hash: String,
    /// Assigned role tier.
    pub role: Role,
    /// Human-readable label (optional).
    #[cfg_attr(feature = "config", serde(default, skip_serializing_if = "String::is_empty"))]
    pub label: String,
    /// Explicit capability grants beyond what the role provides.
    /// Used for orthogonal capabilities like `vpn.handshake`.
    /// Private — only settable via `with_grants()` which validates, or
    /// via `add_entry()` which filters to known capabilities.
    #[cfg_attr(feature = "config", serde(default, skip_serializing_if = "Vec::is_empty"))]
    grants: Vec<String>,
}

impl RosterEntry {
    pub fn new(identity_hash: impl Into<String>, role: Role) -> Self {
        Self { identity_hash: identity_hash.into(), role, label: String::new(), grants: Vec::new() }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    pub fn with_grants(mut self, grants: Vec<String>) -> Self {
        self.grants = grants.into_iter().filter(|g| is_valid_capability(g)).collect();
        self
    }

    /// Read-only access to the grants list.
    pub fn grants(&self) -> &[String] {
        &self.grants
    }

    /// Check whether this entry holds a specific capability (via role or grant).
    pub fn has_capability(&self, cap: &str) -> bool {
        // Only honor grants that are known capabilities (defense in depth).
        capabilities_for_role(self.role).contains(&cap)
            || (is_valid_capability(cap) && self.grants.iter().any(|g| g == cap))
    }
}

/// Validate that a string is a valid hex identity hash (32 hex chars).
fn is_valid_identity_hash(hash: &str) -> bool {
    hash.len() == 32 && hash.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Validate that a string is a valid blocked prefix (>= MIN_BLOCKED_PREFIX_LEN hex chars).
fn is_valid_blocked_prefix(prefix: &str) -> bool {
    prefix.len() >= MIN_BLOCKED_PREFIX_LEN && prefix.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Central authorization policy. Resolves identities → roles → capabilities.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "config", derive(Serialize, Deserialize))]
pub struct RbacPolicy {
    /// Role assigned to identities not in the roster.
    #[cfg_attr(feature = "config", serde(default = "default_role"))]
    pub default_role: Role,

    /// Explicit identity → role mappings.
    #[cfg_attr(feature = "config", serde(default))]
    roster: Vec<RosterEntry>,

    /// Blocked identity hash prefixes. Any identity whose hash starts
    /// with one of these prefixes is treated as `Role::Blocked`.
    #[cfg_attr(feature = "config", serde(default))]
    blocked: Vec<String>,
}

#[allow(dead_code)] // Referenced by serde(default) when config feature enabled
fn default_role() -> Role {
    Role::Peer
}

impl Default for RbacPolicy {
    fn default() -> Self {
        Self { default_role: Role::Peer, roster: Vec::new(), blocked: Vec::new() }
    }
}

impl RbacPolicy {
    pub fn new(default_role: Role) -> Self {
        Self { default_role, roster: Vec::new(), blocked: Vec::new() }
    }

    /// Normalize and validate a deserialized policy, reporting every issue.
    ///
    /// Call this after deserializing from config. Returns a list of warnings
    /// for every entry that was dropped, filtered, or normalized. Callers
    /// should log these warnings so operators can fix their config.
    ///
    /// For a silent version (testing only), use `normalize_quiet()`.
    pub fn normalize(&mut self) -> Vec<crate::PolicyWarning> {
        use crate::PolicyWarning;

        let mut warnings = Vec::new();

        // Normalize and validate roster entries.
        for entry in &mut self.roster {
            let original = entry.identity_hash.clone();
            entry.identity_hash = entry.identity_hash.to_ascii_lowercase();
            if original != entry.identity_hash {
                warnings.push(PolicyWarning::NormalizedIdentityHash {
                    original,
                    normalized: entry.identity_hash.clone(),
                });
            }

            let grants_before: Vec<String> = entry.grants.clone();
            entry.grants.retain(|g| is_valid_capability(g));
            for g in &grants_before {
                if !entry.grants.contains(g) {
                    warnings.push(PolicyWarning::UnknownGrant {
                        identity_hash: entry.identity_hash.clone(),
                        grant: g.clone(),
                    });
                }
            }
        }

        // Report and drop invalid identity hashes, then deduplicate (last wins).
        let roster_before = std::mem::take(&mut self.roster);
        for entry in roster_before {
            if !is_valid_identity_hash(&entry.identity_hash) {
                warnings.push(PolicyWarning::InvalidIdentityHash {
                    identity_hash: entry.identity_hash.clone(),
                    label: entry.label.clone(),
                });
                continue;
            }
            if let Some(existing) =
                self.roster.iter_mut().find(|e| e.identity_hash == entry.identity_hash)
            {
                warnings.push(PolicyWarning::DuplicateRosterEntry {
                    identity_hash: entry.identity_hash.clone(),
                    kept_role: entry.role.as_str().to_string(),
                    dropped_role: existing.role.as_str().to_string(),
                });
                *existing = entry;
            } else {
                self.roster.push(entry);
            }
        }

        // Normalize and validate blocked prefixes.
        let blocked_before = std::mem::take(&mut self.blocked);
        for prefix in blocked_before {
            let normalized = prefix.to_ascii_lowercase();
            if normalized != prefix {
                warnings.push(PolicyWarning::NormalizedBlockedPrefix {
                    original: prefix.clone(),
                    normalized: normalized.clone(),
                });
            }
            if is_valid_blocked_prefix(&normalized) {
                if !self.blocked.contains(&normalized) {
                    self.blocked.push(normalized);
                }
            } else {
                warnings.push(PolicyWarning::InvalidBlockedPrefix { prefix: prefix.clone() });
            }
        }

        warnings
    }

    /// Normalize without collecting warnings (for testing convenience).
    pub fn normalize_quiet(&mut self) {
        let _ = self.normalize();
    }

    // ── Roster management ─────────────────────────────────────

    /// Add or replace a roster entry. Identity hashes are normalized to lowercase.
    ///
    /// Returns `false` if the identity hash is not a valid 32-char hex string.
    pub fn add_entry(&mut self, mut entry: RosterEntry) -> bool {
        entry.identity_hash = entry.identity_hash.to_ascii_lowercase();

        if !is_valid_identity_hash(&entry.identity_hash) {
            return false;
        }

        // Filter grants to known capabilities.
        entry.grants.retain(|g| is_valid_capability(g));

        if let Some(existing) =
            self.roster.iter_mut().find(|e| e.identity_hash == entry.identity_hash)
        {
            *existing = entry;
        } else {
            self.roster.push(entry);
        }
        true
    }

    /// Remove a roster entry by identity hash. Returns true if found.
    pub fn remove_entry(&mut self, identity_hash: &str) -> bool {
        let normalized = identity_hash.to_ascii_lowercase();
        let len_before = self.roster.len();
        self.roster.retain(|e| e.identity_hash != normalized);
        self.roster.len() < len_before
    }

    /// Add a blocked identity hash prefix (normalized to lowercase).
    ///
    /// Returns `false` if the prefix is shorter than `MIN_BLOCKED_PREFIX_LEN`
    /// (8 hex chars / 4 bytes) or contains non-hex characters.
    pub fn block(&mut self, prefix: impl Into<String>) -> bool {
        let p = prefix.into().to_ascii_lowercase();
        if !is_valid_blocked_prefix(&p) {
            return false;
        }
        if !self.blocked.contains(&p) {
            self.blocked.push(p);
        }
        true
    }

    /// Remove a blocked prefix. Returns true if found.
    pub fn unblock(&mut self, prefix: &str) -> bool {
        let normalized = prefix.to_ascii_lowercase();
        let len_before = self.blocked.len();
        self.blocked.retain(|p| *p != normalized);
        self.blocked.len() < len_before
    }

    /// Get a roster entry by identity hash.
    pub fn get_entry(&self, identity_hash: &str) -> Option<&RosterEntry> {
        let normalized = identity_hash.to_ascii_lowercase();
        self.roster.iter().find(|e| e.identity_hash == normalized)
    }

    /// Iterate over all roster entries.
    pub fn entries(&self) -> &[RosterEntry] {
        &self.roster
    }

    /// Iterate over blocked prefixes.
    ///
    /// Crate-internal only — exposing blocked prefixes to untrusted callers
    /// enables evasion (choosing hashes outside blocked ranges).
    #[allow(dead_code)] // Used in tests and future enforcement points
    pub(crate) fn blocked_prefixes(&self) -> &[String] {
        &self.blocked
    }

    /// Number of blocked prefixes (safe to expose).
    pub fn blocked_count(&self) -> usize {
        self.blocked.len()
    }

    // ── Policy evaluation ─────────────────────────────────────

    /// Resolve the effective role for an identity.
    ///
    /// Check order: blocked list (prefix match) → explicit roster → default role.
    pub fn resolve_role(&self, identity_hash: &str) -> Role {
        let normalized = identity_hash.to_ascii_lowercase();

        // 1. Blocked prefix check
        if self.blocked.iter().any(|prefix| normalized.starts_with(prefix.as_str())) {
            return Role::Blocked;
        }

        // 2. Explicit roster
        if let Some(entry) = self.roster.iter().find(|e| e.identity_hash == normalized) {
            return entry.role;
        }

        // 3. Default
        self.default_role
    }

    /// Check whether an identity holds a specific capability.
    ///
    /// Capabilities come from two sources:
    /// 1. The role's cumulative capability set.
    /// 2. Explicit grants on the roster entry.
    pub fn has_capability(&self, identity_hash: &str, cap: &str) -> bool {
        let normalized = identity_hash.to_ascii_lowercase();

        // Blocked identities have no capabilities.
        if self.blocked.iter().any(|prefix| normalized.starts_with(prefix.as_str())) {
            return false;
        }

        // Check roster entry (role caps + explicit grants).
        if let Some(entry) = self.roster.iter().find(|e| e.identity_hash == normalized) {
            return entry.has_capability(cap);
        }

        // Fall back to default role's capability set.
        capabilities_for_role(self.default_role).contains(&cap)
    }

    /// Whether the default role grants a given capability.
    /// Used to decide between ALLOW_ALL vs ALLOW_LIST in RNS handlers.
    pub fn default_role_grants(&self, cap: &str) -> bool {
        capabilities_for_role(self.default_role).contains(&cap)
    }

    /// Get the list of identity hashes that hold a given capability
    /// (only explicitly rostered identities, not those covered by default).
    ///
    /// Blocked identities are excluded even if they have a roster entry.
    ///
    /// Crate-internal — exposing the full admin list enables targeted attacks.
    /// External callers should use `has_capability()` for point checks.
    #[allow(dead_code)] // Used in tests and future enforcement points
    pub(crate) fn allow_list(&self, cap: &str) -> Vec<String> {
        self.roster
            .iter()
            .filter(|e| {
                // Exclude blocked identities — blocked overrides roster.
                !self.blocked.iter().any(|prefix| e.identity_hash.starts_with(prefix.as_str()))
                    && e.has_capability(cap)
            })
            .map(|e| e.identity_hash.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Capability;

    fn test_policy() -> RbacPolicy {
        let mut policy = RbacPolicy::new(Role::Peer);
        assert!(policy.add_entry(
            RosterEntry::new("aaaa1111bbbb2222cccc3333dddd4444", Role::Admin).with_label("Alice"),
        ));
        assert!(policy.add_entry(
            RosterEntry::new("eeee5555ffff6666aaaa7777bbbb8888", Role::Operator)
                .with_label("Bob")
                .with_grants(vec![Capability::VPN_HANDSHAKE.to_string()]),
        ));
        assert!(policy.add_entry(
            RosterEntry::new("1111222233334444555566667777aaaa", Role::Monitor)
                .with_label("Charlie"),
        ));
        assert!(policy.block("deadbeef"));
        assert!(policy.block("ca3e9813"));
        policy
    }

    // ── Role resolution ───────────────────────────────────────

    #[test]
    fn resolve_rostered_identity() {
        let policy = test_policy();
        assert_eq!(policy.resolve_role("aaaa1111bbbb2222cccc3333dddd4444"), Role::Admin);
        assert_eq!(policy.resolve_role("eeee5555ffff6666aaaa7777bbbb8888"), Role::Operator);
    }

    #[test]
    fn resolve_unknown_gets_default() {
        let policy = test_policy();
        assert_eq!(policy.resolve_role("0000000000000000ffffffffffffffff"), Role::Peer);
    }

    #[test]
    fn resolve_blocked_prefix() {
        let policy = test_policy();
        assert_eq!(policy.resolve_role("deadbeef11112222333344445555aaaa"), Role::Blocked);
        assert_eq!(policy.resolve_role("ca3e981300000000aaaa00001111ffff"), Role::Blocked);
    }

    #[test]
    fn blocked_overrides_roster() {
        let mut policy = test_policy();
        // Block a prefix that matches Alice's hash.
        assert!(policy.block("aaaa1111"));
        assert_eq!(policy.resolve_role("aaaa1111bbbb2222cccc3333dddd4444"), Role::Blocked);
    }

    #[test]
    fn case_insensitive() {
        let policy = test_policy();
        assert_eq!(policy.resolve_role("AAAA1111BBBB2222CCCC3333DDDD4444"), Role::Admin);
        assert_eq!(policy.resolve_role("DEADBEEF11112222333344445555aaaa"), Role::Blocked);
    }

    // ── Capability checks ─────────────────────────────────────

    #[test]
    fn admin_has_exec() {
        let policy = test_policy();
        assert!(policy.has_capability("aaaa1111bbbb2222cccc3333dddd4444", Capability::RPC_EXEC));
    }

    #[test]
    fn operator_no_exec() {
        let policy = test_policy();
        assert!(!policy.has_capability("eeee5555ffff6666aaaa7777bbbb8888", Capability::RPC_EXEC));
    }

    #[test]
    fn operator_has_config_update() {
        let policy = test_policy();
        assert!(policy
            .has_capability("eeee5555ffff6666aaaa7777bbbb8888", Capability::RPC_CONFIG_UPDATE));
    }

    #[test]
    fn orthogonal_grant() {
        let policy = test_policy();
        // Bob has explicit VPN grant.
        assert!(
            policy.has_capability("eeee5555ffff6666aaaa7777bbbb8888", Capability::VPN_HANDSHAKE)
        );
        // Alice (admin) does NOT have VPN — it's orthogonal.
        assert!(
            !policy.has_capability("aaaa1111bbbb2222cccc3333dddd4444", Capability::VPN_HANDSHAKE)
        );
    }

    #[test]
    fn blocked_has_no_capabilities() {
        let policy = test_policy();
        assert!(!policy.has_capability("deadbeef11112222333344445555aaaa", Capability::CHAT_SEND));
    }

    #[test]
    fn unknown_identity_gets_default_caps() {
        let policy = test_policy();
        // Default is Peer — has chat.send but not rpc.exec.
        assert!(policy.has_capability("0000000011111111aaaa2222bbbb3333", Capability::CHAT_SEND));
        assert!(!policy.has_capability("0000000011111111aaaa2222bbbb3333", Capability::RPC_EXEC));
    }

    // ── Aether capabilities ───────────────────────────────────

    #[test]
    fn peer_can_query_and_report() {
        let policy = test_policy();
        let unknown = "0000000011111111aaaa2222bbbb3333";
        assert!(policy.has_capability(unknown, Capability::AETHER_QUERY));
        assert!(policy.has_capability(unknown, Capability::AETHER_REPORT));
    }

    #[test]
    fn peer_cannot_delegate() {
        let policy = test_policy();
        let unknown = "0000000011111111aaaa2222bbbb3333";
        assert!(!policy.has_capability(unknown, Capability::AETHER_DELEGATE));
    }

    #[test]
    fn operator_can_delegate() {
        let policy = test_policy();
        assert!(
            policy.has_capability("eeee5555ffff6666aaaa7777bbbb8888", Capability::AETHER_DELEGATE)
        );
    }

    // ── Roster management ─────────────────────────────────────

    #[test]
    fn add_replaces_existing() {
        let mut policy = test_policy();
        assert!(policy.add_entry(
            RosterEntry::new("aaaa1111bbbb2222cccc3333dddd4444", Role::Peer)
                .with_label("Alice demoted"),
        ));
        assert_eq!(policy.resolve_role("aaaa1111bbbb2222cccc3333dddd4444"), Role::Peer);
    }

    #[test]
    fn remove_entry() {
        let mut policy = test_policy();
        assert!(policy.remove_entry("aaaa1111bbbb2222cccc3333dddd4444"));
        assert_eq!(policy.resolve_role("aaaa1111bbbb2222cccc3333dddd4444"), Role::Peer);
        assert!(!policy.remove_entry("nonexistent"));
    }

    #[test]
    fn unblock() {
        let mut policy = test_policy();
        assert!(policy.unblock("deadbeef"));
        assert_eq!(policy.resolve_role("deadbeef11112222333344445555aaaa"), Role::Peer);
    }

    #[test]
    fn invalid_grants_filtered_at_construction() {
        let entry = RosterEntry::new("aaaa0000bbbb1111cccc2222dddd3333", Role::Peer)
            .with_grants(vec!["fake.cap".to_string(), Capability::VPN_HANDSHAKE.to_string()]);
        assert_eq!(entry.grants().len(), 1);
        assert_eq!(entry.grants()[0], Capability::VPN_HANDSHAKE);
    }

    #[test]
    fn invalid_grants_filtered_at_add() {
        let mut policy = RbacPolicy::default();
        assert!(policy.add_entry(
            RosterEntry::new("aaaa0000bbbb1111cccc2222dddd3333", Role::Peer)
                .with_grants(vec!["fake.cap".to_string(), Capability::VPN_HANDSHAKE.to_string(),]),
        ));
        let entry = policy.get_entry("aaaa0000bbbb1111cccc2222dddd3333").expect("entry exists");
        assert_eq!(entry.grants().len(), 1);
        assert_eq!(entry.grants()[0], Capability::VPN_HANDSHAKE);
    }

    #[test]
    fn has_capability_rejects_unknown_grants() {
        // Even if a grant string somehow got into the entry, has_capability
        // won't honor it if it's not a known capability.
        let entry = RosterEntry {
            identity_hash: "aaaa0000bbbb1111cccc2222dddd3333".into(),
            role: Role::Peer,
            label: String::new(),
            grants: vec!["smuggled.capability".into()],
        };
        assert!(!entry.has_capability("smuggled.capability"));
    }

    // ── Identity hash validation ──────────────────────────────

    #[test]
    fn reject_short_identity_hash() {
        let mut policy = RbacPolicy::default();
        assert!(!policy.add_entry(RosterEntry::new("aaaa", Role::Peer)));
    }

    #[test]
    fn reject_non_hex_identity_hash() {
        let mut policy = RbacPolicy::default();
        assert!(!policy.add_entry(RosterEntry::new("zzzz1111bbbb2222cccc3333dddd4444", Role::Peer)));
    }

    #[test]
    fn reject_short_blocked_prefix() {
        let mut policy = RbacPolicy::default();
        assert!(!policy.block("aa"));
        assert!(!policy.block("aabb"));
        assert!(policy.block("aabbccdd")); // 8 chars = minimum
    }

    // ── Allow list ────────────────────────────────────────────

    #[test]
    fn allow_list_for_exec() {
        let policy = test_policy();
        let list = policy.allow_list(Capability::RPC_EXEC);
        assert_eq!(list, vec!["aaaa1111bbbb2222cccc3333dddd4444"]);
    }

    #[test]
    fn allow_list_excludes_blocked() {
        let mut policy = test_policy();
        // Block Alice's prefix — she should disappear from allow lists.
        assert!(policy.block("aaaa1111"));
        let list = policy.allow_list(Capability::RPC_EXEC);
        assert!(list.is_empty(), "blocked identity should not appear in allow list");
    }

    #[test]
    fn default_role_grants_chat() {
        let policy = test_policy();
        assert!(policy.default_role_grants(Capability::CHAT_SEND));
        assert!(!policy.default_role_grants(Capability::RPC_EXEC));
    }

    // ── Config deserialization ─────────────────────────────────

    #[test]
    #[cfg(feature = "config")]
    fn deserialize_from_json() {
        let json = serde_json::json!({
            "default_role": "peer",
            "roster": [
                {
                    "identity": "aaaa1111bbbb2222cccc3333dddd4444",
                    "role": "admin",
                    "label": "Alice",
                    "grants": ["vpn.handshake"]
                }
            ],
            "blocked": ["deadbeef"]
        });

        let mut policy: RbacPolicy = serde_json::from_value(json).expect("should parse");
        let warnings = policy.normalize();
        assert!(warnings.is_empty(), "clean config should produce no warnings");
        assert_eq!(policy.default_role, Role::Peer);
        assert_eq!(policy.entries().len(), 1);
        assert_eq!(policy.entries()[0].role, Role::Admin);
        assert_eq!(policy.blocked_prefixes(), &["deadbeef"]);
    }

    #[test]
    #[cfg(feature = "config")]
    fn normalize_reports_all_issues() {
        use crate::PolicyWarning;

        let json = serde_json::json!({
            "default_role": "peer",
            "roster": [
                {
                    "identity": "AAAA1111BBBB2222CCCC3333DDDD4444",
                    "role": "admin",
                    "label": "Alice (uppercase)"
                },
                {
                    "identity": "short",
                    "role": "peer",
                    "label": "Invalid (too short)"
                },
                {
                    "identity": "bbbb2222cccc3333dddd4444eeee5555",
                    "role": "peer",
                    "grants": ["fake.grant", "vpn.handshake"]
                }
            ],
            "blocked": ["DEADBEEF", "ab", "aabbccdd"]
        });

        let mut policy: RbacPolicy = serde_json::from_value(json).expect("should parse");
        let warnings = policy.normalize();

        // Alice normalized to lowercase, short entry dropped.
        assert_eq!(policy.entries().len(), 2);
        assert_eq!(policy.entries()[0].identity_hash, "aaaa1111bbbb2222cccc3333dddd4444");
        // Invalid grant filtered, valid one kept.
        assert_eq!(policy.entries()[1].grants(), &["vpn.handshake"]);
        // Short prefix "ab" dropped, "DEADBEEF" normalized, "aabbccdd" kept.
        assert_eq!(policy.blocked_prefixes().len(), 2);
        assert!(policy.blocked_prefixes().contains(&"deadbeef".to_string()));
        assert!(policy.blocked_prefixes().contains(&"aabbccdd".to_string()));

        // Verify warnings cover every issue.
        assert!(
            warnings.iter().any(|w| matches!(w,
                PolicyWarning::NormalizedIdentityHash { original, .. }
                if original == "AAAA1111BBBB2222CCCC3333DDDD4444"
            )),
            "should warn about normalized Alice hash"
        );
        assert!(
            warnings.iter().any(|w| matches!(w,
                PolicyWarning::InvalidIdentityHash { identity_hash, .. }
                if identity_hash == "short"
            )),
            "should warn about invalid 'short' hash"
        );
        assert!(
            warnings.iter().any(|w| matches!(w,
                PolicyWarning::UnknownGrant { grant, .. }
                if grant == "fake.grant"
            )),
            "should warn about unknown grant"
        );
        assert!(
            warnings.iter().any(|w| matches!(w,
                PolicyWarning::InvalidBlockedPrefix { prefix }
                if prefix == "ab"
            )),
            "should warn about short blocked prefix"
        );
        assert!(
            warnings.iter().any(|w| matches!(w,
                PolicyWarning::NormalizedBlockedPrefix { original, .. }
                if original == "DEADBEEF"
            )),
            "should warn about normalized blocked prefix"
        );
    }
}
