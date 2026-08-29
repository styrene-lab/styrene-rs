# Leviculum-Informed RNS Evidence Wave Tasks

## 1. Governance, ownership, and schema dependency
<!-- specs: rns-corpus-governance -->

- [ ] 1.1 Record the ownership table for Reticulum 1.5.1 authority/schema, parity tasks `4.7`/`5.7`/`12.6`/`12.9`, the existing runner, platform gates, and behavior-owned follow-up OpenSpecs
- [ ] 1.2 Wait for `reticulum-1-5-parity-wave` to publish its immutable Python RNS 1.5.1 authority and fixture-schema validation contract
- [ ] 1.3 Add conforming Leviculum category metadata with repository, revision, AGPL-3.0-or-later license, category-only role, independent case ID, and no source/generated-artifact provenance
- [ ] 1.4 Add evidence validation cases that require exactly one of `green`, `red-confirmed`, `invalid`, or `blocked` and reject stronger evidence or claim promotion
- [ ] 1.5 Verify a `red-confirmed` record cannot close without a separate behavior-owned OpenSpec ID and that this change contains no production-edit authorization

## 2. Deterministic evidence prerequisites
<!-- specs: rns-evidence-ledgers, rns-live-case-handoff -->

- [ ] 2.1 Add or identify test-only in-memory interfaces and a bounded event scheduler ordered by `(virtual_time, insertion_sequence)`
- [ ] 2.2 Reuse existing injected monotonic clocks; record a blocker for any required observation that needs a new production clock seam
- [ ] 2.3 Define the stable observation ledger for packet, interface, path, link, request, resource, queue, terminal, and forbidden-success events
- [ ] 2.4 Add replay controls requiring identical ordered observation digests for the same case revision, schedule ID, inputs, and limits
- [ ] 2.5 Validate runner-compatible case manifests against the existing revision/topology/milestone/assertion/deadline/artifact/cancellation/cleanup contract without registering a catalog entry
- [ ] 2.6 Prove ordinary prerequisite tests use pure in-memory byte streams with no sleep, PTY, Python, network, hardware, or live runner process
- [ ] 2.7 Classify every prerequisite case `green`, `red-confirmed`, `invalid`, or `blocked`; route any red-confirmed test-support behavior into a separate test-infrastructure OpenSpec

## 3. Frame-admission evidence ledger
<!-- specs: rns-evidence-ledgers, rns-corpus-governance -->

- [ ] 3.1 Independently author bounded cases for truncated Type 1/Type 2 headers and hashes, invalid fields/lengths, hops 127/128/255, zero data, and over-MTU frames
- [ ] 3.2 Run the cases against unchanged Styrene with valid authority-owned fixture controls and retain side-effect, allocation, rejection, and post-error observations
- [ ] 3.3 Classify every case `green`, `red-confirmed`, `invalid`, or `blocked`; open a frame-admission OpenSpec for each red-confirmed behavior before any production edit

## 4. Announce and IFAC evidence ledger
<!-- specs: rns-evidence-ledgers, rns-corpus-governance -->

- [ ] 4.1 Independently author cases for tampered announce signatures/app data, malformed optional fields, wrong IFAC, duplicate input, and valid policy-blocked traffic
- [ ] 4.2 Run unchanged Styrene and retain acceptance, rejection, deduplication, counter, path-state, and next-valid-packet observations
- [ ] 4.3 Classify every case and link each red-confirmed announce or IFAC mismatch to a new behavior-owned OpenSpec

## 5. Resource-advertisement parsing evidence ledger
<!-- specs: rns-evidence-ledgers -->

- [ ] 5.1 Independently author cases for truncated MessagePack, inconsistent transfer/data sizes, excessive parts, invalid flags, oversized metadata, and decompression bounds
- [ ] 5.2 Run unchanged Styrene and retain allocation ceilings, active-state counts, proof/completion absence, parser recovery, and terminal observations
- [ ] 5.3 Classify every case and link each red-confirmed parser mismatch to a new resource-admission OpenSpec

## 6. HDLC deframing evidence ledger
<!-- specs: rns-evidence-ledgers -->

