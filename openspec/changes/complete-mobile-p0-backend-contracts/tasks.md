# Complete Mobile P0 Backend Contracts Tasks

## 1. Corpus And Contract Gate
<!-- specs: mobile-backend-corpus -->

- [x] 1.1 Add `mobile-backend-p0-v1` with exact source revisions, existing P0 integration-case references, application-parity journey references, ownership, assertions, forbidden outcomes, required tests, frontend handoffs, and exclusions
- [x] 1.2 Add dedicated P0 integration cases for persist-before-ACK, Unicode preview, contact conversation creation, fail-closed custody, durable identity edit, and canonical retry, then link the corresponding application-parity rows without changing their evidence status
- [x] 1.3 Add a strict Rust corpus validator and mutation tests for unknown or non-P0 references, duplicate ownership, blank assertions, empty tests, unsafe paths, invalid state transitions, and false frontend readiness
- [x] 1.4 Record the current backend state as available, partial, defective, or missing without treating source inspection as execution evidence

## 2. Propagation Poll Safety
<!-- specs: mobile-backend-messaging -->

- [ ] 2.1 Add failing `MobileNode::poll_hub` tests for accepted, durable duplicate, decode rejection, storage failure, acknowledgement failure, and mixed partial outcomes
- [ ] 2.2 Replace unconditional hub deletion with per-item durable acknowledgement eligibility or delegate the helper to standard propagation synchronization
- [ ] 2.3 Add bounded Unicode-safe preview tests at ASCII, multibyte scalar, combining-mark, empty, and over-limit boundaries
- [ ] 2.4 Expose typed per-item and remote acknowledgement outcomes without weakening standard-propagation durable-before-ACK behavior

## 3. Identity Custody And Metadata
<!-- specs: mobile-backend-identity -->

- [ ] 3.1 Add failing target and feature-matrix tests proving Keychain, Android Keystore, and encrypted-file selection never falls back to plaintext
- [ ] 3.2 Remove implicit plaintext and empty-passphrase production paths while preserving explicit development-only `PlaintextFile`
- [ ] 3.3 Add secret-free custody DTOs and mobile query APIs for requested backend, active backend, protection, authentication, downgrade, availability, and typed failure
- [ ] 3.4 Persist normalized public identity metadata and test query, announce, restart, invalid edit, and unchanged identity hash
- [ ] 3.5 Add physical iOS and Android custody test handoffs without marking unavailable device evidence complete

## 4. Runtime, Interface, Capability, And Recovery
<!-- specs: mobile-backend-runtime, mobile-backend-observability -->

- [ ] 4.1 Add a typed offline-ready runtime projection independent from transport and bearer state, with boot, shutdown, and no-interface tests
- [ ] 4.2 Add bounded boot-stage and recovery failures and verify failed partial composition releases workers, listeners, and stores
- [ ] 4.3 Populate current generation and typed failure reasons in interface, capability, and storage observations
- [ ] 4.4 Add isolated child-process forced-termination tests for committed state, interrupted work, migration, exactly-once restoration, bounded deadlines, and unconditional cleanup
- [ ] 4.5 Expose storage schema, open, recovery, commit, and degraded status without private paths

## 5. Conversation And Contact Projection
<!-- specs: mobile-backend-messaging -->

- [ ] 5.1 Add a durable conversation-shell schema and idempotent typed start-conversation operation for canonical delivery destinations
- [ ] 5.2 Include empty shells in bounded conversation queries without fabricated preview, timestamp, unread, route, or connectivity state
- [ ] 5.3 Resolve display identity by contact alias, canonical announce name, then bounded destination abbreviation
- [ ] 5.4 Emit typed conversation invalidation after contact mutation and test alias edit, removal, restart, and unrelated-peer isolation

## 6. Attempt Route And Bearer Evidence
<!-- specs: mobile-backend-messaging, mobile-backend-runtime -->

- [ ] 6.1 Define immutable generation-scoped route and interface observation DTOs that can be referenced by one message attempt
- [ ] 6.2 Persist or reconstruct only documented stable correlation and test direct TCP, absent route, stale route, reconnect, retry, and restart outcomes
- [ ] 6.3 Preserve delivery method, bearer, path, and receipt as independent fields and reject inference from current interface state
- [ ] 6.4 Add projection and serialization tests that retain unknown evidence explicitly and expose no private underlay data

## 7. Diagnostics And Export
<!-- specs: mobile-backend-observability -->

- [ ] 7.1 Add a bounded chronological diagnostic ring with stable sequence, source, stage, severity, time, generation, safe correlation, and truncation metadata
- [ ] 7.2 Add an allowlisted redacted export DTO and canonical serialization independent from generic `Debug`
- [ ] 7.3 Add mutation tests for message content, title, wire bytes, attachments, keys, credentials, tokens, passphrases, private paths, encoded variants, capacity, and deterministic digest
- [ ] 7.4 Expose bounded export bytes and metadata while leaving file save and platform sharing to `styrene-ui`

## 8. Verification And Handoff
<!-- specs: mobile-backend-corpus, mobile-backend-runtime, mobile-backend-identity, mobile-backend-messaging, mobile-backend-observability -->

- [ ] 8.1 Run OpenSpec validation, corpus validators, focused `styrened` and `styrene-ipc` tests, migration and restart tests, formatting, and warning-denied Clippy
- [ ] 8.2 Update backend corpus rows only from retained test evidence and record exact backend revision, test boundary, and unresolved blockers
- [ ] 8.3 Publish an additive frontend handoff for newly verified fields and operations without changing packaged or application-parity status
- [ ] 8.4 Reconcile stale missing-capability text in the integration corpus while preserving host, device, protocol, and packaged evidence gaps
