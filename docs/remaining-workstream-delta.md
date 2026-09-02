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
| Extract Styrene UI repository | 21/23 | Governance confirmation and the desktop acceptance and rollback definition |
| Shared frontend session | 29/29 | Archive after closure review |
| Reticulum/LXMF/NomadNet parity | 85/85 | Archive after closure review |
| RNode firmware provisioning | 16/28 | Exact executors, physical write/recovery evidence, accepted allowlists, and package claims |
| Repository signing profile | 33/35 | Immutable vector publication and compatibility lanes |
| FreeTAK RNS hardening | 4/46 | Admission plus key, receipt, Link, resource, supervision, and interface-policy hardening |
| Operator profile lifecycle | 14/19 | Launcher packaging checks, frontend migration, and verification |
| Operation-scoped authorization | 18/18 | Archive after closure review |
| Native RNode endpoint transport | 11/11 | Archived on 2026-09-02 |

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
revision and the pinned upstream revisions. Live gates stay isolated in the
capability registry, as repository policy requires. Fixture gates verify the
record offline instead. Each claim group needs a passing hosted run for every
scenario its live gates opt into. The record must also match the runner's pins
and the workflow matrix.

On that evidence the LXMF direct, resource, and propagation claims and the
NomadNet transport claim are verified. Reticulum operations stay degraded
until routed channel evidence exists.

The routed channel gate closes the last live gap. No pinned Python application
speaks a channel protocol the daemon exposes. Two Rust nodes therefore reach
each other only through a pinned Python transport instance. They open a link
and a reliable channel across it and exchange echoed messages both ways. The proof
records both routes, the hop's transport identity as the next hop, the link,
and every message sent, proved, echoed, and proved back intact.

Enabling it exposed two more transport defects, both fixed. The link proved
received packets with the link request proof context. Python transports never
forward that context on an established link and Python endpoints ignore it,
so a Rust proof could not cross a hop. Proofs now carry the plain context
Python uses, and the initiator accepts both.

The transport also delivered link proofs only to initiator links. The destination side of a link never
learned that its own channel packets were proved and tore the link down after
retries. Proofs now reach inbound links as well.

Repository policy keeps live Python runs manual and scheduled workflows
fixture-only, so each hosted dispatch is an operator action recorded in the
evidence file. A hosted dispatch has recorded the routed channel scenario, so
the Reticulum operations claim is verified as well.

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

Neutral framing, the bounded IPC client, negotiation, event fanout,
compatibility polling, full operation coverage, and the one-shot CLI migration
are complete. The TUI connects, negotiates, queries, and receives pushed events
through the shared client, and no frontend crate depends on the IPC server.

The `styrene-session` crate now owns the Live, Embedded, and Fixture profiles.
Live connects to an existing endpoint and fails with a recoverable typed error.
Embedded starts a `styrened` runtime over a private socket and shuts it down on
close. Fixture answers from a script over an in-process stream pair with no
daemon or network interface. A loopback e2e test shows Live and Embedded
sessions returning the same identity, status, capability, and device records.
Ordinary validation now also runs the IPC wire and client unit tests, which CI
had never selected.

The TUI migration exposed a decoding defect. The TUI decoded typed payloads
with rmpv's enum decoding, which rejects the daemon's string-spelled enum
fields. Pushed message events therefore never reached the TUI. Typed decoding
now goes through the shared client decoder, and a loopback e2e test covers the
TUI daemon layer.

Removing the TUI's hand parsers exposed a second defect. The serde spelling of
the NomadNet host discovered capability did not match the wire spelling. No
typed consumer could decode a device that advertised it. The contract now pins
the wire spelling and tolerates unknown capabilities.

The TUI smoke checks run without a terminal: a headless render across every
workspace, a Live-failure unit test, and an embedded runtime e2e test. The
`styrene-ui` desktop now pins `styrene-rs` at `be869620` and runs on the shared
client and sessions. Its request broker, daemon bridge, and parsers are gone,
conversations use the canonical record, and its live scenario catalog mirrors
every pinned runner scenario. The tested revision pair is recorded in the
change's design document. The ledger is complete and awaits closure review.

