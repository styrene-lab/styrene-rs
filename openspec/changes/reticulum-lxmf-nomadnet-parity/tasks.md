# Reticulum, LXMF, and NomadNet Parity Tasks

## 1. Claims And Baseline Evidence
<!-- specs: parity-claims, interop-verification -->

- [x] 1.1 Inventory current fixture, Rust E2E, ignored live, and manual evidence by parity claim
- [x] 1.2 Add machine-readable claim levels, required gates, upstream revisions, and unsupported reasons
- [x] 1.3 Pin Python RNS, LXMF, and NomadNet revisions and record fixture provenance
- [x] 1.4 Prevent ignored or manual scenarios from producing a verified release claim
- [x] 1.5 Add documentation tests ensuring Styrene-specific pages and propagation are not labelled native NomadNet or LXMF propagation

## 2. Production Composition Contracts
<!-- specs: production-composition, parity-claims -->

- [x] 2.1 Add startup contract tests for each production runtime's advertised capabilities and active components
- [x] 2.2 Add small protocol registration helpers only where ordering and semantics must be shared
- [x] 2.3 Register receipt, resource, retry, page, and enabled propagation handlers in each claiming production runtime
- [x] 2.4 Classify test-only protocol capabilities as internal evidence rather than production parity
- [x] 2.5 Derive active and degraded capabilities from initialized services and authorization
- [x] 2.6 Expose capability version, reason, and connection generation through IPC

## 3. Runtime Truth And Operations
<!-- specs: reticulum-operations, production-composition -->

- [x] 3.1 Add failing tests for real interface identity, endpoints, counters, peers, age, and stale state
- [x] 3.2 Replace configuration-derived and evenly divided interface observations with runtime registry data
- [x] 3.3 Add typed source, observed-time, generation, freshness, and correlation fields to network observations
- [x] 3.4 Add route age, expiry, loss, and rediscovery observations
- [x] 3.5 Add active versus historical link lifecycle observations with ID, interface, RTT, and reason
- [x] 3.6 Add typed announce, path-request, probe, link-open, link-close, and cancellation operations

## 4. Native Reticulum Requests
<!-- specs: reticulum-operations -->

- [x] 4.1 Add path-hash, duplicate-registration, access-policy, and request-size tests
- [x] 4.2 Implement destination request-path registration and identified-link authorization
- [x] 4.3 Add client request receipts before request transmission with deadline and cancellation state
- [x] 4.4 Correlate packet responses, resource responses, denial, link close, timeout, and malformed responses
- [x] 4.5 Enforce maximum response size and select packet or resource response by encoded size
- [x] 4.6 Add request progress and terminal observations to transport and IPC
- [ ] 4.7 Add Python/Rust request packet and resource interoperability scenarios

## 5. Routing, Links, Channels, And Resources
<!-- specs: reticulum-operations, interop-verification -->

- [x] 5.1 Add a deterministic fast-proof regression proving pending link insertion precedes send
- [x] 5.2 Fix outbound link registration ordering and preserve interface-bound proof validation
- [x] 5.3 Integrate channel retry deadlines into the supervised transport scheduler
- [x] 5.4 Integrate resource retries, cancellation, timeout, progress, and cleanup into the same scheduler
- [x] 5.5 Replace diagnostic resource E2E behavior with assertive completion and integrity checks
- [x] 5.6 Add deterministic A-to-B-to-C path, delivery, proof, route-loss, and rediscovery tests
- [ ] 5.7 Add pinned live routed link, request, channel, and resource evidence

## 6. Authoritative LXMF Router
<!-- specs: lxmf-messaging, production-composition -->

- [x] 6.1 Add delivery-decision tests from IPC request through actual wire representation
- [x] 6.2 Introduce a router coordinator owning queued messages, attempts, method selection, and deadlines
- [x] 6.3 Honor Direct, Opportunistic, Propagated, and Paper without silent method substitution
- [x] 6.4 Persist requested method, actual method, fallback reason, attempts, and correlation
- [x] 6.5 Wire transport send acceptance, authenticated packet delivery receipts, verified resource completion, cancellation, and expiry into one representation-aware lifecycle with exact correlation
- [x] 6.6 Ensure unified production startup installs the authenticated RNS receipt bridge and verified outbound-resource completion worker used by tested delivery
- [x] 6.7 Add retry idempotence and terminal-state stickiness tests

## 7. LXMF Fidelity, Conversations, And Attachments
<!-- specs: lxmf-messaging -->

