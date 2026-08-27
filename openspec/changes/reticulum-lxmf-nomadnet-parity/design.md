# Reticulum, LXMF, and NomadNet Parity Design

## Design goals

1. Protocol behavior remains in `styrene-rns`, `styrene-lxmf`, and `styrened`; clients consume typed contracts.
2. Upstream interoperability claims require upstream evidence. Rust-only tests prove internal behavior, not parity.
3. Production startup and test startup use the same protocol composition.
4. Every operator-visible state identifies source, observation time, generation, freshness, correlation, and support level.
5. Native Reticulum requests are the shared foundation for NomadNet and compatible LXMF propagation control flows.
6. Existing Styrene-specific page and propagation protocols remain explicitly named until native replacements are proven; they are not compatibility aliases.

## Claim model

Support is published per claim rather than as one `compatible` boolean:

| Claim | Minimum evidence |
|---|---|
| `rns.primitives` | Python-generated fixture agreement for covered bytes and crypto |
| `rns.operations` | Live path, link, request, resource, proof, loss, and recovery scenarios |
| `lxmf.codec` | Canonical payload, message ID, signature, paper, and propagation-envelope vectors |
| `lxmf.direct` | Bidirectional Direct and Opportunistic live scenarios with lifecycle evidence |
| `lxmf.resources` | Bidirectional resource-backed message completion and integrity |
| `lxmf.propagation` | Standard Python/Rust propagation discovery, offer, fetch, offline delivery, and restart |
| `micron.rendering` | Canonical fixtures and renderer assertions |
| `nomadnet.transport` | Native Python/Rust `/page` and `/file` scenarios in both directions |

Each claim is `unsupported`, `experimental`, `verified`, or `degraded`. `verified` requires enabled automated gates. A dated manual result may be retained as evidence but cannot produce `verified`.

## Layer boundaries

```text
styrene-dx / styrene-tui
        |
        | typed IPC commands, observations, capabilities
        v
styrened service and operation coordinators
        |
        +-- LXMF router coordinator
        +-- Native NomadNet page service/client
        +-- Propagation node/client
        |
        v
styrene-rns transport
        +-- destinations and announces
        +-- paths and forwarding
        +-- links and identification
        +-- requests and request receipts
        +-- channels and resources
```

UI code does not parse raw RNS packets, select LXMF wire representations, implement propagation exchanges, or manufacture page stages.

## Runtime composition contracts

Desktop, headless, mobile, and test runtimes retain independent startup and lifecycle ownership. There is no universal node factory and no runtime calls an interoperability runner.

Small protocol components may provide registration helpers where semantics and ordering must be identical, including:

- LXMF delivery and receipt correlation
- Native request-path registration
- Standard propagation handlers
- Native NomadNet page and file handlers
- Typed event and observation bridges

Each production entrypoint has startup contract tests proving that every advertised capability has its required destination, handler, worker, scheduler, and event bridge. Test-only components are permitted, but their evidence is internal and cannot satisfy a production parity claim.

Interoperability runners remain external. They launch shipped Rust artifacts through public command-line interfaces and observe them as black boxes alongside upstream protocol peers.

Each runtime publishes an `ActiveCapabilities` value derived from components that actually initialized. Optional-service failure produces a degraded capability with a reason. Client capability state is scoped to the IPC connection generation.

## Native Reticulum requests

### Server registry

Destinations own a request-path registry. Each entry contains:

- Canonical path and path hash
- Access policy: public, identified, allow-list, or callback
- Maximum request and response sizes
- Handler accepting request data, remote identity, link context, and request ID
- Response mode selected from packet or resource according to encoded size

Registration rejects duplicate paths and invalid limits.

### Client receipts

`Link::request` creates pending request state before transmission. State contains:

- Request ID and path hash
- Started/deadline timestamps
- Request and response sizes
- Progress and response-transfer state
- Optional response bytes
- Terminal status and protocol error

Response packets and resources correlate by request ID. Timeout, malformed response, cancellation, link close, and resource failure are terminal. Canonical access denial is recorded by the server and emits no wire response, so the remote receipt reaches its bounded timeout. Late responses cannot revive a terminal request.

### Link race correction

Pending outbound link state is inserted before sending the link request. If send fails, state is removed and completed with transport failure. Proof processing remains interface-bound and idempotent.

## Runtime observations

The transport runtime owns interface identity and counters. Configuration records do not stand in for runtime interfaces. Observation DTOs carry:

```text
source
observed_at
generation
freshness
entity identity
domain-specific state
correlation_id, when part of an operation
```

Path and link records include age and source. Closed links remain inspectable history but do not create active topology edges. Route expiry and route-loss events update both transport and clients.

## Scheduling and bounded failure

Channel retry polling, resource deadlines, request deadlines, receipt expiry, propagation work, and page request timeouts run under supervised schedulers owned by composition. Schedulers use an injectable monotonic clock in deterministic tests.

Every operation has:

- A bounded queue
- A deadline
- Cancellation behavior
- One terminal outcome
- Correlation shared across lower-layer events

## LXMF router coordinator

The coordinator owns outbound decisions and lifecycle rather than treating `styrene-lxmf` as only a codec.

### Outbound pipeline

