//! Operation-scoped authorization.
//!
//! One reusable policy for operation, resource, and context decisions so
//! consumers do not keep local operation-to-role tables or parse trusted
//! identity themselves. The evaluator is pure: no I/O, no clocks, no
//! framework types. Evaluation order is fixed:
//!
//! 1. Reject a missing principal, then any role the policy does not know.
//! 2. Expand explicit role bundles (validated at load time).
//! 3. Select exact and suffix-prefix grants for the principal and operation.
//! 4. Apply resource and context constraints.
//! 5. Give every matching explicit deny precedence over allows.
//! 6. Return one structured decision and a bounded audit projection.
//!
//! Unknown operations, malformed input, and an unavailable policy fail closed.

mod decision;
mod discovery;
mod grant;
mod issuer;
mod limits;
mod operation;
mod policy;
mod principal;
pub mod testing;

pub use decision::{
    AuditFields, AuthorizationDecision, AuthorizationRequest, DecisionReason, RequestContext,
    ResourceRef,
};
pub use discovery::{DescriptorWarning, OperationDescriptor, PolicyDescriptor};
pub use grant::{Constraint, Effect, Grant, GrantId, RoleBundle, Selector};
pub use issuer::{
    AuthenticationState, IssuerError, IssuerMapping, PrincipalExtractor, TrustedIssuerConfig,
};
pub use limits::Limits;
pub use operation::{Operation, OperationError, OperationPattern};
pub use policy::{OperationPolicy, PolicyBuilder, PolicyError, PolicySlot};
pub use principal::{AuthSource, Principal, PrincipalError, PrincipalSummary};
