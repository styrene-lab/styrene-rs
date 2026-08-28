# Extract Styrene UI Repository

## Intent

Move Dioxus application ownership into a dedicated `styrene-ui` repository while
preserving history and keeping protocol and daemon authority in `styrene-rs`.

## Scope

This change creates the GUI repository and extracts `styrene-dx` history. It
establishes workspace, dependency, compatibility, testing, and release boundaries.

This change excludes mobile feature parity, removal of native mobile hosts,
moving `styrene-tui`, moving daemon or protocol implementation, and duplicating
the interoperability runner.

This change depends on `shared-frontend-session` and is the prerequisite for
`shared-dioxus-mobile-ui`.

## Success criteria

- `styrene-ui` builds independently from an immutable `styrene-rs` revision.
- Extracted Dioxus files retain auditable source history.
- The desktop Live, Embedded, Fixture, and Protocol Lab behavior passes in the
  new repository.
- `styrene-rs` remains authoritative for daemon, IPC, protocol, transport, and
  interoperability contracts.
- `styrene-tui` remains operational in `styrene-rs` through the shared session
  boundary.
