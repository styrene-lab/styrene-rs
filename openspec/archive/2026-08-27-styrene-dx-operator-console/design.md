# Styrene DX Operator Console Design

## Design Principles

1. The daemon is authoritative. The UI derives views from daemon contracts and never infers protocol state from appearance or timing.
2. Runtime behavior is explicit. Live, Embedded, and Fixture profiles have distinct lifecycle and safety rules.
3. Pages consume domain stores, not raw IPC frames.
4. Queries, commands, subscriptions, and scenario controls have separate typed paths.
5. Every displayed fact carries enough provenance to explain where it came from and when it was observed.
6. Operate and Lab share rendering and inspection, but Lab-only mutation is capability-gated and visually distinct.

## Application Architecture

```text
Dioxus shell and routes
        |
        +-- Command / Network / Messages / Fleet
        +-- Propagation / Content / Lab / System
        +-- Activity drawer / Context inspector
        |
Domain stores and view models
        |
Application services
        +-- Query service
        +-- Command service
        +-- Subscription reducer
        +-- Scenario service
        |
BackendSession trait
        +-- LiveIpcSession
        +-- EmbeddedSession
        +-- FixtureSession
        |
styrened / styrene-ipc / interop harness
```

The UI crate owns presentation state, navigation, selection, filters, drafts, and scenario display state. It does not own RNS routing, LXMF delivery, propagation persistence, RBAC policy, or protocol retries.

## Runtime Profiles

### Live

Live connects to an explicitly configured daemon socket. Missing, stale, refused, incompatible, or unauthorized connections produce a recoverable connection screen. Live never starts an embedded daemon as a side effect.

### Embedded

Embedded starts `styrened` only after explicit selection. Its identity, configuration, storage, listener addresses, and cleanup policy are displayed before startup. Test-oriented ephemeral state is separate from persistent operator state.

### Fixture

Fixture runs without network access or a daemon process. It replays deterministic snapshots and event streams through the same `BackendSession` interface used by Live and Embedded. Fixtures cover empty, healthy, degraded, high-cardinality, and active-scenario states.

The selected profile is always visible in the application chrome. Switching profiles tears down the old session before activating the new one.

## Request Broker

The current `Arc<Mutex<DaemonBridge>>` serializes unrelated work and lets polling wait behind long requests. It is replaced by a broker with:

- One bounded outbound queue
- One writer task
- One reader task
- Correlation-ID keyed in-flight requests
- Independent event broadcast
- Per-request deadlines and cancellation
- Connection-generation IDs to reject stale responses after reconnect
- Backpressure and explicit overload outcomes
- Structured latency and failure metrics

The broker supports concurrent logical requests even if the wire connection has one physical writer. Long-running operations do not hold a global application lock.

Polling is a compatibility fallback. Incremental subscriptions and cursored snapshots are preferred. Repeated full 500-peer snapshots are not a steady-state refresh strategy.

## State Model

State is divided by domain:

| Store | Authoritative contents |
|---|---|
| `RuntimeStore` | Profile, connection generation, compatibility, daemon health |
| `IdentityStore` | Local identity, role, capabilities, policy summary |
| `NetworkStore` | Peer observations, routes, links, interfaces, traffic counters |
| `MessageStore` | Conversations, messages, receipts, drafts, transfer state |
| `FleetStore` | Managed devices, capabilities, jobs, privileged-action outcomes |
| `PropagationStore` | Peers, sync state, queue inventory, capacity, expiry, failures |
| `ContentStore` | Page hosts, navigation, source, render result, transfer diagnostics |
| `ScenarioStore` | Catalog, topology, run state, milestones, assertions, evidence |
| `ActivityStore` | Bounded normalized timeline with correlation and severity |

Each store has a reducer for snapshots and events. Stores expose selectors or page-specific view models so large peer or message collections do not trigger full-application rerenders.

## Observation Provenance

