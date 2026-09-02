# Shared Frontend Session Design

## Reassessment

Every consumer now runs on the shared client. The client owns negotiation,
operation coverage, event fanout, compatibility polling, and generation
filtering. The `styrene-session` crate owns the Live, Embedded, and Fixture
profiles. The CLI, the TUI, and the `styrene-ui` desktop use those contracts
and no frontend imports server wire internals.

## Tested Revision Pair

- `styrene-rs`: `772a8eaa2e651f547f53700132c30ec4f03f6f86` (merge of #53).
- `styrene-ui`: pull request #16 from `feat/shared-client-sessions`, head
  `1baba22d7e3f18aad7c8a5e04311001fe50d9612`, merged as
  `d64b287ea999faceaaeb611d97082be3043318ec`, pins that revision for the
  desktop and mobile applications.

The pair advanced on 2026-09-02. `styrene-rs` moved to
`cf39251d6639e18cd5e88b0fa47d476d8c01aa0a` (merge of #57). `styrene-ui` moved
to pull request #17, head `795daec75b3c3139d01a494699a4073050c27e37`. That
pair adds Quick, Local, and Connected profile selection and backend profile
truth.

The pair advanced again on 2026-09-02 to the consolidated hardening corpus.
`styrene-rs` moved to `354a91cd494a7bac5703f7d1b128a2af95d08d8c` (merge of
#66). `styrene-ui` moved to pull request #18, merged as
`344796386711bc7b4ac5e038436d36d262c6eb93`, which pins that revision in all
three manifests with no source change. The library workspace, the desktop
application (118 passing tests), and the iOS check built and tested against it.

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
