# Remaining Styrene Workstream Delta

## Purpose

This document summarizes the work that remains after the mobile consolidation
and OpenSpec reconciliation completed on 2026-09-01. It is a planning snapshot,
not a task authority.

The authoritative checkboxes remain in each linked OpenSpec `tasks.md`. Update
those files first when implementation or evidence changes. Then update this
summary if the change affects priorities or release boundaries.

## Revision Boundary

This assessment uses these canonical revisions:

- `styrene-rs`: `2e9500192e55a234e1fe6bbcd1aaf2186ead2874`
- `styrene-ui`: `902a8d91fb182142262cdb17ae2514a901893c82`

The backend revision includes PR #31, which added the destination convergence
corpus. The UI revision includes PR #15, which extracted the deterministic iOS
App Lock policy behind a tested controller.

## Current Position

Core mobile backend contracts are complete. Shared mobile product implementation
is substantially complete. The remaining mobile work is concentrated in Android
BLE, packaged execution, physical acceptance, accessibility evidence, and
release claims.

Several non-mobile workstreams also remain active. These include shared frontend
sessions, live protocol interoperability, firmware execution, repository-signing
publication, FreeTAK hardening, operator profiles, and operation-scoped
authorization.

## Workstream Summary

| Workstream | Status | Remaining delta |
| --- | ---: | --- |
| Complete mobile P0 backend contracts | 35/35 | Verification and archival only |
| Complete mobile product workflows | 37/49 | Recovery, accessibility, packages, physical runs, and claims |
| Deliver mobile messaging minimum | 51/77 | Corpus provenance, Android BLE GATT, package and physical evidence, and RNode claims |
| Shared Dioxus mobile UI | 31/55 | Android BLE, packaged corpus replay, physical acceptance, assistive technology, and release verification |
| iOS App Lock policy | 15/17 | Physical iPhone matrix and separate App Lock versus Keychain prompt observations |
| Desktop network workflow polish | 13/16 | Native keyboard/accessibility checks, retained fixture captures, and Live/Embedded smoke checks |
| Extract Styrene UI repository | 18/23 | Governance, desktop public-session boundary, historical rollback record, and final desktop validation |
| Shared frontend session | 9/29 | Negotiation, event/reconnect behavior, TUI migration, common sessions, desktop migration, and revision-pair verification |
| Reticulum/LXMF/NomadNet parity | 84/85 | Routed channel evidence |
| RNode firmware provisioning | 16/28 | Exact executors, physical write/recovery evidence, accepted allowlists, and package claims |
| Repository signing profile | 33/35 | Immutable vector publication and compatibility lanes |
| FreeTAK RNS hardening | 4/46 | Admission plus key, receipt, Link, resource, supervision, and interface-policy hardening |
| Operator profile lifecycle | 0/19 | Current-main implementation from tests through frontend migration |
| Operation-scoped authorization | 0/18 | Principal, grant, constraint, issuer, decision, discovery, and compatibility contracts |
| Native RNode endpoint transport | 11/11 | Archive-ready; no implementation delta |

## Mobile Product Delta

### Destination convergence

The destination convergence corpus is complete at the component and
embedded-session level. Discovered, manual, pasted, and scanned candidates
reach one backend operation and create one durable conversation. Rejected
candidates create no shell or contact, even when the frontend forwards them.

The corpus surfaced two defects, both fixed. The backend reported one message
for an empty conversation shell. The frontend truncated a pasted candidate
with leading whitespace before trimming it.

Packaged proof of the same journeys remains a separate gate.

Authority:

- `tests/fixtures/mobile-destination-convergence-v1/revision-pair.json`
- `openspec/changes/complete-mobile-product-workflows/tasks.md`, task 8.2

### Recovery and custody

Backend and UI recovery implementations exist. The remaining gate is a complete
cross-repository and packaged matrix. It covers migration, restart, invalid
backups, interrupted operations, custody continuity, and unchanged public identity.

Physical custody remains unevidenced until the assigned Apple and Android hosts
complete every required lifecycle stage. Component, simulator, or host-only tests
cannot replace physical evidence.

Authority:

- `openspec/changes/complete-mobile-product-workflows/tasks.md`, tasks 5.7 and 8.4
- `openspec/changes/deliver-mobile-messaging-minimum/tasks.md`, tasks 7.4 and 8.3

### Android BLE GATT

Android USB fallback is implemented. Android BLE GATT is not. Adapter work covers
permissions, NUS discovery, MTU conversion, writes, callbacks, and close behavior.
Package checks and physical RNode acceptance remain separate gates.

Authority:

- `openspec/changes/deliver-mobile-messaging-minimum/tasks.md`, tasks 11.3-11.5 and 12.1-12.5
- `openspec/changes/shared-dioxus-mobile-ui/tasks.md`, tasks 4.3, 5.2, 5.5, and 5.6

### Packaged execution and accessibility

Source, reducer, component, and CSS tests do not prove packaged behavior. The
package matrix covers clean builds, cold launch, fatal logs, and upgrade survival.
It also covers product journeys, platform lifecycle, and final ledger reconciliation.

