# Shared Frontend Session Design

## Reassessment

Every consumer now runs on the shared client. The client owns negotiation,
operation coverage, event fanout, compatibility polling, and generation
filtering. The `styrene-session` crate owns the Live, Embedded, and Fixture
profiles. The CLI, the TUI, and the `styrene-ui` desktop use those contracts
and no frontend imports server wire internals.

## Tested Revision Pair

- `styrene-rs`: `be869620b792a3d1610438b6c3b1deb2d40448e8` (merge of #53).
- `styrene-ui`: pull request #16 from `feat/shared-client-sessions`, head
  `f00fb20da8ba8e7a31609d6a8b4888cd5595ed00`, merged as
  `ecbac266b8bf743958df659bbc75f015d197221b`, pins that revision for the
  desktop and mobile applications.

Desktop validation at that pair covered `cargo test -p styrene-dx` with 115
passing tests and the ignored Live-failure smoke run explicitly. It also
covered warning-denied Clippy with the CI exclusions, workspace tests, and the
mobile host and iOS target checks.

## Authority

`styrene-ipc::Daemon` and its typed records remain the application contract.
`styrened::DaemonFacade` remains the implementation authority. The new client
and session crates adapt access to that authority. They do not define a second
set of conversation, message, network, or capability records.

## Crate Boundaries

`styrene-ipc-wire` owns stable frame types, opcode discriminants, payload bounds,
request identifier representation, and codecs used by both socket adapters. It
contains no listener, daemon implementation, frontend, or lifecycle policy.

`styrene-ipc-client` owns remote client mechanics:

- Wire framing and opcode mapping.
- Request correlation, deadlines, cancellation, and bounds.
- Independent event subscription and compatibility polling.
- Connection negotiation and generation changes.
- typed errors and diagnostics.

It depends on `styrene-ipc` and `styrene-ipc-wire`, not on
`styrene-ipc-server`.

`styrene-session` owns frontend lifecycle selection:

- `LiveSession` wraps `styrene-ipc-client`.
- `EmbeddedSession` owns a `styrened` runtime and exposes the same daemon contract.
- `FixtureSession` implements supported deterministic operations without network access.
- A common session handle exposes profile, generation, capabilities, and shutdown.

The exact crate names can change only if the same ownership boundary remains
clear and no frontend imports server wire implementation details.

## TUI And Dioxus Use

`styrene-tui` remains in `styrene-rs`. It replaces its manual socket framing and
payload parsing with the shared client. Its terminal-specific state and Ratatui
widgets remain local.

`styrene-ui` replaces its private request broker and daemon bridge with the same
public client and session contracts at a declared immutable `styrene-rs`
revision. Its stores, reducers, routes, Dioxus components, and validation remain
in that repository.

The one-shot `styrene` CLI also replaces its sequential raw-wire client and
manual parsers. Command formatting and process exit behavior remain local.

The shared boundary does not require Ratatui and Dioxus to share presentation
reducers. It requires them to consume identical typed daemon semantics.

## Embedded Mobile Use

Rust Dioxus mobile code consumes a specialized backend-owned embedded host. It
shares canonical daemon records, capabilities, generation facts, and lifecycle
semantics with `EmbeddedSession`, but retains mobile-only boot, custody, bearer,
RNode handoff, and propagation synchronization operations. Rust platform
services provide OS integration without defining a second daemon API.

The mobile host exposes explicit typed methods. UI code does not access
`DaemonFacade` or `AppContext` fields directly.

## Failure Behavior

Live connection failure never starts an embedded daemon. Reconnect increments a
generation and cancels stale in-flight operations. Queue exhaustion, deadline,
cancellation, protocol incompatibility, authorization, and disconnection remain
distinct typed outcomes.

Embedded startup failure releases all resources created by that attempt.
Closing a session is idempotent and waits for owned runtime shutdown.

## Compatibility And Rollout

The reusable client first gains parity tests against the existing TUI client in
`styrene-rs` and the one-shot CLI. The CLI and Ratatui migrate first to prove the
contract in independent consumers. `styrene-ui` then pins the reviewed backend
revision and migrates Dioxus in its own repository. Each private client is
removed only after its owning repository's smoke and contract suites pass
through the shared implementation.
