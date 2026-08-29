# Leviculum-Informed RNS Evidence Wave Design

## Evidence-only boundary

This change produces tests, test-support infrastructure, case definitions,
metadata, observations, and evidence ledgers. It does not change production
behavior. A test-support change must remain outside production builds. If a
case requires a new production seam or behavior, classify the case `blocked` or
`red-confirmed` as applicable. Open a separate behavior-owned OpenSpec before
editing production code.

Leviculum is a category reference only. It does not supply protocol authority,
source code, tests, expected bytes, generated fixtures, binaries, constants,
logs, or pass results. The immutable reference is
`https://codeberg.org/Lew_Palm/leviculum.git` at
`9d5de12dcb9b236b7ef02dc3b88cd2fafcc8efa1`, licensed
AGPL-3.0-or-later.

## Ownership and dependencies

| Concern | Sole owner | This change's role | Dependency or handoff |
|---|---|---|---|
| Python RNS 1.5.1 behavioral authority | `reticulum-1-5-parity-wave` | Consume the immutable authority decision | Wait for its authority record before Python-derived metadata |
| Fixture schema and canonical 1.5.1 provenance | `reticulum-1-5-parity-wave` | Add conforming Leviculum category metadata and independent case records | Wait for its schema and validation contract; do not fork either |
| Production RNS behavior | Behavior-specific follow-up OpenSpec | Observe and classify unchanged Styrene | `red-confirmed` requires a new OpenSpec before production edits |
| Request packet/resource live registration | `reticulum-lxmf-nomadnet-parity` task `4.7` | Supply request/response schedules, assertions, and cases | Handoff only; do not register |
| Routed link/request/channel/resource live registration | `reticulum-lxmf-nomadnet-parity` task `5.7` | Supply routing, link, proof, resource, and recovery cases | Handoff only; do not register |
| Live gate enablement | `reticulum-lxmf-nomadnet-parity` task `12.6` | Supply dependency and evidence-class requirements | Do not enable, schedule, or reinterpret the gate |
| Support claims | `reticulum-lxmf-nomadnet-parity` task `12.9` | Supply classified evidence ledger entries | Do not generate or promote claims |
| Interop process supervision and report contract | Existing `styrene-interop-runner` | Validate case compatibility and emit handoff manifests | Do not create a runner, catalog, topology allocator, or report format |
| PTY capability | Existing platform-specific test/gate owner | Supply a raw-HDLC PTY case | Keep out of ordinary validation |
| Physical LNode capability | Existing hardware gate owner through parity task `12.6` | Supply black-box schedule and assertions | Do not substitute virtual or Python evidence |

## Required order

1. Confirm the Reticulum 1.5 parity wave's immutable Python RNS 1.5.1 authority.
2. Wait for its fixture schema and provenance validator contract.
3. Add clean-room Leviculum category metadata and result classifications by
   using that schema's extension mechanism.
4. Establish test-only deterministic scheduler, injected-clock, observation,
   replay, and existing-runner case-contract prerequisites.
5. Resolve each scenario's authority and expected observations. Resolve the
   restart policy before authoring restart cases.
6. Author and run focused in-memory scenario cases against unchanged Styrene.
7. Author PTY, live Python, and physical LNode case packages separately. Hand
   live packages to parity tasks `4.7`, `5.7`, and `12.6`.
8. Classify every result and retain evidence. Open a separate OpenSpec for every
   `red-confirmed` behavior before production changes.
9. Give the completed evidence ledger to parity task `12.9`. This change does
   not publish claims.

Scenario tasks cannot start before step 4. Python-derived fixture metadata
cannot start before steps 1 and 2. A missing dependency produces `blocked`, not
an inferred schema or local replacement.

## Result classification

Every case has exactly one terminal classification:

| Result | Meaning | Required record | Permitted next action |
|---|---|---|---|
| `green` | Unchanged Styrene satisfies all valid case assertions | Styrene revision, case revision, schedule ID, observation digest, limits, and retained artifacts | Keep as evidence; no production edit |
| `red-confirmed` | A reproducible observation conflicts with current Styrene authority after the case, harness, and environment are validated | Minimal mismatch, authority citation, rerun evidence, affected behavior owner, and new OpenSpec ID | Plan the behavior in the new OpenSpec; no production edit here |
| `invalid` | The case, fixture, assumption, provenance, or assertion is wrong or outside declared scope | Rejection reason and invalidated input digest | Correct or retire the case; do not open a behavior fix |
| `blocked` | Authority, schema, test seam, platform capability, live registration, hardware, or required decision is unavailable | Blocker owner, dependency, and unblock condition | Wait for the owner; do not infer a result or edit production |