Network entities are not interchangeable:

- A **peer observation** means a valid announce or fixture observation was accepted.
- A **route** means the daemon path table has a next hop and hop count.
- A **link** means an RNS link has a lifecycle state.
- An **interface** means a configured transport interface has runtime state.
- An **association** may relate records but is not rendered as a packet path.

Every observation records source, observed time, generation, and freshness. The combined graph uses different edge styles and a legend. Discovery-only peers do not receive solid direct-route edges.

## Application Shell

Primary routes:

| Route | Primary purpose |
|---|---|
| Command | Operational summary, alerts, quick actions, active scenario |
| Network | Discovery, route, link, interface, and combined topology views |
| Messages | LXMF conversations, delivery lifecycle, receipts, resources |
| Fleet | Device inventory, status, remote jobs, policy-aware actions |
| Propagation | Queue, peers, synchronization, capacity, expiry, failures |
| Content | Local and remote Micron pages, source, render, diagnostics |
| Lab | Scenario catalog, topology, controls, timeline, assertions, evidence |
| System | Runtime profiles, identity, interfaces, policy, storage, diagnostics |

Persistent chrome contains profile, connection health, identity, active alerts, and activity-drawer access. A contextual inspector shows details for the selected peer, route, link, message, device, page request, or scenario milestone.

## Command Page

Command is the default Operate route. It answers:

- What runtime and identity am I operating?
- Is transport healthy?
- What changed recently?
- Which deliveries, links, resources, or sync jobs require attention?
- Is a Lab scenario active?

Cards are summaries linked to authoritative detail pages. Command does not duplicate full management surfaces.

## Network Page

Network provides Discovery, Routes, Links, Interfaces, and Combined modes. The existing force-directed graph is retained behind a graph-model boundary and decomposed into layout, rendering, interaction, filters, and inspector components.

Required controls include search, role, capability, freshness, status, anonymous-peer visibility, layer selection, layout pause, zoom, and fit. High-cardinality fixtures establish rendering and interaction budgets.

Context actions include message, browse content, request path, inspect announces, inspect route, and inspect link. Block, link creation, or other mutations require capability checks and confirmation.

## Messages Page

Messages separates conversation state from message and transfer lifecycle. It displays method, source, destination, status, receipt, retry, resource, propagation, and error information when available.

Operate mode provides ordinary composition and cancellation. Lab mode can reveal raw fields, stamps, tickets, encoded size, and correlated transport events without changing the canonical message path.

## Fleet Page

Fleet exposes only daemon-supported operations. Inventory is capability-driven rather than assuming every peer is manageable. Status, execution, reboot, profile application, remote inbox, blocking, and grouping are modeled as auditable jobs.

Privileged actions show target, capability, parameters, timeout, confirmation, request correlation, and terminal result. Unsupported operations remain visibly unavailable with the daemon reason.

## Propagation Page

Propagation distinguishes local store state, outbound propagation selection, remote peer state, synchronization, offers, fetches, downloads, expiry, and capacity policy. Queue records expose age, recipient, bytes, attempts, and terminal state without revealing sensitive payloads by default.

The page must work in a disabled state and explain what capability or configuration is missing.

## Content Page

The existing `styrene-micron` renderer is retained. Navigation uses a typed page address rather than ad hoc string splitting. Each request exposes stages: path discovery, identity resolution, link establishment, transfer, parsing, and render.

The page provides rendered and source views, history, local inventory, host metadata, request timing, transfer size, and actionable errors. Editing and publishing local pages are deferred until browsing and diagnostics are stable.

## Protocol Lab

Lab consumes a `ScenarioBackend` abstraction implemented by the same pinned harness used in CLI and CI. A scenario definition includes:

- Stable scenario ID and description
- Required implementation revisions or fixture provenance
- Topology and runtime profile
- Inputs and bounded deadline
- Ordered milestones
- Assertions
- Allowed operator controls
- Retained artifact policy

