# Mobile UI Contract

The iOS and Android applications are one Rust-owned Dioxus product. They share
components, information architecture, terminology, state semantics, fixtures,
and action availability. Maintained Swift and Kotlin hosts or adapters are not
part of the product.

## Product Shape

The primary navigation has four destinations:

1. **Messages** provides conversations, unread state, threads, drafts, send,
   recovery, and delivery evidence.
2. **People** combines saved contacts and discovered peers with identity, route,
   and trust details.
3. **Network** presents identity announcements, active transports, connection
   configuration, and bounded diagnostics.
4. **More** contains identity, propagation, pages, settings, diagnostics, and
   capability disclosures.

Fleet control, terminals, unrestricted administration, and Lab workflows are
outside the base mobile product.

## Authority

The embedded Rust runtime owns identity, protocol, storage, routing, delivery,
and transport truth. Renderer-neutral Rust reducers own presentation state.
Dioxus components render that state and dispatch typed Rust actions.

The UI must not infer a bearer from an LXMF method, infer delivery from transport
activity, or treat preview records as live observations. Preview and fixture
sessions remain visibly marked.

## Message Composition

Plain text is the mobile minimum. Delivery disclosure keeps these concepts
independent:

- LXMF method: direct, opportunistic, propagated, or paper.
- Bearer: RNode/LoRa, public TCP, tunnel, or another interface.
- Evidence: persisted, queued, sent, propagation upload, delivered, or failed.

A failed send preserves the current draft. A successful completion clears only
the draft revision that initiated that send.

## Shared States

The product exposes typed stopped, starting, connecting, connected,
reconnecting, degraded, and failed states. Discovery does not imply
connectivity. Late work from an old connection generation cannot mutate the
current session.

## Pages

The initial Pages workflow calls the typed `browse_page(host, path)` Rust API and
shows source, request provenance, and explicit failure. It does not claim
structured Micron rendering, forms, downloads, caching, or host discovery.

## Platform Services

Platform lifecycle, Bluetooth, Android USB, secure storage, permissions,
notifications, and packaging are exposed through typed Rust services owned by
`styrene-ui`. Generated platform scaffolding is disposable build output and is
not maintained in `styrene-rs`.

Unavailable operations return typed reasons. Platform services do not own
navigation, product state, protocol state, or daemon semantics.

## Required Projections

Production UI consumes typed Rust records for conversations, messages, peers,
interfaces, propagation, capability availability, and bounded redacted
diagnostics. Missing projections remain explicitly unavailable; the UI never
fabricates successful lifecycle transitions.
