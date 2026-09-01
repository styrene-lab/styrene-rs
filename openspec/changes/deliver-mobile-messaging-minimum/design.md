# Deliver Mobile Messaging Minimum Design

## Reassessment

The shared messaging foundation is implemented. General packaged product
acceptance now belongs to `complete-mobile-product-workflows` and must not be
executed twice under competing ledgers.

This change retains three distinct gaps. They are unresolved application-corpus
provenance, Android BLE GATT implementation, and physical RNode acceptance.
Channel detachment, queue bounds, fragmentation, serialization, and retained
handoff have deterministic implementation coverage; they do not establish a
physical support claim. The remaining acceptance tasks stay open until retained
simulator, package, device, and interoperability evidence exists.

## Product Boundary

This change delivers one Dioxus mobile product from shared Rust source. iOS and
Android are packaging and runtime targets for that application, not separate
product implementations. No maintained native-language comparison application
or fallback host exists.

The minimum product contains Messages, People, Network, and More. Propagation is
a client detail under messaging or network settings, not a mobile administration
workspace.

## Reference Application Corpus

Product parity and protocol interoperability are separate evidence axes:

- The application-parity corpus records observed operator workflows in
  RNS-compatible messaging applications.
- Pinned Python RNS and LXMF runs establish wire and protocol compatibility.
- The mobile-minimum state corpus establishes deterministic typed presentation
  and reducer behavior.
- Packaged Dioxus simulator and device runs prove Styrene delivers the accepted
  workflow on each target.

No axis substitutes for another. A screenshot or reference-app observation does
not prove wire compatibility. A Python round trip does not prove the mobile
workflow. A host component test does not prove a packaged application.

The backend-owned application corpus lives at a versioned path such as
`tests/fixtures/mobile-application-parity-v1/corpus.json`. Each admitted record
contains:

- Application name, version, and build.
- Platform and OS.
- Bundled or observed RNS/LXMF version, when available.
- Source or binary provenance and observation date.
- Evidence artifacts and evidence scope.

The UI repository consumes a versioned copy with the exact backend revision.

References are classified as protocol authority, observed RNS/LXMF application,
candidate RNS/LXMF application, or interaction-only reference. Only protocol
authorities and executed interoperability scenarios support protocol claims. A
candidate cannot establish a workflow floor until its provenance and observation
are admitted. Interaction-only references may inform information architecture or
ergonomics but cannot establish an RNS, LXMF, propagation, receipt, or bearer
outcome.

The recovered inventory starts with these evidence scopes:

- Skywave `1.0` build `5`, with Reticulum `0.9.4`, is an observed RNS/LXMF
  application for public TCP connection and canonical `lxmf.delivery` announces
  only. It has no retained propagation or full-message acceptance evidence.
- Skywave `1.0` build `9` is installed as the iOS beta bundle
  `co.horsfalldesign.skywave`. Privacy-reviewed physical-device captures now
  record its read-only launch, navigation, identity, interface, propagation, and
  composition-entry surfaces and report Reticulum `1.4.2`. It remains a
  candidate until distribution provenance and immutable publication are
  resolved. Build `9` does not inherit build `5` interoperability evidence, and
  no LXMF revision was established by the capture.
- Python Reticulum `1.4.2` at
  `b48b96e61676504e0a4e527b33b9a0b4495c6872` and Python LXMF at
  `795fdaa2b0777c13033787d933d1afc94a2377cb` are protocol authorities.
- NomadNet `1.2.8` at `ad10301569a39d4f43b3d21ae9fc392602c937ca`
  is a pinned native-application reference whose bidirectional application gates
  remain incomplete and whose workflows are excluded from the mobile minimum.
- Sideband, Columba, and MeshChat remain candidate RNS/LXMF applications until
  exact versions, provenance, and executed observations are recovered or rerun.
- Meshtastic and MeshCore remain interaction-only references unless separately
  proven compatible. They cannot contribute RNS or LXMF evidence.

The deleted Swift and Kotlin Styrene hosts are historical migration inputs, not
external parity references. Their behavior cannot satisfy application-corpus,
protocol, or packaged-Dioxus gates. RNode firmware provenance belongs to the
hardware acceptance corpus, not the application-parity corpus.

The recorded LXMF revision is labelled `1.1.0` in propagation evidence and
`1.1.1` in shared provenance and capability metadata. Corpus admission must
resolve that discrepancy from the referenced revision. No validator may select
one label implicitly.

The corpus maps each required journey to one designated observed floor and
records observed facts, the Styrene requirement, accepted intentional
differences, exclusions, and status. Status is one of `matched`,
`intentionally_different`, `deferred`, `unsupported`, or `unevidenced`.
Conflicting reference behavior is resolved explicitly in the matrix rather than
silently combining the most expansive behavior from every application.

## Authority Boundaries

The embedded Rust node owns identity, transport, peer observations, canonical
messages, drafts, delivery methods, attempts, receipts, propagation selection,
synchronization, persistence, and correlation. Shared Rust stores and reducers
own product state. Dioxus components render that state and dispatch typed Rust
actions. They must not derive protocol success from display strings, elapsed
time, queue size, or local button completion.