VoiceOver and TalkBack require separate packaged evidence. The project also needs
a criterion-by-criterion WCAG 2.2 Level AA applicability and evidence matrix
before it can claim conformance.

Authority:

- `openspec/changes/complete-mobile-product-workflows/tasks.md`, sections 7 and 8
- `openspec/changes/shared-dioxus-mobile-ui/tasks.md`, sections 7-10

### iOS App Lock

Policy decisions now live in a pure controller. Deterministic tests cover
persisted values, the decision matrix, exactly-once launch prompts, post-reboot
behavior, negative authentication outcomes, setup exemption, and startup
ordering. The startup owner permits an explicit retry after a closed outcome.

Physical evidence remains open. An iPhone must still show same-process retry,
post-reboot once-per-boot, `Off`, cancellation, unavailable authentication, and
failed authentication, with App Lock and Keychain prompts recorded separately.

App Lock controls entry to the application session. Keychain custody controls
identity material. Evidence for one boundary does not satisfy the other.

Authority:

- `styrene-ui/openspec/changes/ios-app-lock-policy/tasks.md`

## Protocol And Transport Delta

### Live interoperability

The Direct, resource-backed Direct, and Opportunistic LXMF gates are
bidirectional. One dispatch of the live workflow runs every pinned scenario.
Each run retains proof that both implementations sent, received, and confirmed
delivery of a canonical message, and records the wire representation each side
used. The Rust node proves received single delivery packets the way LXMF does,
so Python senders reach `DELIVERED` for opportunistic messages as well.

Enabling those gates exposed five defects, all fixed. Reopening the message
store released the process's SQLite locks, so an external reader could delete
the write-ahead log and later commits were lost. The transport rejected
canonical Python delivery proofs, both plain-context link proofs and implicit
proofs addressed to the truncated packet hash. Opportunistic sends tracked an
empty packet hash, so no proof could ever correlate.

The live runner signalled process groups in a form that Linux `kill`
misparses, so no hosted scenario had ever passed its revision probe. The stream
interface encoded outbound frames
into a fixed 2 KB buffer. A link-MTU resource part whose escaped ciphertext
exceeded that size was dropped without any signal. Resource transfers to Python
failed at random until the buffer covered a fully escaped frame.

The propagation retrieval gate queues a Python message on the Rust
propagation node, restarts the node, and has the recipient identity retrieve
and acknowledge it. The NomadNet host gate has a pinned Python client fetch
static, dynamic, allow-listed, denied, and file paths from the Rust native
host. Enabling it exposed a sixth defect: file responses lacked the name and
data pair NomadNet expects, so downloads could not be saved.

The NomadNet client gate runs the same five paths in the other direction. The
`styrene` command line browses pages and downloads files through the daemon
socket, and a pinned Python NomadNet node serves them. Enabling it exposed two
more defects, both fixed. Python serves a file as a resource that carries the
name in resource metadata and the raw file as data. The transport rejected that
shape as malformed because it expected the request envelope. A form submission
without a Micron link directive sent no fields at all, so a scripted post could
never reach a dynamic page.

The canonical NomadNet fixture set comes from the pinned Python node handlers
and RNS request packing. The generator runs them against a fixed page tree, two
identities, a link id, and submitted fields. Offline tests replay the same
inputs through the Rust native host and request layer. They require
byte-identical request envelopes, page and file responses, dynamic page
environment, allow-list policy, response envelopes, and resource metadata
framing.

The fixture provenance recorded the pinned NomadNet revision as release 1.2.8.
That revision carries version 1.2.3, and the record now says so.

The propagation policy gates bound the Rust queue through documented
environment overrides. The capacity gate sets the queue below one message, so
the pinned Python sender's upload is cancelled and returns to `OUTBOUND`. The
Rust snapshot records a `capacity` failure with an empty queue. The pinned
Python control client reads the same empty store under the same limit.

The expiry gate queues a message for a second Python identity with a twenty
second expiry. It waits for the Rust node to expire the item and then has the
recipient retrieve. The retrieval completes with no messages while the item is
`expired` on the node.

Enabling these gates exposed one more defect. A capacity rejection of a direct
client upload recorded no failure observation, so operators could not see why a
message was refused.

The routed gates place a pinned Python transport hop between the Python
endpoint and the Rust node. The direct and resource-backed direct gates run
both legs across the hop. Both nodes must report a two hop route and name the
same transport identity as the next hop. The Rust route record also names its
interface. The routed NomadNet gate fetches every host path through the hop
and proves the client saw two hops. Request packet and resource
interoperability is covered by the NomadNet gates in both directions together
with the canonical fixtures.

Enabling the routed NomadNet gate exposed one more defect. The Rust host
announced its NomadNet destination once at startup. A transport node that
joined later never learned the path and could not answer a client's path
request. The host now re-announces the node with every delivery announce and
on operator-triggered announces, as a Python NomadNet node does on its
announce interval.

The hosted routed runs then failed while the local runs passed. A pinned
Python transport permits about one announce per hour per destination and
cancels a pending path response whenever another announce for that
destination arrives. The harness had the Rust node announcing every second,
so the hop never answered the sender's path requests. The routed scenarios
now announce rarely and trigger one announce once the hop is connected. This
is a harness cadence rather than a protocol defect. It still shows that a Rust
node announcing far above Python's rate target is silenced by Python
transports.

