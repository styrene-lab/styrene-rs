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

- `styrene-rs`: `dee1bb9d9950c2740cb415b8422041bcf6402095`
- `styrene-ui`: `bc3e433b10b7895df5b34cd0f7337ecaa9f946cb`

The backend revision includes PR #29, which reconciled active backend and
cross-repository OpenSpecs. The UI revision includes PR #13, which added the iOS
App Lock TDD contract and reassessed desktop network workflow polish.

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
| iOS App Lock policy | 2/17 | Pure policy tests, persistence, failure ordering, reboot semantics, presentation coverage, and complete physical matrix |
| Desktop network workflow polish | 13/16 | Native keyboard/accessibility checks, retained fixture captures, and Live/Embedded smoke checks |
| Extract Styrene UI repository | 18/23 | Governance, desktop public-session boundary, historical rollback record, and final desktop validation |
| Shared frontend session | 9/29 | Negotiation, event/reconnect behavior, TUI migration, common sessions, desktop migration, and revision-pair verification |
| Reticulum/LXMF/NomadNet parity | 75/85 | Canonical NomadNet fixtures and enabled bidirectional live gates |
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

The initial policy implementation is merged. It lacks deterministic coverage for
policy decisions, persisted state, setup completion, exactly-once launch prompts,
post-reboot behavior, negative authentication outcomes, and startup ordering.

App Lock controls entry to the application session. Keychain custody controls
identity material. Evidence for one boundary does not satisfy the other.

Authority:

- `styrene-ui/openspec/changes/ios-app-lock-policy/tasks.md`

## Protocol And Transport Delta

### Live interoperability

Rust behavior and handoff descriptions are not live parity evidence. Remaining
gates require enabled, non-ignored, revision-pinned Python/Rust runs for routed
RNS, Direct and Opportunistic LXMF, resource-backed LXMF, standard propagation,
and native NomadNet.

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
2. Complete deterministic App Lock policy and failure tests.
3. Run clean simulator, emulator, and packaged mobile workflow matrices.
4. Execute physical custody, RNode, QR, lifecycle, and accessibility matrices on their assigned hosts.
5. Enable and retain pinned live RNS, LXMF, propagation, and NomadNet gates.
6. Complete exact firmware executors and physical recovery evidence.
7. Finish frontend-session migration before building operator profiles on duplicated lifecycle code.
8. Implement operation-scoped authorization before consumers add more local policy tables.
9. Publish signing vectors and run compatibility lanes.
10. Archive verified changes only after OpenSpec archive dry runs pass.

The cross-repository destination convergence test from the original order is
complete and recorded in the mobile product delta above.

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
