# RNS Evidence Ledgers - Delta Spec

## ADDED Requirements

### Requirement: Deterministic prerequisites precede behavior scenarios

Before behavior scenarios run, test support must provide pure in-memory
interfaces, an injected monotonic clock where existing seams permit it, bounded
event scheduling, stable observations, and replay digests. These prerequisites
must drive normal Styrene state machines without creating a production runtime
or requiring a new production seam.

#### Scenario: Deterministic prerequisite is reproducible
Given identical case revision, schedule ID, inputs, limits, and unchanged Styrene revision
When the prerequisite replay executes twice
Then ordered observations have the same digest within declared step, byte, virtual-time, and watchdog bounds
And no protocol milestone depends on sleep, PTY, Python, hardware, or a live runner

#### Scenario: Existing production seam is insufficient
Given a required observation cannot be controlled through existing test-only or injected-clock APIs
When prerequisite validation reaches that observation
Then dependent cases are classified blocked and name the missing seam
And this change does not add or authorize the production seam

### Requirement: Focused ledgers observe unchanged Styrene

Each focused ledger must independently define authority-backed expected and
forbidden observations, run against unchanged Styrene, retain non-vacuous
evidence, and classify every case. A ledger must not include a production fix.

#### Scenario: Focused scenario executes
Given a focused frame, routing, link, request, resource, bound, or raw-HDLC case has valid authority and prerequisites
When the case runs against unchanged Styrene
Then the ledger retains its inputs, schedule, limits, observations, and artifact digests
And the case receives exactly one permitted classification

#### Scenario: Scenario is too broad to identify behavior ownership
Given one case combines independent failures from multiple behavior owners
When ledger admission reviews its assertions
Then the case is classified invalid or split into focused cases
And no aggregate red result is used to authorize production work

### Requirement: Malformed-input evidence proves bounded recovery or records a mismatch

Focused frame, announce, IFAC, resource-advertisement, and HDLC cases must
observe bounded processing, side effects, terminal state, and recovery to the
next valid input. The evidence must distinguish rejection, policy filtering,
and fabricated success.

#### Scenario: Malformed input precedes a valid control
Given an independently authored malformed input is followed by an authority-owned valid control
When unchanged Styrene processes both through a bounded in-memory schedule
Then evidence records allocation and state bounds plus whether the valid control is processed exactly once
And the result is classified without changing parser or transport production code

### Requirement: Routing and link evidence records topology-specific outcomes

Three-node, diamond, hop-asymmetry, path-loss, link, proof, and identify ledgers
must record topology, directed interfaces, hops, path/link state, correlation,
terminal outcomes, and forbidden loops, storms, duplicate delivery, or silent
success.

#### Scenario: Routed topology case completes
Given a focused topology and deterministic delivery schedule declare route, failure, and recovery actions
When unchanged Styrene processes the case
Then evidence records exact packet interfaces, hops, path/link transitions, and one terminal outcome
And a mismatch is classified red-confirmed only after topology and replay validation

### Requirement: Request and resource evidence records correlation and terminality

Packet requests, resource responses, segmentation, fault schedules, teardown,
and bounds ledgers must record link/request/resource correlation, bytes and
hashes, progress, retries, state counts, and exactly-one terminal observations.
Late or stale traffic must be an explicit forbidden-success observation.

#### Scenario: Resource fault schedule completes
Given a seeded schedule declares selective loss, duplication, reordering, proof handling, and limits
When unchanged Styrene processes the resource case
Then evidence records distinct parts, retries, integrity, delivery count, active-state release, and terminal state
And the classification does not imply a fix or live interoperability claim

### Requirement: Restart evidence follows a pre-decided no-resume policy

Restart cases must be authored only after accepted Styrene authority confirms
that active link, request, and resource state is process-local and does not
resume. The case must expect separate interrupted-run evidence, empty
post-restart active state, and rejection of stale correlation. It must not
require a post-process callback from the terminated runtime.

#### Scenario: No-resume authority is confirmed
Given accepted Styrene authority defines active RNS operation state as process-local
When a restart case is authored and executed
Then the case expects no resumed pre-restart link, request, or resource state
And it observes whether stale parts, proofs, or responses avoid fabricated completion

#### Scenario: Restart authorities conflict
Given an accepted authority defines resumable in-flight state or the authority is unresolved
When restart case planning begins
Then the restart ledger is classified blocked pending an owner decision
And no test selects a restart policy

### Requirement: Queue evidence measures bounds without prescribing a fix

Focused announce/path and link/request/resource/interface ledgers must exercise
zero, exact, and one-over-capacity inputs where valid. They must retain depth,
state/byte high-water marks, admission/refusal, fairness, retry, release, and
repeated-cycle plateau observations.

#### Scenario: Capacity cycle is observed
Given a focused queue reaches capacity, receives one additional item, releases capacity, and repeats
When unchanged Styrene executes the bounded schedule
Then evidence records the admission or refusal outcome and whether retained state plateaus
And any confirmed mismatch opens a subsystem-owned OpenSpec rather than a production task here

### Requirement: Ordinary raw-HDLC evidence uses pure in-memory byte streams

Ordinary raw-HDLC validation must use pure in-memory duplex streams. It must
cover simplified HDLC without KISS, fragmentation, coalescing, partial writes,
malformed recovery, disconnect, reopen, bounded queues, and stale-delivery
absence. PTY execution is a separate platform capability.

#### Scenario: In-memory raw-HDLC case runs ordinarily
Given a pure in-memory duplex stream and a deterministic raw-HDLC schedule
When ordinary validation executes
Then exact wire bytes and ordered payload observations are retained without PTY, Python, network, hardware, or a live process
And the result proves only in-memory raw-HDLC behavior

#### Scenario: PTY capability is absent
Given a PTY-specific raw serial case runs on a platform without the declared PTY capability
When platform preflight executes
Then the PTY case is classified blocked
And the in-memory result is not substituted as PTY evidence
