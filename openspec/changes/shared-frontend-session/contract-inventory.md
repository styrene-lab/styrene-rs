# Frontend Session Contract Inventory

Original inventory revision: `f2a5999893970fe4b9677db4bd671c8a006d4f47`

Reassessed against backend revision `23beb83dbed95165347debdaabb1a672febfdc92`
and UI revision `6a1143665ff2afbc3da076d6b1c3eb326f3fe527`.

## Authorities

`styrene-ipc::Daemon`, its component traits, typed records, and `IpcError` are
the transport-neutral application contract. `styrened::DaemonFacade` is the
authorized implementation of that contract. Frontend code must not create a
second daemon contract from wire maps or implementation fields.

`styrene-ipc-server` is a Unix-socket adapter. Stable framing now lives in
`styrene-ipc-wire`, and the one-shot CLI uses `styrene-ipc-client`. The TUI still
imports server wire internals directly, and the desktop application remains in
the independent `styrene-ui` repository with its existing broker and session
logic. Those two consumers remain migration work.

## Current Consumers

| Consumer | Access path | Request behavior | Events and generation | Main duplication |
|---|---|---|---|---|
| `styrene` CLI | Shared `styrene-ipc-client` connector in `crates/apps/styrene/src/ipc_client.rs` | Bounded typed requests through the public client | No subscriptions or reconnect required for one-shot use | Endpoint selection only |
| `styrene-tui` | Sequential Unix client in `crates/apps/styrene-tui/src/daemon.rs` | One globally serialized command stream, fixed five-second IPC timeout | Separate event stream, ten-second polling, generation checks in presentation reducer | Framing, maps, compatibility parsing, subscriptions, reconciliation, authorization checks |
| `styrene-dx` | `BackendSession` and broker in `styrene-ui/apps/desktop/src` | Bounded concurrent broker, per-request deadlines, pending map, cancellation cleanup | Separate event and polling streams, frontend and daemon generations | Framing, typed decoding, profile lifecycle, duplicated daemon records |
| Dioxus mobile | `styrened::mobile::MobileNode` through `apps/mobile/src/session.rs` in `styrene-ui` | In-process typed calls on a bounded worker | Backend, UI-session, and platform generations | Specialized mobile DTOs and one direct `DaemonFacade` field access |

Live and Embedded DX profiles in `styrene-ui` already use the same `IpcBackend` operations.
Their intended difference is ownership: Live owns client tasks, while Embedded
also owns the daemon runtime and temporary resources. Fixture implements a
limited deterministic operation set and opens no daemon or external interface.

## Operation Families

The canonical `Daemon` traits cover identity, messaging, status and discovery,
fleet and terminal operations, events and network observations, tunnels, and
pages and downloads. The frontend clients expose overlapping subsets:

| Family | CLI | TUI | DX | Mobile host |
|---|---|---|---|---|
| Identity, status, discovery | Yes | Yes | Event and poll projection | Yes |
| Conversations, messages, drafts | Partial | Broad | Broad | Broad |
| Contacts and conversation management | No | Partial | Backend-ready, mostly unsurfaced | Partial |
| Routes, interfaces, links, requests, resources | Partial | Broad | Broad | Mobile snapshot subset |
| Propagation client | Read-only | Read-only | Read and queue inspection | Selection and synchronization |
| Fleet, terminal, tunnels | Broad CLI subset | Broad | Fleet subset | Not a mobile product surface |
| Pages and downloads | No | Broad | Broad | Page browse subset |

The wire adapter does not yet provide clean parity for revision-safe draft
clearing, metadata-only attachment listing, peer search and bookmarks, tunnel
rekey and SA listing, tunnel-operation queries, or page-host listing. Contract
tests must record these as explicit exclusions until opcodes and dispatch exist.

## Duplicate Mechanics

The TUI and DX still own overlapping request, compatibility, subscription, and
projection mechanics. The CLI migration removed its duplicate framing and typed
response parser. Remaining behavior differs materially:

- TUI serializes all calls behind one mutex. A slow operation blocks polling and
  later commands, and a late timed-out response leaves the stream ambiguous.
- DX has the strongest implementation: a capacity-32 concurrent broker,
  generation-prefixed request identifiers, per-call deadlines, out-of-order
  response correlation, cancellation cleanup, stale-response rejection, and
  disconnect propagation.
