# Mobile UI Contract

The iOS and Android applications implement the same mobile product, not two
platform-specific feature menus. They share information architecture,
terminology, state semantics, fixture scenarios, and action availability while
using native SwiftUI and Jetpack Compose components.

## Product Shape

The mobile application is a compact communicator informed by Sideband,
Meshtastic, MeshCore, Columba, MeshChat, NomadNet, and Styrene's own TUI and DX
work. Those applications are interaction references, not architectural
authorities. Styrene's capability manifest and daemon observations remain
authoritative.

The primary navigation has four destinations:

1. **Messages** - activity-sorted conversations, unread state, thread, draft,
   send, failure recovery, and delivery evidence.
2. **People** - one directory spanning saved contacts and discovered peers,
   with identity, route, and trust details progressively disclosed.
3. **Network** - phone-to-interface-to-mesh health, announce and refresh,
   active transports, lightweight field map, and bounded diagnostics.
4. **More** - identity, propagation, pages, settings, diagnostics, and product
   capability disclosures.

Full fleet control, terminal access, unrestricted topology inspection, and Lab
workflows are not part of the base mobile product.

## Operator Priority

The mobile hierarchy optimizes for the common communication loop:

1. confirm the active local identity.
2. open or start a conversation.
3. compose and queue a message.
4. inspect delivery and route evidence when the result needs attention.

Messages therefore exposes the local identity as a persistent **You** control.
The control opens the same identity view as More. Network health uses a compact
status row in Messages and does not displace the conversation list. Advanced
network configuration remains in Network and Settings.

The shared shape scale is 12 points for controls, 16 points for ordinary cards,
and 20 points for high-emphasis containers. Fully rounded capsules are reserved
for status, filters, and compact metadata. Screens use 16-point outer margins
and 8-12 point vertical rhythm between related elements.

## Message Composition

Plain text direct delivery is the current mobile baseline. The composer keeps
attachments and delivery options visible because they are expected messaging
capabilities. Each unavailable action includes its mobile API dependency.

Message delivery disclosure separates three independent concepts:

- **LXMF method**: direct, opportunistic, propagated, or paper.
- **bearer**: RNode/LoRa, public TCP, WireGuard peer tunnel, or another interface.
- **evidence**: queued, sending, sent, delivered, failed, and receipt or resource
  completion details.

The UI must not derive the bearer from an LXMF method. It must not derive a
delivery state from transport activity. Preview threads can demonstrate labels
such as `Direct · LoRa · 2 hops`, `Direct · Public TCP`, and
`Direct · WireGuard peer`, but each example remains visibly marked Preview.

## Capability Settings

Settings distinguishes local host preferences from daemon capabilities. A local
preference can use an enabled control and persists on the host. A daemon-backed
control remains disabled until a typed mobile operation exists. Its reason names
the missing operation or projection.

The capability inventory includes messaging lifecycle and receipts,
attachments, delivery methods, propagation, Micron pages, interface policy,
notifications, background operation, identity custody, and diagnostics. Product
profile exclusions remain visible in About rather than appearing as controls.

## Pages

Mobile can call the basic `browse_page(host, path)` API. The first Pages surface
is an experimental source browser. It has a destination hash, native page path,
explicit fetch action, raw text result, and failure state. It does not
claim structured Micron rendering, link navigation, forms, downloads, cache
state, or page-host discovery. Those features require typed page sessions.

## Shared States

Both hosts use the same user-facing states:

- `offline`
- `starting`
- `ready` - the local node is available without a routable interface
- `connected`
- `degraded`
- `stopping` - shown by hosts that expose an explicit stop lifecycle

Discovery does not imply connectivity. An announced peer is `discovered` until
route or link evidence supports a stronger state. Geographic location and
network path are separate observations.

Data that does not come from the running daemon is visibly labeled **Preview**.
The mockup may use preview rows to exercise empty and high-information states,
but it must never present them as live mesh observations.

## Shared Actions

Both hosts use the same terms for an action when that action is available. A
listed action can be disabled on a host or build when it includes a clear reason:

- open conversation
- compose message
- announce identity
- refresh observations
- save or edit contact
- inspect person
- inspect network path
- configure connection
- copy or share public identity
- open propagation, pages, settings, or diagnostics
- inspect the local identity from Messages
- inspect message delivery method and bearer evidence

Unavailable actions remain visible only when their disabled reason teaches the
user something useful. Otherwise they are omitted.

## Platform Boundaries

The following remain host-specific:

- iOS Keychain, local-network permission, background task integration, signing,
  and SwiftUI navigation behavior.
- Android USB permission, RNode serial/KISS integration, secure identity store,
  background service behavior, and Compose navigation behavior.

Generated UniFFI bindings, XCFrameworks, JNI libraries, and local IDE settings
are build artifacts and are never source-controlled.

## Required Backend Projections

The interactive mockup identifies projections that the current mobile FFI does
not yet provide. Production wiring requires typed mobile records for:

- conversation preview, reply delivery hash, pin, mute, and retained draft.
- message lifecycle, requested and actual delivery method, attempts, receipts,
  authentication, stamps, and attachment transfer.
- person discovery age, route/link truth, capabilities, block state, and notes.
- interface type, state, traffic, signal, last observation, and failure reason.
- propagation peer, last synchronization, queue, transfer, and failure state.
- capability availability with disabled reasons.
- bounded, redacted diagnostic events and export metadata.

Until these projections exist, native hosts must not infer protocol truth from
display strings or fabricate successful lifecycle transitions.
