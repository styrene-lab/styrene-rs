# Deliver Mobile Messaging Minimum Design

## Product Boundary

This change delivers one Dioxus mobile product from shared Rust source. iOS and
Android are packaging and runtime targets for that application, not separate
product implementations. No maintained native-language comparison application
or fallback host exists.

The minimum product contains Messages, People, Network, and More. Propagation is
a client detail under messaging or network settings, not a mobile administration
workspace.

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

1. Add shared fixture schemas and failing Rust state, reducer, and component tests.
2. Add failing embedded-runtime and in-process session contract tests.
3. Implement the smallest backend and host changes that satisfy each slice.
4. Run local two-party tests before public-Brutus tests.
5. Run simulator, emulator, and applicable physical-device acceptance last.

Every asynchronous test uses a milestone, deadline, and correlation identifier.
Arbitrary sleeps are not acceptance evidence. Fixture, simulator, emulator,
physical-device, and public-network results remain separate.

## Rollout And Claims

Existing persisted identity and message data must remain readable. New
propagation configuration uses an additive persisted field and does not infer a
value from the TCP endpoint.

The minimum release claim covers only passing TCP, discovery, text messaging,
and propagation-client gates. RNode, attachment, NomadNet, propagation-host, and
advanced operator claims remain separate.