- TUI and DX each open independent command and event connections and implement
  their own subscription setup, polling, gap reconciliation, and generation
  filtering.
- TUI and DX independently parse identity, status, capabilities, devices,
  conversations, messages, and errors. Compatibility behavior can drift between
  the repositories.

The reusable client must preserve the DX broker guarantees rather than adopting
the sequential CLI or TUI transport as its foundation.

## Generation Domains

The current code uses distinct generation domains that must remain distinct
types or fields:

- IPC physical connection generation, assigned by `styrene-ipc-server`.
- Daemon runtime generation, derived by `DaemonFacade` from active interfaces.
- Frontend session generation, assigned when a Live, Embedded, or Fixture
  session is opened or replaced.
- Mobile UI and platform generations, owned by `styrene-ui` presentation and
  platform adapters.

The client owns IPC connection generations and rejects stale responses and
events. The session owns frontend generation and replacement. Daemon runtime
generation remains a typed backend fact. Presentation reducers may reject stale
session updates but must not reinterpret one generation domain as another.

## Public Dependency Direction

The migration uses these boundaries:

```text
styrene-ipc --------+--------------------+
                    |                    |
styrene-ipc-wire ---+-> styrene-ipc-client -> styrene-session
                    |                           |
                    +-> styrene-ipc-server      +-> styrene / styrene-tui / styrene-ui desktop

styrene-ipc <-- styrened::DaemonFacade <-- styrened::mobile::MobileNode
                                              ^
                                              +-- styrene-ui mobile session
```

`styrene-ipc-wire` owns stable frame types, opcode discriminants, request-ID
width, payload bounds, and codecs. It has no daemon implementation, socket
listener, renderer, or runtime lifecycle dependency.

`styrene-ipc-client` depends on `styrene-ipc` and `styrene-ipc-wire`. It owns
endpoint connection, negotiation, typed operation encoding and decoding,
request correlation, bounds, deadlines, cancellation, subscriptions,
compatibility polling, generation filtering, diagnostics, and typed client
errors. It has no Ratatui, Dioxus, daemon implementation, or presentation-state
dependency.

`styrene-session` owns common Live, Embedded, and Fixture profile metadata,
capabilities, frontend generation, event delivery, replacement, and idempotent
shutdown. It returns canonical `styrene-ipc` records. Host-specific path and
onboarding policy remain in applications.

`styrene-ipc-server` depends on `styrene-ipc` and `styrene-ipc-wire`. Frontend
applications must not depend on it after migration.

Mobile remains an in-process backend host, not a second wire client. Mobile-only
boot, secure custody, bearer arbitration, RNode handoff, lifecycle, and
propagation synchronization remain backend-owned specialized operations. The
mobile host must expose explicit typed methods for shared facts such as identity
and capabilities; `styrene-ui` must not access public `DaemonFacade` or
`AppContext` fields.

## Record Ownership

Canonical IPC records remain in `styrene-ipc`. Migration removes or narrows
these frontend duplicates:

- DX `ConversationInfo` becomes `styrene_ipc::ConversationInfo`.
- DX `PathTableEntry` becomes `styrene_ipc::PathInfo`.
- DX transport portions of `DaemonEvent` become shared client or session events.
- DX `PageResponse` returns `PageContent` directly.
- CLI, TUI, and DX response parsers become one typed client decoder per
  operation.

Renderer-specific rows, selection state, activity text, sorting, redaction, and
disabled-reason presentation remain in each frontend.

## Migration Order

1. Add contract tests for operation coverage, typed errors, capabilities, and
   documented wire exclusions.
2. Extract the neutral wire module without changing frame bytes or opcode
   discriminants.
3. Create `styrene-ipc-client` from the DX broker and move typed operation
   codecs behind it.
4. The one-shot CLI migration is complete. Migrate the TUI, then remove its
   raw-wire imports and duplicate parsers.
5. Add common Live, Embedded, and Fixture session lifecycle contracts with
   explicit task ownership and idempotent shutdown.
6. Migrate `styrene-ui` desktop stores to canonical records and shared session events.
7. Replace mobile UI facade access with an explicit mobile-host method and adapt
   common capability and generation facts without moving mobile-only behavior
   into UI code.

No private client or parser is removed until its operation, compatibility, and
failure tests pass through the shared implementation.