- [ ] 6.1 Independently author pure in-memory cases for noise, invalid escapes, incomplete and oversized frames, fragmentation, coalescing, adjacent flags, and valid-frame recovery
- [ ] 6.2 Replay every schedule against unchanged Styrene and retain byte/state bounds plus exactly-once ordered payload observations
- [ ] 6.3 Classify every case and link each red-confirmed HDLC mismatch to a new framing OpenSpec

## 7. Three-node routing evidence ledger
<!-- specs: rns-evidence-ledgers, rns-live-case-handoff -->

- [ ] 7.1 Independently author a three-node in-memory schedule for announce propagation, path request/response, exact hops, next interface, deduplication, and terminal data delivery
- [ ] 7.2 Run and replay unchanged Styrene, retaining packet and path observations plus queue high-water marks and forbidden duplicate delivery
- [ ] 7.3 Classify every case `green`, `red-confirmed`, `invalid`, or `blocked`, open a routing OpenSpec for red-confirmed behavior, and hand the live variant's schedule/assertions to parity task `5.7` without registration

## 8. Diamond return-path evidence ledger
<!-- specs: rns-evidence-ledgers, rns-live-case-handoff -->

- [ ] 8.1 Independently author a four-node diamond schedule with one failed arm, a stale alternate path, attached-interface proof return, and loop/proof-storm prohibitions
- [ ] 8.2 Run and replay unchanged Styrene, retaining exact proof interfaces, link terminality, packet counts, and path observations
- [ ] 8.3 Classify every case, open a routing/proof OpenSpec for red-confirmed behavior, and hand live assertions to parity task `5.7`

## 9. Hop-asymmetry evidence ledger
<!-- specs: rns-evidence-ledgers, rns-live-case-handoff -->

- [ ] 9.1 Independently author unequal forward/return route schedules without packet-field mutation and declare authority-backed hop and silent-success assertions
- [ ] 9.2 Run and replay unchanged Styrene, retaining route lengths, wire/accepted hops, forwarding interfaces, path refresh, drop, and terminal observations
- [ ] 9.3 Classify every case, opening a separate path/proof OpenSpec for red-confirmed behavior and handing live cases to parity task `5.7`

## 10. Path-loss and recovery evidence ledger
<!-- specs: rns-evidence-ledgers, rns-live-case-handoff -->

- [ ] 10.1 Independently author focused schedules for path expiry, failed next hop, better/worse replacement, rediscovery, alternate route, and silent-resume observation
- [ ] 10.2 Run unchanged Styrene under virtual time and retain route state, request count, selected interface, bounded completion, and forbidden stale-route success
- [ ] 10.3 Classify every case `green`, `red-confirmed`, `invalid`, or `blocked`, open a path-recovery OpenSpec for red-confirmed behavior, and hand live schedules/assertions to parity task `5.7` without enabling a gate

## 11. Link-establishment and proof evidence ledger
<!-- specs: rns-evidence-ledgers, rns-live-case-handoff -->

- [ ] 11.1 Independently author cases for pending-before-send observation, valid/forged/wrong-interface proof, duplicate link request, encrypted data, proof loss, close, and cancellation
- [ ] 11.2 Run unchanged Styrene and retain link IDs, proof hashes/interfaces, duplicate-delivery counts, active/history state, and exactly one terminal outcome
- [ ] 11.3 Classify every case, open a link/proof OpenSpec for red-confirmed behavior, and hand live cases to parity task `5.7`

## 12. Identify and protected-access evidence ledger
<!-- specs: rns-evidence-ledgers, rns-live-case-handoff -->

- [ ] 12.1 Independently author cases for valid identify, malformed/truncated/forged identify, authenticated identity retention, and identified/allow-list/callback access
- [ ] 12.2 Run unchanged Styrene and retain identity, access, handler-call, response, denial, and generic-data non-delivery observations
- [ ] 12.3 Classify every case, open an identify/access OpenSpec for red-confirmed behavior, and hand live cases to parity tasks `4.7` and `5.7` as applicable

