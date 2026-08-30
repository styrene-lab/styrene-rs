# Complete Mobile P0 Backend Contracts Design

## Authority And Ownership

`tests/fixtures/mobile-integration-v1/corpus.json` remains the product acceptance
inventory. `tests/fixtures/mobile-application-parity-v1/corpus.json` remains the
application-workflow comparison ledger. The new
`tests/fixtures/mobile-backend-p0-v1/corpus.json` is an implementation and
handoff ledger only. It references rows in both existing corpora and cannot add
an application observation, establish protocol authority, or satisfy packaged
evidence.

`styrened`, `styrene-ipc`, and backend storage own the behavior in this change.
`styrene-ui` continues to own presentation state, reducers, platform services,
native adapters, clipboard and share actions, OS background scheduling,
notifications, and packaged acceptance. Existing backend APIs remain available
while additive contracts are developed on this branch.

## Work Order

1. Admit and validate the backend P0 corpus.
2. Correct data-loss, panic, and custody-downgrade paths.
3. Add runtime, capability, interface, identity, and storage projections.
4. Add conversation and delivery-observation contracts.
5. Add bounded diagnostics and forced-termination evidence.
6. Reconcile the integration and application-parity corpora with completed
   backend evidence without changing packaged or external parity status.

Safety changes precede additive UI-facing APIs. A frontend must not integrate
legacy hub polling or report secure custody while the corresponding safety case
is incomplete.

## Legacy Polling Safety

The standard propagation synchronization path remains canonical. It already
requires durable acceptance before acknowledgement. If `MobileNode::poll_hub`
is retained for host background opportunities, it must use the same invariant:

- acknowledge only a message that was durably accepted or a duplicate whose
  canonical durable record is verified;
- do not acknowledge rejected, undecodable, or storage-error items;
- return per-item typed outcomes so partial success is visible;
- surface deletion failure rather than reporting unqualified completion; and
- truncate previews by Unicode scalar or grapheme boundary under an explicit
  byte and character limit.

Delegating the helper to the standard synchronization path is preferable to
maintaining two acceptance algorithms. Removing the helper is permitted only
after all callers and corpus actions migrate to the standard path.

## Identity Custody

`PlaintextFile` remains an explicit development and test choice. Selecting
`Keychain`, `AndroidKeystore`, or `EncryptedFile` in a build that cannot provide
that backend must fail before identity creation. There is no implicit fallback.
An encrypted-file backend must receive non-empty host-provided key material; an
empty static passphrase is not a production custody mode.

The public custody projection reports the requested backend, active backend,
availability, protection class, authentication requirement, downgrade state,
and a typed failure. It contains no key bytes, key identifiers that grant
access, credentials, or export operation. Public display metadata is persisted
separately from private identity material and restored before announce data is
constructed.

## Runtime And Generation

Runtime readiness and transport connectivity are independent facts. A composed
node with durable stores and workers available but no active interface is ready
offline, not connected and not shut down. The public contract must represent
that state without requiring a fabricated bearer.

Capabilities, interfaces, failures, and storage health carry the same session
generation as the snapshot that produced them. A consumer can reject stale
observations using that field. Boot failures include a bounded stage and typed
retryability; they must not expose internal paths or secrets.

Process restart resets in-memory operation generations, but durable records
carry stable identities and explicit recovery outcomes. The contract does not
promise that an in-flight network operation resumes after process death.

## Conversation Identity

An explicit start-conversation operation persists a conversation shell keyed by
canonical LXMF delivery destination. The shell may have no messages and no
draft. Listing conversations includes that shell without inventing a preview,
timestamp, unread count, or reachability claim.

Display-name precedence is deterministic:

1. non-empty local contact alias;
2. current canonical announce display name;
3. abbreviated public destination hash.

Changing an alias invalidates affected conversation projections and emits a
typed mutation. It does not rewrite messages or announce observations.

## Delivery Observation

Delivery method, network path, bearer, and receipt remain independent. A
message attempt may reference an immutable route/interface observation that
records session generation, interface identity and kind, next hop, hop count,
freshness, and observation outcome. Absence is represented as unknown, not
inferred from the requested delivery method or current bearer state.

Attempt correlation is stable across projection and restart. A later route
observation cannot silently rewrite evidence attached to an older attempt.
Sensitive underlay details that are not already public backend observations are
not added for UI convenience.

## Diagnostics And Recovery

Mobile diagnostics use a bounded ring with monotonic sequence, wall-clock time
when available, source, stage, severity, session generation, and optional safe
correlation. Message content, title, canonical wire, attachment bytes, keys,
credentials, tokens, passphrases, and private filesystem paths are forbidden.

Export serializes a stable, versioned snapshot through an explicit redaction
pass and reports truncation. Generic `Debug` output is not an export source.
The host share adapter remains frontend-owned.

Storage status reports schema version, open and recovery outcome, last durable
commit evidence, and typed degraded state. Forced-termination tests use a child
process and isolated temporary storage, terminate only a runner-owned process,
and verify exactly-once committed state plus explicit handling of interrupted
work.

## Compatibility And Rollout

New serialized fields use additive defaults where an external IPC consumer
requires rolling compatibility. New exhaustive Rust enums may require a
coordinated frontend update and must not be merged into the shared integration
branch until the frontend handoff is ready. Persisted schema changes require a
migration and restart test.

No corpus row becomes `complete` from implementation alone. Its listed tests
must pass at the declared evidence boundary. Backend completion does not change
application-parity rows from `unevidenced` and does not satisfy packaged gates.

## Risks And Controls

- Dual propagation paths can drift. Prefer one acceptance implementation and
  test partial failure and duplicate handling at the public mobile boundary.
- Custody labels can overclaim hardware properties. Project only properties
  established by the active signer and build.
- Empty conversations can become fabricated content. Persist identity and
  local state only; keep previews optional.
- Route evidence can become retrospective inference. Attach immutable observed
  facts to an attempt or report unknown.
- Diagnostics can leak payloads. Use an allowlisted DTO and mutation tests that
  seed every forbidden secret class.
- Forced-process tests can become destructive. Use isolated paths, bounded
  deadlines, runner-owned child processes, and unconditional cleanup.
