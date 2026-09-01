# Authorization Policy - Delta Spec

## ADDED Requirements

### Requirement: Principals are authenticated and issuer bounded

An authorization principal contains a subject, issuer, roles, groups, session and
client identifiers, authentication source, and bounded claims. Proxy-provided
identity is accepted only after authentication and trusted-issuer validation.

#### Scenario: Trusted proxy identity is authenticated
Given bearer or session authentication succeeded and the configured issuer is trusted
When the principal adapter receives a valid subject and explicit role mapping
Then it returns a normalized principal
And it retains only configured bounded claims

#### Scenario: Proxy identity is not trusted
Given authentication is absent or the supplied issuer is unknown
When the principal adapter receives identity headers
Then it rejects the principal as untrusted
And the headers grant no role or operation

### Requirement: Operation grants are deterministic and deny first

The policy supports exact operation grants and suffix-prefix grants. A matching
explicit deny takes precedence over every matching allow.

#### Scenario: Prefix grant allows an operation
Given an authenticated principal has `allow omegon.native_session.*`
When the policy evaluates `omegon.native_session.read`
Then the decision is allowed
And it identifies the matched prefix grant

#### Scenario: Explicit deny overrides an allow
Given an authenticated principal has a broad matching allow and an exact matching deny
When the policy evaluates the denied operation
Then the decision is denied with reason `explicit_deny`
And no inherited role allow overrides it

#### Scenario: Operation is not configured
Given no normalized grant or operation declaration matches the request
When the policy evaluates the operation
Then it denies the request with reason `misconfigured_operation`

### Requirement: Resource and context constraints limit grants

Grants can constrain stable resource fields and bounded request attributes. A
grant applies only when every declared constraint matches.

#### Scenario: Resource constraint matches
Given a principal has an allow scoped to session `default`
When the principal requests the operation for session `default`
Then the scoped grant can allow the request

#### Scenario: Context deny matches
Given policy denies event ingress where `trigger_kind` is `shutdown`
When an otherwise authorized principal submits that context
Then the decision is denied with reason `explicit_deny`
And unrelated context attributes do not weaken the constraint

### Requirement: Roles are explicit grant bundles

Coarse roles expand through validated data-backed inheritance. Unknown roles,
cycles, and invalid grants fail policy loading without partial activation.

#### Scenario: Operator inherits monitor grants
Given a valid Operator bundle inherits Monitor
When an Operator principal requests a Monitor operation
Then the inherited grant participates in the decision
And the decision identifies its effective source

#### Scenario: Role inheritance is invalid
Given role bundles contain a cycle or unknown parent
When the policy is loaded
Then loading fails before the policy becomes active
And the previous valid policy remains authoritative

### Requirement: Decisions are structured and audit safe

Every evaluation returns allowed state, a stable reason, operation, requirements,
matched grant when present, principal summary, optional resource, and bounded audit
fields without secret authentication material.

#### Scenario: Authentication is missing
Given a protected operation request has no authenticated principal
When the policy evaluates the request
Then it denies with reason `missing_authentication`
And its audit projection contains no credential or unrestricted claim value

#### Scenario: Policy is unavailable
Given no valid active policy is available
When a protected operation is requested
Then the decision is denied with reason `policy_unavailable`
And the caller can distinguish the failure from a capability denial

### Requirement: Policy discovery uses enforcement semantics

Effective-policy discovery describes declared operations, allowed state,
requirements, scopes, and warnings by evaluating the same normalized policy used
for enforcement.

#### Scenario: Client requests effective policy
Given an authenticated principal and active policy
When the client requests a policy descriptor
Then each operation result agrees with enforcement for the same context
And warnings identify coarse, fallback, or partial policy state

#### Scenario: Discovery is not authorization
Given a stale client cached an allowed descriptor
When current enforcement denies the operation
Then the operation is denied
And the cached descriptor does not grant authority
