# Shared Frontend Session

## Intent

Give Ratatui, Dioxus, and embedded hosts one typed frontend session contract.
Remove the duplicate IPC clients and prevent frontend code from depending on raw
wire frames or daemon application internals.

## Scope

This change adds a reusable IPC client, a transport-neutral frontend session,
explicit Live and Embedded lifecycle adapters, generation-safe subscriptions,
bounded request brokering, and contract fixtures. It migrates `styrene-tui` and
the in-tree `styrene-dx` application to that boundary.

This change excludes creating the new GUI repository, redesigning either UI,
and moving protocol behavior into a frontend crate.

This change is the prerequisite for `extract-styrene-ui-repository`.

## Success criteria

- Ratatui and Dioxus issue commands, queries, and subscriptions through the same
  reusable client and session contracts.
- Live and Embedded sessions expose the same typed daemon operations.
- Request concurrency, bounds, deadlines, cancellation, and connection
  generations have shared tests.
- Frontend applications no longer parse raw IPC maps or import server wire
  framing.
- Existing TUI and Dioxus Live, Embedded, Fixture, and failure smoke tests pass.