- [x] 7.1 Define canonical inbound content, timestamp, field, signature, and stamp representations without lossy string conversion
- [x] 7.2 Persist verified, invalid, unknown-identity, and not-applicable authentication states
- [x] 7.3 Wire outbound stamp policy, inbound validation, ticket persistence, expiry, and learned peer cost
- [x] 7.4 Move conversation pin, mute, read state, contacts, drafts, and attempts to file-backed storage
- [x] 7.5 Add stable keyset pagination for conversations and message history under concurrent live events
- [x] 7.6 Implement mark-read, search, retry, cancel, delete, contact, pin, and mute IPC outcomes
- [x] 7.7 Implement attachment blob storage, packet/resource transfer, checksums, progress, cancellation, and retrieval

## 8. Standard LXMF Propagation
<!-- specs: lxmf-propagation, production-composition -->

- [x] 8.1 Generate Python fixtures for propagation announces, request paths, offers, transient IDs, and encrypted payloads
- [x] 8.2 Register the standard `lxmf.propagation` destination and compatible announce metadata
- [x] 8.3 Implement offer comparison, transient-ID deduplication, ingest, and identified-client retrieval
- [x] 8.4 Enforce propagation stamps, transfer limits, recipient authorization, capacity, and expiry policy
- [x] 8.5 Persist peers, selection, queue attempts, sync checkpoints, failures, and restart state
- [x] 8.6 Add typed peer, offer, fetch, download, sync, policy, and failure observations
- [x] 8.7 Correlate outbound and inbound messages with propagation queue records
- [ ] 8.8 Add Python/Rust discovery, offer, fetch, offline delivery, expiry, capacity, and restart gates
- [x] 8.9 Schedule fresh propagation announces and include them in operator-triggered network announces

## 9. Native NomadNet Host
<!-- specs: nomadnet-pages, reticulum-operations, production-composition -->

- [ ] 9.1 Add canonical Python NomadNet request and response fixtures for pages, fields, files, and authorization
- [x] 9.2 Register `/page/...` and `/file/...` through the native destination request registry
- [x] 9.3 Serve packet-sized content as responses and larger pages/files as verified resources
- [x] 9.4 Announce `nomadnetwork.node` only after native handlers are active
- [x] 9.5 Implement identified-link and `.allowed` access policy
- [x] 9.6 Implement bounded dynamic page execution, request environment, submitted fields, and secret redaction
- [x] 9.7 Add Python NomadNet-to-Rust static, dynamic, authorized, denied, and file scenarios

## 10. Native NomadNet Client
<!-- specs: nomadnet-pages, reticulum-operations -->

- [x] 10.1 Add typed page-address validation and host capability discovery from native announces
- [x] 10.2 Implement one browse coordinator for path, identity, link, request, transfer, parse, and render stages
- [x] 10.3 Preserve canonical source bytes, request metadata, parser warnings, checksums, and cache status
- [x] 10.4 Implement relative links, history, reload, cache bypass, and connection close without duplicate navigation
- [x] 10.5 Implement interactive Micron fields and native submitted-field requests with password redaction
- [x] 10.6 Implement file download progress, cancellation, integrity, and save handoff
- [ ] 10.7 Add Rust-to-Python NomadNet static, dynamic, failure-stage, and file scenarios

## 11. Operator UX Parity
<!-- specs: reticulum-operations, lxmf-messaging, lxmf-propagation, nomadnet-pages, production-composition -->

- [x] 11.1 Replace hard-coded client capabilities with generation-scoped daemon negotiation
- [x] 11.2 Add announce, path request, probe, route, link, interface, request, resource, and cancellation workflows
- [x] 11.3 Add delivery-method composition, validation, draft retention, immediate outcome, retry, cancel, and history pagination
- [x] 11.4 Add authenticity, stamps, receipts, resource progress, propagation correlation, and terminal error inspection
- [x] 11.5 Add authoritative propagation peer, sync, capacity, policy, and failure views
- [x] 11.6 Replace presentation-generated page stages with correlated daemon observations
- [x] 11.7 Add verified page-host inventory, local inventory, native navigation, forms, files, cache, and diagnostics
- [x] 11.8 Add disconnected, stale, denied, unsupported, timeout, cancellation, and partial-failure fixtures for every operation

## 12. Shared Runner And Release Gates
<!-- specs: interop-verification, parity-claims -->

- [x] 12.1 Extract one structured live runner used by CLI, CI, and Protocol Lab
- [x] 12.2 Record topology, revisions, correlations, milestones, assertions, timings, checksums, logs, and cleanup evidence
- [x] 12.3 Replace arbitrary sleeps with bounded milestone waits and deterministic topology allocation
- [x] 12.4 Enable bidirectional Direct and Opportunistic Python/Rust LXMF gates
- [x] 12.5 Enable bidirectional resource-backed LXMF gates
- [ ] 12.6 Enable routed Reticulum, standard propagation, and native NomadNet gates
- [x] 12.7 Keep ordinary workspace validation offline and deterministic
- [x] 12.8 Run warning-denied Clippy, formatting, unit, component, fixture, property, restart, and migration tests
- [ ] 12.9 Generate final support claims only from passing non-ignored gate evidence
