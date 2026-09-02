//! Bounds applied to untrusted policy and request input.

/// Size limits that keep policy loading and evaluation linear and bounded.
/// Every limit is inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_grants: usize,
    pub max_roles: usize,
    /// Longest permitted inheritance chain, counting the starting role.
    pub max_role_depth: usize,
    pub max_operation_len: usize,
    pub max_constraints_per_grant: usize,
    pub max_claims: usize,
    pub max_claim_len: usize,
    pub max_attributes: usize,
    pub max_identifier_len: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_grants: 4_096,
            max_roles: 256,
            max_role_depth: 16,
            max_operation_len: 256,
            max_constraints_per_grant: 16,
            max_claims: 32,
            max_claim_len: 512,
            max_attributes: 64,
            max_identifier_len: 256,
        }
    }
}
