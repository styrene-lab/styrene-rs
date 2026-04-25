//! Policy warnings — every silent failure during config loading is reported.

use core::fmt;

/// A non-fatal issue discovered during policy config loading.
///
/// Warnings don't prevent the policy from loading — they report what was
/// dropped, filtered, or normalized so the operator can fix their config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyWarning {
    /// A roster entry was dropped because the identity hash is not valid
    /// (must be exactly 32 lowercase hex characters).
    InvalidIdentityHash { identity_hash: String, label: String },
    /// A blocked prefix was dropped because it's too short (minimum 8 hex
    /// chars) or contains non-hex characters.
    InvalidBlockedPrefix { prefix: String },
    /// An identity hash was normalized from uppercase to lowercase.
    NormalizedIdentityHash { original: String, normalized: String },
    /// A blocked prefix was normalized from uppercase to lowercase.
    NormalizedBlockedPrefix { original: String, normalized: String },
    /// A grant was dropped from a roster entry because it's not a known
    /// capability string (possible typo).
    UnknownGrant { identity_hash: String, grant: String },
    /// Duplicate roster entry for the same identity hash. The later entry
    /// was kept and the earlier one was dropped.
    DuplicateRosterEntry { identity_hash: String, kept_role: String, dropped_role: String },
}

impl fmt::Display for PolicyWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentityHash { identity_hash, label } => write!(
                f,
                "dropped roster entry: invalid identity hash '{identity_hash}' \
                 (must be 32 hex chars){label_suffix}",
                label_suffix =
                    if label.is_empty() { String::new() } else { format!(" [label: {label}]") }
            ),
            Self::InvalidBlockedPrefix { prefix } => {
                write!(f, "dropped blocked prefix: '{prefix}' (must be >= 8 hex chars)")
            }
            Self::NormalizedIdentityHash { original, normalized } => {
                write!(f, "normalized identity hash: '{original}' -> '{normalized}'")
            }
            Self::NormalizedBlockedPrefix { original, normalized } => {
                write!(f, "normalized blocked prefix: '{original}' -> '{normalized}'")
            }
            Self::UnknownGrant { identity_hash, grant } => write!(
                f,
                "dropped unknown grant '{grant}' from identity '{identity_hash}' \
                 (not a known capability — possible typo?)"
            ),
            Self::DuplicateRosterEntry { identity_hash, kept_role, dropped_role } => write!(
                f,
                "duplicate roster entry for '{identity_hash}': \
                 kept role '{kept_role}', dropped role '{dropped_role}'"
            ),
        }
    }
}
