//! Role hierarchy — cumulative privilege tiers.

/// Privilege tiers on the Styrene mesh. Each role inherits all capabilities
/// from tiers below it. The numeric value determines ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Role {
    /// Explicit deny — all messages dropped, all requests rejected.
    Blocked = 0,
    /// No access. Fail-closed default for restrictive deployments.
    None = 1,
    /// Mesh peer — chat, basic queries, relay requests.
    Peer = 10,
    /// Read-only monitoring — inbox queries, dashboards, datalink.
    Monitor = 20,
    /// Fleet operator — config updates, restricted terminal, write ops.
    Operator = 30,
    /// Full control — exec, reboot, self-update, full terminal.
    Admin = 40,
}

impl Role {
    /// Parse a role name string (case-insensitive).
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "blocked" => Some(Self::Blocked),
            "none" => Some(Self::None),
            "peer" => Some(Self::Peer),
            "monitor" => Some(Self::Monitor),
            "operator" => Some(Self::Operator),
            "admin" => Some(Self::Admin),
            _ => Option::None,
        }
    }

    /// Role name as a lowercase string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Blocked => "blocked",
            Self::None => "none",
            Self::Peer => "peer",
            Self::Monitor => "monitor",
            Self::Operator => "operator",
            Self::Admin => "admin",
        }
    }

    /// Whether this role has any access at all.
    pub fn has_access(self) -> bool {
        self >= Self::Peer
    }
}

#[cfg(feature = "config")]
impl<'de> serde::Deserialize<'de> for Role {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::from_name(&s).ok_or_else(|| serde::de::Error::unknown_variant(&s, ROLE_NAMES))
    }
}

#[cfg(feature = "config")]
impl serde::Serialize for Role {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

#[cfg(feature = "config")]
const ROLE_NAMES: &[&str] = &["blocked", "none", "peer", "monitor", "operator", "admin"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordering() {
        assert!(Role::Blocked < Role::None);
        assert!(Role::None < Role::Peer);
        assert!(Role::Peer < Role::Monitor);
        assert!(Role::Monitor < Role::Operator);
        assert!(Role::Operator < Role::Admin);
    }

    #[test]
    fn from_name_case_insensitive() {
        assert_eq!(Role::from_name("ADMIN"), Some(Role::Admin));
        assert_eq!(Role::from_name("Peer"), Some(Role::Peer));
        assert_eq!(Role::from_name("unknown"), Option::None);
    }

    #[test]
    fn roundtrip_name() {
        for role in
            [Role::Blocked, Role::None, Role::Peer, Role::Monitor, Role::Operator, Role::Admin]
        {
            assert_eq!(Role::from_name(role.as_str()), Some(role));
        }
    }

    #[test]
    fn has_access() {
        assert!(!Role::Blocked.has_access());
        assert!(!Role::None.has_access());
        assert!(Role::Peer.has_access());
        assert!(Role::Monitor.has_access());
        assert!(Role::Operator.has_access());
        assert!(Role::Admin.has_access());
    }
}