## 13. Packet-request and receipt evidence ledger
<!-- specs: rns-evidence-ledgers, rns-live-case-handoff -->

- [ ] 13.1 Independently author cases for request ID/link correlation, denial timeout, malformed/duplicate/late/wrong-link packet response, cancellation, and link close
- [ ] 13.2 Run unchanged Styrene under virtual time and retain status, response, timeout, capacity, terminality, and forbidden resurrection observations
- [ ] 13.3 Classify every case, open a request/receipt OpenSpec for red-confirmed behavior, and hand live cases to parity task `4.7`

## 14. Resource-response evidence ledger
<!-- specs: rns-evidence-ledgers, rns-live-case-handoff -->

- [ ] 14.1 Independently author cases for packet/resource response threshold, request ID and link correlation, response limits, wrong resource, late completion, and terminality
- [ ] 14.2 Run unchanged Styrene and retain envelope, advertisement, resource hash, progress, response bytes, and exactly-one-terminal observations
- [ ] 14.3 Classify every case, open a response-resource OpenSpec for red-confirmed behavior, and hand live cases to parity task `4.7`

## 15. Resource-segmentation evidence ledger
<!-- specs: rns-evidence-ledgers, rns-live-case-handoff -->

- [ ] 15.1 Independently author cases for exact part/segment thresholds, one-over-threshold, metadata, repeated identical payloads, map hashes, and integrity
- [ ] 15.2 Run and replay unchanged Styrene, retaining bytes, hashes, part maps, segment counts, delivery count, and terminal state
- [ ] 15.3 Classify every case, open a segmentation OpenSpec for red-confirmed behavior, and hand applicable live cases to parity task `5.7`

## 16. Resource fault-schedule evidence ledger
<!-- specs: rns-evidence-ledgers, rns-live-case-handoff -->

- [ ] 16.1 Independently author seeded cases for selective part/proof loss, duplicate parts, reordering, proof replay, bounded retries, and exactly-once integrity
- [ ] 16.2 Run and replay unchanged Styrene, retaining schedule, retry, requested-part, proof, state-count, payload-digest, and terminal observations
- [ ] 16.3 Classify every case, open a resource-recovery OpenSpec for red-confirmed behavior, and hand live cases to parity task `5.7`

## 17. Resource teardown evidence ledger
<!-- specs: rns-evidence-ledgers, rns-live-case-handoff -->

- [ ] 17.1 Independently author cases for local/remote cancellation, link close, multiple simultaneous inbound/outbound resources, stale parts/proofs, and state release
- [ ] 17.2 Run unchanged Styrene, retaining per-resource terminal events, active-state counts, leaked-task observations, and forbidden resurrection/completion
- [ ] 17.3 Classify every case, open a teardown OpenSpec for red-confirmed behavior, and hand applicable live cases to parity task `5.7`

## 18. Resource restart evidence ledger
<!-- specs: rns-evidence-ledgers -->

- [ ] 18.1 Confirm before test authoring that current accepted Styrene authority retains the no-resume policy for process-local link/request/resource state; classify an authority conflict as a decision blocker
- [ ] 18.2 Only after 18.1, independently author restart cases expecting empty post-restart active state, separate interrupted-run evidence, and rejection of stale parts/proofs/responses without a post-process callback requirement
- [ ] 18.3 Run unchanged Styrene and retain pre-restart correlation, cleanup, post-restart state, stale-traffic, and forbidden fabricated-completion observations
- [ ] 18.4 Classify every case and open a restart-policy or restart-behavior OpenSpec for each red-confirmed result

## 19. Announce and path-bound evidence ledger
<!-- specs: rns-evidence-ledgers -->

- [ ] 19.1 Independently author cases for zero, exact, and one-over announce/path capacities, per-interface fairness, retry admission, refusal/eviction, release, and repeated-cycle plateau
- [ ] 19.2 Run unchanged Styrene and retain depth, byte/state high-water, drop/refusal, fairness, recovery, and plateau observations
- [ ] 19.3 Classify every case and open a bounded-control-plane OpenSpec for red-confirmed behavior

