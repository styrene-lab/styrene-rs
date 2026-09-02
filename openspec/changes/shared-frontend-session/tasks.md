# Shared Frontend Session Tasks

## 1. Contract Inventory
<!-- specs: frontend-session/spec -->

- [x] 1.1 Inventory CLI, TUI, Dioxus, embedded mobile, `styrene-ipc`, and `DaemonFacade` operations and typed records
- [x] 1.2 Identify duplicate wire parsing, request brokering, polling, subscription, and generation behavior
- [x] 1.3 Add contract tests for operation parity, capability preservation, and typed failures
- [x] 1.4 Document the public crate boundary and dependency direction before moving code

## 2. Reusable IPC Client
<!-- specs: frontend-session/spec -->

- [x] 2.1 Create `styrene-ipc-client` without dependencies on Ratatui or Dioxus
- [x] 2.2 Move framing, opcode mapping, negotiation, request correlation, and typed decoding behind the client API
- [x] 2.3 Add bounded concurrency, deadlines, cancellation, event fanout, reconnect generations, and compatibility polling
- [x] 2.4 Add tests for overlap, out-of-order responses, timeout, cancellation, overload, disconnect, and stale generations
- [x] 2.5 Prevent frontend crates from importing `styrene-ipc-server::wire`

## 3. Frontend Sessions
<!-- specs: frontend-session/spec -->

- [x] 3.1 Define the common session profile, metadata, capability, generation, and shutdown contracts
- [x] 3.2 Implement `LiveSession` with the reusable IPC client and no Embedded fallback
- [x] 3.3 Implement `EmbeddedSession` with deterministic startup, ownership, and idempotent shutdown
- [x] 3.4 Implement a network-isolated `FixtureSession` for supported deterministic operations
- [x] 3.5 Verify Live and Embedded operations return equivalent typed daemon records

## 4. Ratatui Migration
<!-- specs: frontend-session/spec -->

- [x] 4.1 Replace TUI socket framing and payload-map parsing with the shared client
- [x] 4.2 Preserve TUI command authorization, generation checks, subscriptions, and polling behavior
- [x] 4.3 Remove obsolete TUI client code after parity tests pass
- [x] 4.4 Run TUI unit, integration, Live-failure, Embedded, and terminal smoke checks

## 5. CLI Migration
<!-- specs: frontend-session/spec -->

- [x] 5.1 Replace CLI socket framing and payload-map parsing with the shared client
- [x] 5.2 Preserve one-shot endpoint selection, operation deadlines, typed outcomes, and exit behavior
- [x] 5.3 Remove the obsolete CLI client after command and failure tests pass

## 6. Cross-Repository Dioxus Migration
<!-- specs: frontend-session/spec -->

- [x] 6.1 In `styrene-ui`, pin the reviewed `styrene-rs` revision and replace the Dioxus request broker and daemon bridge with the public client and sessions
- [x] 6.2 In `styrene-ui`, adapt existing stores without creating duplicate IPC domain records
- [x] 6.3 In `styrene-ui`, preserve explicit Live, Embedded, and Fixture profile behavior
- [x] 6.4 In `styrene-ui`, remove obsolete Dioxus client code after component and smoke tests pass

## 7. Verification And Release Boundary
<!-- specs: frontend-session/spec -->

- [x] 7.1 Run focused client, session, daemon, CLI, and TUI tests in `styrene-rs`, plus Dioxus tests in `styrene-ui`
- [x] 7.2 Run each repository's warning-denied Clippy, formatting, dependency-boundary, and documentation checks
- [x] 7.3 Verify ordinary workspace tests require no external daemon, Python runtime, or network access
- [x] 7.4 Publish or expose an immutable `styrene-rs` revision consumable by `styrene-ui` and record the tested revision pair