The current `MobileNode` composition and `DaemonFacade` remain the authoritative
operation boundary. The Dioxus application uses an in-process typed Rust session.
It does not route product operations through a generated-language bridge or
duplicate propagation and delivery logic in platform code.

Platform integration enters through Rust-owned typed services. Unavoidable
launcher or build-system glue may start the Rust application, but it must not own
navigation, product state, protocol state, or workflow decisions.

## Configuration Model

Identity storage, TCP interface configuration, and propagation-node selection
are separate persisted values:

- The identity determines the mobile sender and retrieval authorization.
- The TCP endpoint, such as `rns.styrene.io:4242`, provides network access.
- The `lxmf.propagation` destination hash identifies the selected store-and-forward node.

Changing or reconnecting the TCP endpoint does not silently change the selected
propagation node. A propagation selection is ready only after the backend has a
valid active announce and compatible policy metadata for that destination.

## Session State

The presentation model distinguishes stopped, starting, connecting, connected,
reconnecting, degraded, and failed states. Bearer observations remain separate
for TCP, Bluetooth RNode, and Android USB. Each snapshot and event carries or is
applied under a connection generation so late work cannot replace current state.

Directory entries are keyed by canonical destination hash. Repeated announces
update freshness and metadata. They do not create duplicate people.

## Bluetooth RNode Boundary

The shared `styrene-rns` RNode engine owns KISS framing, incremental decoding,
RNode detection, radio configuration readback, payload admission, write
fragmentation, and shutdown framing. It consumes one cancellation-safe ordered
byte attempt. It does not own discovery, permission, approval, reconnect, or
application lifecycle.

The embedded mobile session owns one active RNode bearer and maps its lifecycle
to the matching backend bearer observation. Bluetooth attempts require an
explicitly approved peripheral. Android USB attempts continue to require an
explicit fallback request and cannot replace active approved Bluetooth.

Rust platform services own BLE discovery and native callbacks. They validate
the Nordic UART Service and its write and notification characteristics before
opening the ordered byte attempt. Writes use response mode, remain serialized,
and do not exceed the platform-reported safe write size. Notification boundaries
never imply KISS frame boundaries.

The host stores only the approved platform peripheral identifier and operator
approval state needed for reconnect. Unknown advertisements never connect
automatically. Attempt generations reject stale discovery, connection, write,
notification, and disconnect callbacks. Interruption closes the current attempt
idempotently and permits a bounded reconnect opportunity while the application
is active. Background execution remains best-effort.

The current `US_915_DEVELOPMENT` profile is a physical test profile, not a
production default. Physical transmission requires an explicit test
jurisdiction record. Production enablement requires a separate regional profile
selection decision.

## Message State

The mobile composer submits a destination, content, and explicit requested LXMF
method. The backend returns the canonical message identifier. Draft clearing is
conditional on acceptance and draft revision so a late result cannot erase text
entered after submission.

The UI projects backend lifecycle states without collapsing them:

- Local persistence and queue acceptance
- Transport send acceptance
- Propagation upload acceptance
- Authenticated recipient delivery evidence, when available
- Retryable or terminal failure

Retry retains canonical identity and creates or resumes the backend-defined
attempt. Duplicate events and repeated snapshots are idempotent.

## Propagation Synchronization

The existing standard LXMF propagation coordinator remains the protocol owner.
The mobile boundary adds selection, manual synchronization, automatic trigger,
progress, and terminal observation APIs where they are missing.

Automatic synchronization is single-flight and bounded. Eligible triggers are
initial connection, reconnection, and an allowed foreground opportunity. A
cooldown prevents trigger storms. Background execution is best-effort and must
not be presented as guaranteed.

The client persists each validated message before sending cleanup
acknowledgement. A failed validation or durable write does not acknowledge that
transient identifier. Repeated upload and synchronization therefore remain safe
under process interruption.

## Test-First Strategy

Implementation proceeds by observable slice:

1. Admit provenance-locked reference observations and classify their authority.
2. Define the workflow parity matrix and intentional differences.
3. Add shared fixture schemas and failing Rust state, reducer, and component tests.
4. Add failing embedded-runtime and in-process session contract tests.
5. Implement the smallest backend and application changes that satisfy each slice.
6. Run local two-party tests before public-Brutus tests.
7. Replay accepted journeys on simulator, emulator, and applicable physical devices.

Every asynchronous test uses a milestone, deadline, and correlation identifier.
Arbitrary sleeps are not acceptance evidence. Fixture, simulator, emulator,
physical-device, and public-network results remain separate.

BLE implementation follows red-green slices. Bearer attribution and backend
readiness are tested before native adapters. A fake ordered-byte attempt proves
bounds, fragmentation, cancellation, and retention before CoreBluetooth or
Android GATT code is added. Each native adapter then receives contract tests and
packaged build gates before physical acceptance.

## Rollout And Claims

Existing persisted identity and message data must remain readable. New
propagation configuration uses an additive persisted field and does not infer a
value from the TCP endpoint.

The minimum release claim covers only TCP, discovery, text messaging, and
propagation-client journeys that pass application-floor, typed-state, protocol,
and packaged-target gates applicable to the claim. A required row classified as
deferred, unsupported, or unevidenced blocks that workflow claim. RNode,
attachment, NomadNet, propagation-host, and advanced operator claims remain
separate.
