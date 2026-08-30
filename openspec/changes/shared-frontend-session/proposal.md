# Shared Frontend Session

## Intent

Give Ratatui, Dioxus, and embedded hosts one typed frontend session contract.
Remove the duplicate IPC clients and prevent frontend code from depending on raw
wire frames or daemon application internals.

## Scope

This change adds a reusable IPC client, a transport-neutral frontend session,
explicit Live and Embedded lifecycle adapters, generation-safe subscriptions,
bounded request brokering, and contract fixtures in `styrene-rs`. It migrates
`styrene-tui` and the `styrene` CLI in this repository and coordinates
`styrene-ui` adoption through public, immutable `styrene-rs` contracts. Mobile
uses the common typed semantics through its specialized embedded backend host;
mobile-only lifecycle and bearer operations remain backend-owned extensions.

This change excludes redesigning either UI and moving protocol behavior into a
frontend crate. Dioxus source and validation remain owned by `styrene-ui`.

The Dioxus authority transfer is complete. Remaining shared-session work is a
cross-repository contract migration, not an in-tree Dioxus migration.

## Success criteria

- Ratatui and Dioxus issue commands, queries, and subscriptions through the same
  reusable client and session contracts.
- Live and Embedded sessions expose the same typed daemon operations.
- Request concurrency, bounds, deadlines, cancellation, and connection
  generations have shared tests.
- Frontend applications no longer parse raw IPC maps or import server wire
  framing.
- Existing TUI checks pass in `styrene-rs`, and the corresponding Dioxus Live,
  Embedded, Fixture, and failure checks pass in `styrene-ui` against the declared
  backend revision.
