# Mobile Backend P0 Corpus - Delta Spec

## ADDED Requirements

### Requirement: Backend P0 work uses one versioned implementation corpus

The backend must maintain one versioned implementation corpus that maps each
backend-owned P0 work item to existing mobile integration cases and application
parity journeys. The corpus is an implementation ledger and cannot replace its
referenced acceptance or parity authority.

#### Scenario: Backend work item is admitted
Given a backend-owned P0 gap is required by the mobile integration or application-parity corpus
When the work item is added to the backend P0 corpus
Then it identifies current state, owning surfaces, required tests, observable assertions, forbidden outcomes, frontend handoff, and exclusions
And every referenced integration case and parity journey resolves to the existing authoritative corpora

#### Scenario: Host-only work is proposed
Given a proposed row concerns only presentation, native platform permission, packaged execution, or accessibility
When backend corpus validation runs
Then the row is rejected as frontend-owned
And no backend API is invented to satisfy a host-only assertion

### Requirement: Corpus states do not promote evidence

A backend corpus row must use `available`, `partial`, `defective`, or `missing`
for current contract state and `planned`, `implementing`, `verified`, or
`blocked` for delivery state. A verified row establishes only the tests and
boundary named by that row.

#### Scenario: Backend tests pass
Given a corpus row has all required backend tests and assertions satisfied
When its delivery state becomes verified
Then its retained evidence identifies the exact backend and corpus revisions
And application-parity and packaged status remain unchanged

#### Scenario: Referenced authority changes
Given an integration case or parity journey is renamed, removed, or changes priority
When backend corpus validation runs
Then every stale or non-P0 reference fails validation
And the backend row cannot remain verified by implication

### Requirement: Corpus validation is non-vacuous

Validation must reject duplicate IDs, duplicate ownership of the same backend
outcome, blank assertions, unknown source paths, empty required-test sets,
unbounded fault scenarios, and forbidden-outcome lists that cannot fail.

#### Scenario: Required negative assertion is removed
Given a safety row no longer forbids data loss, panic, plaintext downgrade, payload disclosure, stale generation, or fabricated evidence as applicable
When the corpus validator reads the mutated row
Then validation fails with the affected row ID
And the row cannot be classified verified

#### Scenario: Frontend handoff names unavailable data
Given a row declares a backend field or operation ready for frontend use
When that contract is absent or the row state is defective, missing, or blocked
Then validation rejects the ready handoff
And the frontend remains directed to the previously available contract only