Environment failure is `blocked` unless it proves that the case itself is
invalid. A timeout is not automatically `red-confirmed`; the evidence must show
a protocol mismatch rather than harness or capacity failure. A result cannot be
called fixed in this change.

## Clean-room metadata

The Reticulum 1.5 parity wave owns the canonical schema and fixture authority.
This change waits for that contract and adds only conforming records for:

- Leviculum repository, immutable revision, AGPL-3.0-or-later license, and
  plain-language category;
- independent local case ID, authoring basis, and review status;
- applicable RNS 1.5.1 authority record;
- schedule ID, deterministic seed, limits, and case digest;
- expected and forbidden observations derived from protocol authority and
  current Styrene design, not Leviculum implementation details;
- evidence class and terminal result classification;
- artifact digests and follow-up OpenSpec ID when `red-confirmed`.

Do not extend or replace the canonical schema until its owner exposes an
extension mechanism. If the required metadata cannot be represented, classify
metadata-dependent cases `blocked` and request a schema decision from the
Reticulum wave.

Do not copy, translate, mechanically transform, compile, link, vendor, or use
Leviculum source or artifacts as an oracle. A physical LNode can be observed as
a black-box peer only when board and firmware provenance are recorded. Its
behavior does not define canonical expected results.

## Evidence prerequisites

These prerequisites occur immediately after governance and before scenario
authoring. They are test-support only.

### Deterministic scheduler

Reuse normal `styrene-rns` state machines behind in-memory test interfaces. A
case uses an event queue ordered by `(virtual_time, insertion_sequence)` and a
fixed schedule ID. Actions include deliver, drop, duplicate, reorder,
disconnect, reconnect, cancel, and restart. Limits cover steps, packets, bytes,
queue depth, virtual duration, and a wall-clock hang watchdog.

### Injected clock

Use existing manual monotonic-clock seams. If a required path still uses wall
time and cannot be observed through existing test APIs, record a blocker. Do
not add a production clock seam in this change. Protocol assertions never use
sleeps as milestones.

### Observation ledger

Record only stable protocol and public-state observations: packet hash,
context, hops, interface, path state, link state, request/resource correlation,
queue high-water marks, terminal events, and forbidden duplicate or success
events. Diagnostic traces may be retained but are not authority.

### Replay

Two executions with the same case revision, schedule ID, inputs, and limits
must produce the same ordered observation digest. A replay mismatch blocks the
dependent behavior case until the test-support cause is resolved.

### Existing-runner case contract

Validate that live case manifests can declare revisions, topology needs,
ordered milestones, assertions, deadlines, artifacts, cancellation, and
cleanup through the existing runner contract. Produce handoff manifests only.
Do not add scenario IDs to the catalog, expand live topology allocation, or
change the runner report in this wave. Any missing runner capability is a
blocker owned by the existing parity/runner plan.

## Restart authority decision

Resolve restart expectations before authoring a restart case. Current Styrene
transport design stores active links, request receipts, and resource sender and
receiver state in process-local memory. Existing persistent transport storage
does not define resumable in-flight resource state. Therefore this evidence
wave uses a no-resume policy:

- a restarted runtime starts with no active pre-restart link, request, or
  resource correlation;
- stale pre-restart parts, proofs, or responses cannot recreate or complete the
  old operation;
- external runner evidence records the interrupted pre-restart operation and
  post-restart empty active state separately;
- no case expects persisted transfer resume or a post-process terminal callback
  from the terminated runtime.

If another accepted Styrene authority document defines resumable in-flight RNS
resources, the conflict is a decision blocker. Do not let a test choose between
policies and do not author the restart schedule until the owner resolves it.

## Focused evidence ledgers

The corpus uses small behavior-owned ledgers rather than broad scenario suites:

| Ledger | Independent cases and assertions |
|---|---|
| Frame admission | Truncated headers and hashes, invalid fields and lengths, excessive hops, over-MTU and zero-data inputs |
| Announce and IFAC rejection | Tampered signatures, malformed app data, wrong IFAC, policy drop versus invalid traffic |
| Resource-advertisement parsing | Truncation, inconsistent sizes, excessive parts, invalid flags, decompression bound |
| HDLC deframing | Noise, invalid escapes, incomplete/oversized frames, fragmentation, coalescing, and recovery to the next valid frame |
| Three-node routing | Exact hops, next interface, path request/response, deduplication, and terminal delivery |
| Diamond return path | Failed arm, attached-interface proof return, no loop, and no proof storm |
| Hop asymmetry | Unequal route lengths, bounded hop handling, stale path observation, and no silent success |
| Path loss and recovery | Expiry, better/worse replacement, rediscovery, alternate route, and silent-resume observation |
| Link establishment and proofs | Pending-before-send observation, valid/forged/wrong-interface proofs, duplicate request, close, and proof loss |
| Identify and protected access | Valid identify, malformed/forged identify, authenticated identity, and protected-path denial |
| Packet requests and receipts | Correlation, denial timeout, malformed/duplicate/late/wrong-link response, cancel, and close |
| Resource responses | Packet/resource threshold, response correlation, response limits, and terminality |
| Resource segmentation | Exact thresholds, metadata, repeated identical payloads, part mapping, and integrity |
| Resource fault schedule | Selective part/proof loss, duplicates, reordering, proof replay, bounded retry, and exactly-once delivery |
| Resource teardown | Cancel, link close, multiple active transfers, state release, and stale traffic rejection |
| Resource restart | Pre-decided no-resume policy, empty post-restart active state, and stale correlation rejection |
| Announce and path bounds | Exact capacity, one-over-capacity, per-interface fairness, refusal, release, and state plateau |
| Link, request, resource, and interface bounds | Active-state protection, admission/refusal, retry bounds, release, and repeated-cycle plateau |
| Raw-HDLC in-memory stream | Simplified HDLC without KISS, arbitrary chunking, partial writes, malformed recovery, disconnect, and reopen |
| Raw-HDLC PTY capability | Platform serial adapter behavior over PTY with the same bytes and assertions as the in-memory control |
| Raw-HDLC live Python | Bidirectional announce and data with pinned Python RNS `SerialInterface` |
| Raw-HDLC physical LNode | Black-box announce, path, and data for the recorded board, firmware, transport port, baud, and topology |

Each ledger record includes controls that prevent vacuous passes. Existing
Styrene tests can be cited as controls, but a missing observation remains
`blocked` rather than guessed.

## Raw-HDLC evidence ladder

Raw serial uses simplified HDLC, not KISS/RNode framing. Evidence levels remain
separate:

| Level | Execution | Proves | Does not prove |
|---|---|---|---|
| In-memory | Pure Rust byte streams in ordinary validation | Framing, chunking, partial writes, parser recovery, and test-state behavior | OS serial integration, Python, or hardware |
| PTY platform | Explicit platform capability gate | Serial adapter behavior through an OS PTY | Python or physical LNode behavior |
| Live Python | Parity-owned registered gate with pinned Python RNS `SerialInterface` | Cross-process raw-HDLC behavior at recorded revisions | Physical LNode behavior |
| Physical LNode | Parity-owned enabled hardware gate | Declared behavior for one recorded board and firmware | Canonical authority, other devices, or full protocol parity |

Ordinary validation must not allocate a PTY, launch Python, access serial
hardware, or start a live runner process. Hardware absence is `blocked`, never
green. A debug port cannot substitute for the LNode transport port.

## Live-case handoff

This wave may write runner-compatible case manifests, schedules, assertions,
fixtures, and expected milestone descriptions. It does not register those cases
in `styrene-interop-runner`.

- Parity task `4.7` receives packet and resource request/response cases.
- Parity task `5.7` receives route, path recovery, link, identify, proof,
  channel, resource-fault, raw-serial Python, and physical LNode cases.
- Parity task `12.6` decides when the registered gates are enabled and how
  unavailable platform or hardware dependencies are reported.
- Parity task `12.9` consumes the classified ledger and solely decides claims.

If those owners reject or cannot represent a case, classify its live evidence
`blocked`. Do not create a duplicate catalog or wrapper script.

## Risks and controls

- Evidence work can drift into fixes. The task list contains no production edit
  and every `red-confirmed` result requires a separate OpenSpec.
- Schema work can conflict with RNS 1.5.1 authority. Wait for and extend the
  Reticulum wave contract rather than editing it here.
- Test infrastructure can become a second runtime. Keep it in test support and
  drive normal state machines through in-memory interfaces.
- Live cases can duplicate parity ownership. Produce handoff artifacts only.
- Restart assertions can invent persistence behavior. Apply the no-resume
  authority decision first or mark the case blocked.
- PTY results can be mislabeled as ordinary or hardware evidence. Keep all four
  raw-HDLC levels distinct in manifests and reports.
- AGPL material can leak through translated cases. Require independent
  authorship review and reject unexplained similarity or fixture bytes.