The same hop re-arms its path response timer on every repeated path request.
The harness clients now ask at a real client's pace instead of twice a second.

Support claims now follow hosted evidence. The committed record
`tests/interop/handoffs/pinned-live-evidence.json` lists every
`live-interop.yml` dispatch whose scenarios all passed, with the styrene-rs
revision and the pinned upstream revisions. An offline test requires a passing
hosted run for every pinned scenario. It also refuses a live gate in the
capability registry that names a scenario without one. On that evidence the LXMF direct,
resource, and propagation claims and the NomadNet transport claim are
verified. Reticulum operations stay degraded until routed channel evidence
exists.

The remaining live evidence is a routed channel exchange. No pinned Python
application speaks a channel protocol the Rust daemon exposes, so that
evidence needs a Rust-to-Rust channel across a pinned transport hop.
Repository policy keeps live Python runs manual and scheduled workflows
fixture-only, so each hosted dispatch is an operator action recorded in the
evidence file.

Authority:

- `openspec/changes/reticulum-lxmf-nomadnet-parity/tasks.md`, sections 4, 5, 8-10, and 12

### Firmware provisioning

The firmware policy and corpus are not device executors. Desktop work still needs
an exact bounded Espressif USB executor and physical failure recovery. Mobile work
still needs an accepted board and bootloader allowlist plus complete BLE DFU and
fresh-install evidence.

No synthetic fixture can enable a hardware or package support claim.

Authority:

- `openspec/changes/rnode-firmware-provisioning/tasks.md`

### FreeTAK hardening

The archived Reticulum parity wave already owns constant-time Fernet behavior.
The FreeTAK wave must not duplicate that implementation or fixture corpus.

Open security work covers admission, private persistence, fallback classification,
receipt recovery, Link mutation, and bound-interface dispatch. Resource lifecycle,
task supervision, and internal-interface announce policy also remain open.

Authority:

- `openspec/changes/freetak-rns-hardening-wave/tasks.md`

## Runtime And Authorization Delta

### Shared frontend sessions

Neutral framing, the bounded IPC client, and the one-shot CLI migration are
complete. The TUI still owns raw wire behavior. No common `LiveSession`,
`EmbeddedSession`, or `FixtureSession` implementation exists.

The remaining sequence is client negotiation and event lifecycle, TUI migration,
common sessions, cross-repository desktop migration, and aggregate verification
against one immutable backend/UI pair.

Authority:

- `openspec/changes/shared-frontend-session/tasks.md`

### Operator profiles

An old branch contains a partial prototype, but it is not implementation
authority. Work must restart on current `main` by porting failing tests before
production code.

The profile lifecycle covers coherent Quick and Local roots, atomic promotion,
and coherent snapshots. It also covers verified custody recovery, encrypted
Portable operation, exclusive ownership, typed IPC, and frontend migration.

Authority:

- `openspec/changes/operator-profile-lifecycle/tasks.md`

### Operation-scoped authorization

Issue #2 and a stale prototype identify the need. Current `main` has no accepted
operation policy. TDD work covers principals, grants, constraints, roles, and
trusted issuers. It also covers decisions, audit redaction, and policy discovery.

Authority:

- `openspec/changes/operation-scoped-authorization/tasks.md`

## Publication And Closure Delta

Repository-signing vectors remain candidates until a publication commit marks
them released. Latest, previous-supported, and Identity-main compatibility lanes
also remain open.

The native RNode endpoint transport has no remaining implementation task. Its
OpenSpec passes an archive dry run and can be archived as a separate closure
change. Other 100-percent ledgers should be archived only after their validation
and evidence records satisfy the same closure review.

## Recommended Execution Order

1. Implement and test Android BLE GATT before any Android Bluetooth claim.
2. Run clean simulator, emulator, and packaged mobile workflow matrices.
3. Execute physical custody, App Lock, RNode, QR, lifecycle, and accessibility matrices on their assigned hosts.
4. Enable and retain pinned live RNS, LXMF, propagation, and NomadNet gates.
5. Complete exact firmware executors and physical recovery evidence.
6. Finish frontend-session migration before building operator profiles on duplicated lifecycle code.
7. Implement operation-scoped authorization before consumers add more local policy tables.
8. Publish signing vectors and run compatibility lanes.
9. Archive verified changes only after OpenSpec archive dry runs pass.

The cross-repository destination convergence test and the deterministic App
Lock policy tests from the original order are complete and recorded above.

## Evidence Rules

- A checked task requires implementation and its applicable validation.
- A fixture proves only its recorded bytes and source revision.
- A source test does not prove a packaged application.
- Simulator evidence does not prove a physical device.
- Android evidence does not prove iOS behavior, and iOS evidence does not prove Android behavior.
- A prepared or disabled handoff is unevidenced until its required runner executes successfully.
- Synthetic firmware policy tests do not prove device execution or recovery.
- App Lock authentication and identity custody authentication remain separate observations.
- Capability claims must remain at their current level until every required gate passes.