1. Validate destination, content, fields, method, policy, and size.
2. Persist a queued canonical message and attempt record.
3. Select the requested LXMF method.
4. Apply documented method fallback only when LXMF requires it, recording requested and actual method.
5. Resolve path/link or propagation node as needed.
6. Select packet or resource representation.
7. Apply stamp/ticket policy.
8. Dispatch and correlate packet, resource, receipt, and propagation evidence.
9. Persist one terminal lifecycle outcome.

Direct, Opportunistic, Propagated, and Paper are separate paths. A missing propagation node cannot silently become Direct.

### Inbound pipeline

Canonical records retain protocol values independently from rendering projections. Authentication records distinguish `verified`, `invalid`, `unknown_identity`, and `not_applicable`. Stamp validation is similarly explicit. Unknown identity may trigger later verification, but canonical content is immutable.

### Receipt semantics

Transport send acceptance is not application delivery. For packet representation, an authenticated RNS delivery receipt whose packet hash exactly matches the tracked outbound packet transitions that LXMF message from Sent to Delivered. For resource representation, only verified completion of the exact tracked outbound resource transitions that LXMF message to Delivered. Unknown, untracked, forged, cross-message, duplicate, or representation-mismatched evidence cannot change the message to Delivered. There is no separate LXMF delivery-proof packet ingress.

### Conversation and attachment storage

Conversation projections, read state, contacts, pins, mutes, drafts, attempts, receipts, attachment metadata, and verified attachment blobs use file-backed storage. Message pagination uses stable keyset cursors and deterministic tie-breaking.

## Standard LXMF propagation

The standard propagation implementation is separate from Styrene's CBOR propagation messages.

It owns:

- `lxmf.propagation` destination and compatible announce metadata
- Standard request paths and identified-link authorization
- Offer comparison and transient-ID deduplication
- Encrypted transient payload ingest and retrieval
- Stamp validation and transfer limits
- Queue capacity and expiry policy
- Configured and automatically selected peers
- Peer synchronization checkpoints and failure history
- Restart-persistent queue and attempts

The existing metadata inventory API remains payload-free. Peer and sync support flags become true only when those domains are backed by this implementation. Operator message records link to propagation records by stable correlation, not by destination inference.

## Native NomadNet pages

NomadNet uses native Reticulum request paths, not Styrene LXMF page envelopes.

### Host

The page host registers:

- `/page/index.mu` and discovered `/page/...` paths
- `/file/...` paths for allowed files
- Dynamic page handlers with a bounded execution environment
- `.allowed` policy using identified link identity

Static packet-sized responses use response packets; larger pages and files use response resources. The `nomadnetwork.node` destination announces only after handlers are active.

### Client

A browse coordinator owns:

1. Address validation
2. Path discovery
3. Identity resolution
4. Link establishment and optional identification
5. Native request submission
6. Packet/resource transfer
7. Canonical source retention
8. Micron parsing
9. Rendering projection

Every stage emits typed state under one operation correlation. A failed stage prevents later stages from being marked complete.

### Dynamic fields and secrets

Submitted fields are carried in the native request data expected by NomadNet. Password values are never copied into activity records, debug formatting, history URLs, or evidence exports. Dynamic execution receives only the documented request environment and is bounded by timeout and output size.

## Operator workflows

The operator clients consume capabilities and observations from IPC:

- Network: announce, path request, probe, route, active/historical link, interface, and resource inspection
- Messages: requested/actual method, lifecycle, retry, cancellation, pagination, authenticity, stamps, resources, and propagation correlation
- Propagation: local queue, peers, offers, fetches, sync, capacity, expiry, attempts, and failures
- Content: verified page hosts, native browse stages, source/render, history, cache, forms, file transfers, and diagnostics

Mutation controls are disabled while capabilities are unknown or stale. Fixture backends implement the same contracts but are always identified as fixture evidence.

## Interoperability runner

The shared runner defines topology, pinned revisions, inputs, deadlines, milestones, assertions, cleanup, and artifact policy. It is called by CLI, CI, and Lab. Dioxus never supervises Python or daemon processes directly.

Required live packages:

- RNS three-node route, route loss, rediscovery, link, request, and resource
- LXMF Direct and Opportunistic in both directions
- LXMF resource-backed delivery in both directions
- LXMF propagation discovery, offer, retrieval, offline delivery, expiry, and restart
- NomadNet Python client to Rust host page/file
- NomadNet Rust client to Python host static/dynamic page

Evidence includes revisions, topology, operation correlations, milestones, assertions, timings, bounded logs, and checksums. Process exit is never the sole success assertion.

## Compatibility and rollout

IPC additions use new typed fields and capability versions. Persisted database changes use explicit migrations. Existing Styrene-specific page and propagation paths remain available only where concrete deployed consumers require them and are labelled separately; they are not fallback implementations for native requests.

Delivery order follows dependencies:

1. Claims, shared composition, runtime truth, and capabilities
2. Native request API, link race fix, schedulers, and routed evidence
3. LXMF authoritative delivery and receipt lifecycle
4. Standard propagation
5. Native NomadNet host/client
6. Operator workflows and final live gates

## Risks

- Python protocol behavior has implicit semantics not fully documented; pinned executable references are required.
- Live topology tests can become timing-sensitive; the runner must use milestones and virtualized bounded delays rather than arbitrary sleeps.
- Reticulum request work touches link and resource state machines; invariant/property tests precede live use.
- Standard propagation and Styrene propagation can be confused operationally; capability and UI naming must remain distinct.
- Dynamic pages execute local code; authorization, environment reduction, timeout, and output limits are release blockers.