Lab can start, pause where supported, cancel, reset, and rerun scenarios. It can trigger only controls declared by the scenario, such as announce, path request, link open/close, message send, resource send, disconnect, or restart.

The timeline shows process lifecycle, network observations, protocol milestones, assertions, and failures. Completion uses the harness evidence report; the UI does not independently declare protocol success.

Initial Lab scenarios are fixture playback plus the pinned direct, opportunistic, routed, and topology-primitive interop cases. Later protocol packages register additional scenarios without modifying page internals.

### Lab Integration Boundary

Dioxus components never supervise reference processes directly. Fixture scenarios may execute in process behind `ScenarioBackend`; live scenarios execute through a structured runner boundary that owns topology allocation, child processes, deadlines, cleanup, and evidence. The desktop exchanges typed scenario commands and events with that boundary and remains responsive if the runner fails.

The runner must consume the same scenario definitions and harness implementation used by CLI and CI. Extracting a shared runner/library boundary is a prerequisite for live Lab execution; invoking test binaries or duplicating shell orchestration is not an acceptable integration.

## Activity And Diagnostics

All domains emit normalized activity records with timestamp, severity, kind, summary, entity reference, correlation ID, and optional structured details. The activity store is bounded and supports filters and export.

Diagnostics show request latency, queue depth, event lag, reconnects, dropped updates, fixture provenance, and scenario artifacts. Sensitive fields are redacted before entering UI state or export.

## System And Admin

System owns profile configuration and read/write daemon configuration workflows. Changes are validated before submission and display whether restart or reconnect is required.

Identity export, interface changes, RBAC changes, tunnel operations, terminal operations, and destructive storage actions require explicit capability checks and confirmation. Secret values are never round-tripped through generic debug views.

## Safety Boundaries

- Operate mode has no fault injection.
- Fixture mode cannot reach external network interfaces.
- Embedded mode does not bind production ports unless explicitly configured.
- Destructive actions are disabled while capability state is unknown.
- Reconnect invalidates in-flight commands from the prior generation.
- Scenario cancellation supervises and terminates every owned process.
- UI logs and evidence follow daemon redaction policy.

## Performance Budgets

- Command and page navigation remain responsive while 500 peers are loaded.
- No steady-state request waits two seconds for an application mutex.
- Network updates do not rebuild graph topology when only counters or freshness change.
- Activity and message collections are bounded or virtualized.
- Fixture mode includes a high-cardinality peer and event stream for regression tests.

Exact frame-time and memory thresholds are established from baseline measurements in the Foundation slice and recorded in tests or benchmark documentation.

## Testing

- Reducer tests cover out-of-order, duplicate, stale-generation, and reconnect events.
- Broker tests cover correlation, cancellation, timeout, backpressure, disconnect, and event fanout.
- Fixture tests cover every route and state class.
- Component tests cover navigation, filters, confirmations, disabled capabilities, and errors.
- Graph tests verify edge semantics independently of coordinates.
- Desktop smoke launches Fixture, Live-failure, and Embedded profiles.
- Lab smoke runs fixture scenarios without Python and pinned live scenarios in the dedicated interop gate.
- Accessibility checks cover keyboard navigation, focus, labels, contrast, and reduced motion.

## Migration

The current app remains launchable while slices are introduced. Existing behavior is moved behind new interfaces before visual redesign:

1. Wrap the current bridge as `LiveIpcSession` and introduce explicit startup selection.
2. Add Fixture session and routed shell.
3. Move event handling into reducers and stores.
4. Move graph construction behind typed network observations.
5. Migrate Messages and Content.
6. Add Fleet, Propagation, Lab, and System routes.
7. Remove compatibility bridge state and obsolete monolithic handlers.

No production protocol changes are required to begin. Missing incremental daemon contracts are added as separate, typed daemon changes rather than emulated with increasingly expensive UI polling.