## 20. Link, request, resource, and interface-bound evidence ledger
<!-- specs: rns-evidence-ledgers -->

- [ ] 20.1 Independently author focused capacity cases for active links, pending/terminal requests, incoming/outgoing resources, interface TX, parser state, and retry behavior
- [ ] 20.2 Run unchanged Styrene through repeated saturation/release cycles and retain active-state protection, admission, refusal, terminal, queue, and plateau observations
- [ ] 20.3 Classify every case and open a behavior-owned capacity OpenSpec for each red-confirmed subsystem

## 21. Raw-HDLC in-memory evidence ledger
<!-- specs: rns-evidence-ledgers, rns-live-case-handoff -->

- [ ] 21.1 Independently author pure in-memory duplex cases for simplified HDLC without KISS, arbitrary chunks, partial writes, malformed recovery, back-to-back frames, disconnect, reopen, and bounded queues
- [ ] 21.2 Run and replay unchanged Styrene in ordinary validation, retaining exact wire bytes, ordered payloads, task termination, queue state, and stale-delivery absence
- [ ] 21.3 Classify every case and open a framing/stream OpenSpec for red-confirmed behavior without claiming PTY, Python, or LNode evidence

## 22. Raw-HDLC PTY capability ledger
<!-- specs: rns-evidence-ledgers -->

- [ ] 22.1 Define a platform-capability case that reuses the in-memory wire corpus and assertions through an OS PTY without entering ordinary validation
- [ ] 22.2 Run only where the declared PTY capability exists and retain platform, serial adapter, wire, reconnect, cleanup, and timeout evidence
- [ ] 22.3 Classify absence as `blocked`, case faults as `invalid`, behavior mismatches as `red-confirmed`, and success as PTY-only `green`

## 23. Raw-HDLC live Python handoff ledger
<!-- specs: rns-live-case-handoff, rns-corpus-governance -->

- [ ] 23.1 After the Reticulum wave authority/schema dependency is ready, author pinned Python RNS `SerialInterface` schedules and bidirectional announce/data assertions without registering them
- [ ] 23.2 Validate the case manifest against the existing runner contract and hand it to parity task `5.7`; leave registration and enablement to tasks `5.7` and `12.6`
- [ ] 23.3 Ingest parity-owned execution evidence when available, classify every case `green`, `red-confirmed`, `invalid`, or `blocked`, open a raw-serial interoperability OpenSpec for red-confirmed behavior, and keep it separate from in-memory, PTY, and physical LNode evidence

## 24. Raw-HDLC physical LNode handoff ledger
<!-- specs: rns-live-case-handoff, rns-corpus-governance -->

- [ ] 24.1 Independently author black-box announce/path/data schedules and assertions for a recorded board, firmware digest, transport port, baud, radio profile if used, and topology
- [ ] 24.2 Validate the case manifest against the existing runner contract and hand it to parity tasks `5.7` and `12.6` without registering or enabling it
- [ ] 24.3 Ingest parity-owned hardware evidence when available, classify every case `green`, `red-confirmed`, `invalid`, or `blocked`, open a hardware-interoperability OpenSpec for red-confirmed behavior, and never substitute virtual, PTY, or Python evidence

## 25. Final evidence and handoff audit
<!-- specs: rns-corpus-governance, rns-evidence-ledgers, rns-live-case-handoff -->

- [ ] 25.1 Verify every case has one terminal classification, immutable revisions, schedule/case digest, limits, observations, evidence class, and artifact digests
- [ ] 25.2 Verify every red-confirmed result links a separate behavior-owned OpenSpec and that no production code or production authorization appears in this change
- [ ] 25.3 Verify live case packages were handed only to parity tasks `4.7`/`5.7`/`12.6`, with no local catalog registration, gate enablement, duplicate runner, or claim logic
- [ ] 25.4 Run the Reticulum-wave provenance validator, focused in-memory evidence suites, replay checks, OpenSpec validation, and applicable offline repository checks
- [ ] 25.5 Publish the evidence ledger for parity task `12.9`, preserving green/red-confirmed/invalid/blocked and raw-HDLC evidence levels without generating claims here
