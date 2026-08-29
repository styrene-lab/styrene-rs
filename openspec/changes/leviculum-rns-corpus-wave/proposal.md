# Leviculum-Informed RNS Evidence Wave

## Intent

Create an evidence-only RNS corpus from independently authored schedules,
assertions, and cases inspired by scenario categories in Leviculum
`9d5de12dcb9b236b7ef02dc3b88cd2fafcc8efa1`. The corpus observes unchanged
Styrene behavior. It does not authorize production implementation, register
live gates, or publish compatibility claims.

## Scope

- Wait for the Reticulum 1.5 parity wave to establish Python RNS 1.5.1
  authority, fixture schema, and canonical fixture provenance.
- Extend that schema with Leviculum category metadata and independent Styrene
  case records. Do not create another schema or fixture root.
- Establish test-only deterministic scheduling, injected-clock, observation,
  replay, and existing-runner case-contract prerequisites before scenario work.
- Produce focused ledgers for malformed inputs, routing, path recovery, links,
  identify, proofs, requests, responses, resources, queue bounds, and raw HDLC.
- Keep ordinary validation on pure in-memory byte streams. Treat PTY execution,
  pinned Python RNS, and an attached physical LNode as separate capability
  gates and separate evidence classes.
- Supply live schedules, assertions, and cases to
  `reticulum-lxmf-nomadnet-parity` tasks `4.7`, `5.7`, and `12.6`. Those tasks
  solely own live gate registration and enablement. Its task `12.9` solely owns
  support claims.
- Exclude all production Rust edits, production behavior changes, live catalog
  registration, gate enablement, claim generation, and tracking-marker changes.
- For each observed case, record exactly one result: `green`, `red-confirmed`,
  `invalid`, or `blocked`. A `red-confirmed` result opens a separate
  behavior-owned OpenSpec before any production edit.

## Success criteria

- Ownership and dependency order are explicit and no runner, schema, authority,
  gate, or claim responsibility is duplicated.
- Every case is independently authored and has immutable provenance, limits,
  expected observations, forbidden observations, and one result classification.
- Deterministic scenario prerequisites are available before behavior cases run,
  or the affected cases are classified `blocked` without changing production.
- Restart cases use the current Styrene no-resume authority decision before test
  authoring; an authority conflict becomes a decision blocker.
- The raw-HDLC evidence ladder reports in-memory, PTY, live Python, and physical
  LNode results separately without promoting one class into another.
- Every confirmed behavior mismatch links a new behavior-owned OpenSpec, and no
  production fix occurs in this change.