The loopback network suite (`just test-network`, `network-tests.yml`) does not
pass on `main`. Hosted runs 33639376197 (`main`) and 33638808709 stop at the
first failing binary because `cargo test` fails fast.

A local run with `--no-fail-fast` shows three pre-existing failure classes. Attribution
assertions compare an LXMF source hash with the sender identity hash instead of
its delivery destination. Fleet RPC exec calls time out with no request
handler. RBAC tests depend on that same source semantics. Until those are
repaired, the network suite gives no hosted evidence for any test it contains,
including the TUI loopback test.

Authority:

- `openspec/changes/shared-frontend-session/tasks.md`

### Operator profiles

An old branch contained a partial prototype that was not implementation
authority. Its Quick and Local root, lease, path-escape, private-permission,
managed-daemon, and promotion tests now pass on current `main`. The daemon
composes a managed profile from explicit paths for configuration, identity,
database, nodes, pages, files, and the socket, with no global fallback. A
stopped Quick profile promotes to Local through a staged sibling and one
atomic rename.

Snapshots are immutable hashed generations. A running profile snapshots
through SQLite online backup over the live connections. A snapshot restores to
an unused destination as a new generation and is never modified.

Each profile carries a custody record bound only to the daemon's Reticulum
identity fingerprint. Operators can enroll passphrase-encrypted recovery
slots. Hardware abandonment claims continuity only when a slot reproduces the
recorded fingerprint. Otherwise the profile reports continuity unavailable,
creates no replacement identity, and refuses to start.

Portable profiles live on media that a media inspector reports as encrypted
and capable, with the runtime root kept host-private. They resolve by a stable
volume selector and a profile marker, never by a remembered mount path. Safe
removal quiesces the daemon, checkpoints and synchronizes durable state, and
releases ownership before reporting the media removable. Media that vanishes
under a running daemon stops durable writes with no host fallback. Signed
launcher packaging checks remain open because no launcher packaging exists in
this repository yet.

The daemon now exposes the profile lifecycle over typed IPC. The inventory
carries ownership, persistence, custody, and network-policy fields. Creation,
promotion, snapshot, restore, export, import, adoption, and progress return
typed outcomes. Mutations require the `profile.manage` capability. Promotion
publishes the destination and reports that a restart is required before the
source is released.

The session layer now opens Quick, Local, Portable, and Connected sessions and
reads profile truth from the daemon's inventory. Live is an observed condition
rather than a profile. The TUI runs its ephemeral mode as a managed Quick
session and labels the profile from backend truth. Its Standard mode still
composes an unmanaged runtime on the legacy paths, and the desktop migration
follows in `styrene-ui`.

The profile lifecycle covers coherent Quick and Local roots, atomic promotion,
and coherent snapshots. It also covers verified custody recovery, encrypted
Portable operation, exclusive ownership, typed IPC, and frontend migration.

Authority:

- `openspec/changes/operator-profile-lifecycle/tasks.md`

### Operation-scoped authorization

Issue #2 and a stale prototype identified the need. The `styrene-rbac` crate now
ships an `authz` module. It covers:

- authenticated principals with bounded claims and audit-safe summaries
- exact and suffix-prefix operation grants with explicit deny precedence
- validated role bundles with inheritance
- resource and context constraints
- trusted issuer extraction with a configurable header prefix
- structured decisions with stable reasons and an audit projection
- effective-policy discovery that runs the enforcement evaluator

Policies load atomically and fail closed.

The existing coarse roles remain available as data-backed bundles that cannot
bypass explicit denies. Public-contract tests cover every spec scenario and
issue #2's operation catalog without a consumer role table. Consumers such as
Omegon migrate their catalogs independently. The ledger is complete and
awaits closure review.

Authority:

- `openspec/changes/operation-scoped-authorization/tasks.md`

## Publication And Closure Delta

Repository-signing vectors remain candidates until a publication commit marks
them released. Latest, previous-supported, and Identity-main compatibility lanes
also remain open.

The native RNode endpoint transport had no remaining implementation task. Its
OpenSpec change was archived on 2026-09-02 and its baseline now records the
native RNode transport. Other 100-percent ledgers should be archived only after
their validation and evidence records satisfy the same closure review.

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
