# Reticulum 1.5 Parity Wave Design

## Authority And Provenance

The protocol authority and behavioral reference is `https://github.com/markqvist/Reticulum.git`. This plan is based on a read-only inspection of all 181 commits and all changed files in `b48b96e61676504e0a4e527b33b9a0b4495c6872..149e4151095adf098b8f53eab0c03b37169e8559`, followed by full relevant diffs for `RNS/Packet.py`, `RNS/Transport.py`, `RNS/Interfaces/Interface.py`, `RNS/Reticulum.py`, `RNS/Link.py`, `RNS/Resource.py`, `RNS/Discovery.py`, `RNS/Identity.py`, and `RNS/Cryptography/`.

The endpoint identities are immutable:

| Role | Revision | Release |
|---|---|---|
| Last pinned baseline | `b48b96e61676504e0a4e527b33b9a0b4495c6872` | RNS 1.4.2 |
| New behavioral authority | `149e4151095adf098b8f53eab0c03b37169e8559` | RNS 1.5.1 |

This wave owns the shared additive RNS fixture authority and schema. The implementation creates `tests/interop/fixtures/rns/index-v2.json` with `schema_version: 2`, an `authorities` object keyed by stable authority ID, and a `vectors` array. Each authority record contains `repository`, `revision`, and `release`; each vector contains `id`, `authority_id`, `kind`, `artifact`, `sha256`, `generator`, `source_symbols`, and `expected`. `source_symbols` is an array and `expected` is a typed JSON object so rejection class, decoded fields, bytes, or state transitions are represented without prose parsing.

The index contains both `rns-1.4.2` at `b48b96e...` and `rns-1.5.1` at `149e415...`. Existing 1.4.2 artifact paths, IDs, bytes, and checksums remain unchanged; migration adds index entries around them rather than replacing them. New vectors select `rns-1.5.1` explicitly. `provenance-v1.toml` remains readable for existing consumers during the additive schema migration, but new RNS vectors use the v2 index as authority. Committed fixtures contain only bounded packet bytes and metadata; no generated Python environment or upstream source tree is committed. Fixture generation is an explicit maintainer operation. Normal tests only read committed fixtures.

## Triage

| Upstream behavior | Decision | Current Styrene evidence and remaining gap |
|---|---|---|
| Reject short headers, incomplete hashes, zero data, excessive hops, and over-MTU frames | Adopt | `Packet::from_bytes` checks minimum structural length but accepts zero data and hop values at or above 128; `StaticBuffer::new_from_slice` silently truncates oversized payloads. |
| Increment received hops exactly once before transport processing | Adopt | Path insertion independently adds one, while destination `ReceivedData` exposes the unincremented wire value. Admission does not own one canonical post-ingress hop value. |
| Prioritized bounded inbound queues with per-class drops | Adopt | Interface ingress uses one bounded FIFO channel of 128 entries. It has no canonical traffic classes, configurable class capacities, strict-priority starvation contract, or per-class occupancy/drop snapshots. |
| In-flight same-destination path-request batching and bounded tag state | Adopt | Duplicate tags expire and recursive requests are scoped by `(destination, interface)`, but different tags/requesters can trigger separate discovery and only one requesting interface is represented per operation. |
| Path-request ingress/egress limits and slow-medium timeout | Adopt | Existing announce caps are not equivalent to 1.5 preemptive path-request limiting, and timeout configuration does not derive a minimum round trip from the slowest online bitrate. |
| Link MTU discovery enable/disable, route clamping, and outbound-interface proof timeout | Adopt | Link packets parse and sign MTU signaling, but transport/interface configuration does not own advertised hardware MTU, route-minimum clamping, disable policy, or bitrate-derived proof grace. |
| Cancel every active resource when a link closes | Adopt as regression | Resource orphan cleanup exists, but no adversarial test proves multiple simultaneous inbound/outbound transfers all terminate in one close pass. |
| Align resource receive-part search with the next requested window | Adopt | Receiver behavior is not pinned against the 1.5 out-of-order/window-boundary regression fixed by `65222e0d`. |
| Release receipt collection locks before invoking proof callbacks | Adopt as regression | Rust receipt callbacks are generally dispatched outside collection mutation, but there is no reentrant callback test proving a callback can send without deadlock. |
| Avoid mutation while iterating pending links and announce queues | Keep and prove elsewhere | Styrene uses map/snapshot cleanup and its existing announce limiter rather than Python's mutable lists; broad route-loss and announce behavior remains owned by `reticulum-lxmf-nomadnet-parity` tasks 5.6 and 5.7. |
| Respect destination-specific retained ratchet count | Skip as not applicable | The outbound ratchet store retains one latest remote ratchet per destination and has no Python-style destination ratchet list or configurable retained count. |
| Stream-backed resource size recovery and Python file API fixes | Defer | Styrene's current resource API is byte-owned rather than Python file-object based. Add only if a stream-backed public API is introduced. |
| Constant-time token HMAC comparison | Keep and prove | `fernet.rs` already uses constant-time verification paths. Retain a canonical valid/invalid-tag regression and forbid equality-based replacement. |
| Distinguish blackholed announces from protocol-invalid announces | Adopt | Admission and observations do not expose the canonical policy-drop versus malformed/invalid distinction. |
| Interface discovery implementation/version and operator LXMF address | Adopt | No native `rnstransport.discovery.interface` metadata codec exists. Implement the canonical MessagePack integer keys `0xFD`, `0xFC`, and optional `0xF0` with strict type/length checks. |
| Discovery autoconnect and all discovery interface families | Defer | Codec and truthful observations are required first; automatic network mutation is explicitly outside this wave. |
| Optimized HKDF/IFAC/HDLC, coalesced egress, profiler, CLI, and packaging | Skip | Performance or Python-runtime implementation changes do not alter Styrene's required wire behavior; existing canonical crypto/IFAC fixtures remain authoritative. |

No upstream review marker is advanced by this planned change.

## Admission Boundary

Raw frame validation occurs before conversion to fixed-capacity packet storage and before queue insertion. It validates interface frame length including IFAC overhead, exact Type 1/Type 2 structural fields, non-empty data, and wire hop count `< 128`. Oversized data is rejected, never truncated.

For physical ingress, the accepted packet receives one checked hop increment before classification, deduplication, path updates, link handling, or delivery. A local/shared-instance adapter may explicitly mark ingress as already local and suppress that increment; no later layer guesses or increments again. Outbound packets with hops `>= 128` are rejected. Observations report the post-ingress value and retain the wire value only as diagnostic evidence when needed.

Invalid framing, IFAC, signatures, pre-validation link traffic, and malformed path requests increment typed per-interface violation counters. A cryptographically valid announce from a blackholed identity is a policy drop and does not increment a protocol-invalid counter.

## Priority Ingress

The transport owns four FIFO queues, drained with canonical strict priority in this order:

1. data and general traffic;
2. announces;
3. path requests;
4. ingress-limited traffic.

After every completed dequeue, selection restarts at data. A lower-priority class is serviced only when every higher-priority class is empty at selection time. There is no fairness quota, aging, or starvation prevention: a continuously nonempty higher-priority queue may starve every lower class indefinitely. Sustained-load tests must prove both this starvation and immediate lower-class progress after all higher classes become empty.

Canonical default capacities are `1024`, `128`, `128`, and `8`. Positive configuration overrides are validated at startup. A full target queue drops the new item without blocking interface workers or displacing accepted data. Snapshots expose exact unsigned integer `capacity`, `depth`, and monotonic `dropped` values per class from one lock-consistent instant; no derived `pressure` field is part of the contract. Classification cannot promote a packet already marked ingress-limited.

## Path Requests

Tag replay protection remains keyed by destination plus the tag truncated to 16 bytes. Canonical `max_pr_tags` is a 16,000-entry rotation threshold, not a strict generation capacity: duplicates are checked against current and previous generations, and the insertion that crosses the threshold is retained before the current set becomes previous and a new current generation starts. The synchronously rotated Rust set therefore contains 16,001 entries. Rotation is count-based, not time-based; the older previous generation is discarded at rotation. Boundary tests cover entry 16,000, the retained crossing entry, duplicate rejection in both generations, and acceptance after the containing generation ages out.

A separate in-flight gate is keyed only by destination, permits exactly one entry per destination, and becomes eligible for pruning after canonical Reticulum 1.5.1 `PATH_REQUEST_GATE_TIMEOUT` of 45 seconds. Canonical Python does not impose a fixed total in-flight-map cardinality, so this plan does not invent one; its measurable bound is one entry per distinct destination admitted during the gate lifetime. Every first valid request establishes the gate before ingress limiting or pending-transmission admission. The first unrestricted request with pending capacity starts discovery; later valid requests for the same destination batch into the gate and do not emit another recursive request.

Waiters contain at most one entry for each currently registered requesting interface. Duplicate requests from one interface do not grow waiter state, and detached interfaces are removed or ignored before response. Thus the exact waiter bound for one destination is the current registered-interface count, not a new numeric constant. The pending discovery transmit queue has canonical maximum 32; when full, a new pending discovery item is not enqueued, its waiter remains eligible for an independently learned answer, and the drop is observable. Discovery waiters expire with their discovery request deadline while the independent destination gate remains active for its canonical lifetime. A later unrestricted request under a gate without waiter state recreates that state without emitting another recursive request.

Canonical 1.5.1 has no separate fixed-cardinality overflow branch for the in-flight map or per-destination waiter list, so this wave does not invent one or reject otherwise valid requests at an arbitrary count. Their exact bounds are identity and lifetime bounds: one in-flight entry per destination until pruning after the 45-second threshold and one waiter per currently registered interface. Static overflow behavior applies only to the four ingress queues and the 32-entry pending discovery transmit queue.

A matching announce responds once on every eligible requesting interface and atomically removes in-flight state. Destination-gate timeout and a direct/local answer release in-flight state; the shorter discovery timeout releases only its waiters. Ingress-limited duplicates cannot add unrestricted waiters. Recursive egress performs a pre-send limit check and a final check immediately before dispatch. Discovery timeout is at least the configured path timeout and at least one MTU round trip over the slowest online positive bitrate plus per-hop grace.

## MTU, Links, Resources, And Receipts

Task 6 exclusively owns optional online positive bitrate metadata and all bitrate-derived path-discovery and link-proof deadlines. It ignores offline, missing, and zero bitrate values and supplies the stable runtime bitrate contract consumed by later tasks. For lowest online positive bitrate `b`, canonical medium path grace is `2 * (500 * 8 / max(b, 5)) + 6` seconds and discovery uses `max(configured_path_timeout, medium_path_grace)`. With no positive online bitrate, medium path grace is `0` and the configured path timeout remains authoritative. Extra link-proof grace is `(500 * 8) / outbound_interface_bitrate` for a positive outbound bitrate and `0` otherwise, added to the existing per-hop establishment deadline. Calculations use checked duration arithmetic.

Task 7 consumes Task 6 bitrate metadata but does not define or mutate it. Task 7 owns optional positive hardware MTU metadata, link-MTU enable/disable policy, request signaling, route-minimum clamping, proof construction and validation, and packet/channel/resource payload derivation. Link MTU discovery defaults on and can be disabled globally or per composition. When enabled, link requests signal the initiator MTU; each forwarding hop clamps to the minimum supported MTU or removes signaling when an interface cannot participate. Proof validation authenticates the confirmed MTU and mode. Packet MDU, channel MDU, and resource SDU derive from the negotiated link MTU.

Link proof timeout grace uses the outbound interface bitrate and handles missing/zero bitrate without division. Link close snapshots all correlated resources before cancellation so mutation cannot skip every second entry. Resource receive-part matching starts at `consecutive_completed_height + 1` and searches the requested window. Receipt proof candidates are copied or otherwise selected under synchronization, then validated and callbacks invoked without holding receipt collection synchronization.

## Discovery Metadata

The codec preserves canonical integer keys and MessagePack types. Every emitted record includes interface type, transport flag and ID, implementation name (`Styrene`), implementation version, and sanitized display name. Optional operator address is exactly 16 bytes. Decode rejects wrong types and wrong-length addresses without partially persisting or auto-connecting. Reachability addresses are observations only in this wave.

Discovery stamp generation/validation and live announce exchange depend on the existing parity change's standard LXMF and live Reticulum gates. This wave owns deterministic codec fixtures and does not claim discovery transport interoperability until those dependencies pass.

## Evidence And Validation

Tests are layered:

- Canonical fixtures generated once from revision `149e415...` prove byte and classification behavior.
- Differential harnesses feed identical vectors to a pinned Python helper and Rust only in the dedicated live/fixture-generation environment.
- Adversarial Rust tests cover queue saturation, malformed frames, reentrancy, cancellation, and bounded state without Python.
- Focused component tests use virtual clocks and in-memory interfaces; no sleeps stand in for protocol milestones.

Retained evidence includes fixture manifests and checksums in Git plus bounded structured live artifacts from the existing interop runner. Ordinary `cargo test`, Clippy, and OpenSpec validation must not access a network, launch Python, or require hardware.

## Cross-Wave Ownership

| Change | Owns | Consumes from this wave | Order/dependency |
|---|---|---|---|
| `reticulum-1-5-parity-wave` | RNS fixture index v2, 1.5.1 authority, packet/queue/path/MTU/security/discovery behavior | Existing preserved 1.4.2 fixture artifacts | Producer; fixture schema and Task 1 land first |
| `beechat-rns-corrections-wave` | Beechat-specific correction decisions and tests | RNS fixture index v2 authority IDs and shared vectors | Consumer after this wave Task 1; must not fork schema |
| `freetak-rns-hardening-wave` | FreeTAK-specific hardening decisions and tests | RNS fixture index v2 authority IDs and shared vectors | Consumer after this wave Task 1; must not fork schema |
| `leviculum-rns-corpus-wave` | Leviculum corpus mapping and implementation-specific evidence | RNS fixture index v2 authority IDs and shared vectors | Consumer after this wave Task 1; adds references, not competing canonical authority |
| `reticulum-lxmf-nomadnet-parity` | Broad live RNS/LXMF/NomadNet interoperability and shared live runner | Completed offline behaviors and v2 fixture provenance | Live dependencies remain tasks 4.7, 5.7, 8.8, and 12.6 |

## Rollout And Dependencies

Implementation order is intentionally constrained:

1. Pin evidence and make raw admission/hop semantics fail closed.
2. Add priority queues and violation observations so later control-plane work is bounded.
3. Add in-flight path batching and canonical waiter/tag/pending-queue lifecycle.
4. Add online positive bitrate metadata and all adaptive deadlines.
5. Consume bitrate metadata while completing MTU signaling, clamping, proofs, and payload limits.
6. Land link/resource regressions, then receipt reentrancy, then token authentication.
7. Add the discovery metadata codec and observations.
8. Run focused offline verification, then the existing dedicated live gates after `reticulum-lxmf-nomadnet-parity` tasks 4.7, 5.7, 8.8, and 12.6 are available.

Each behavior can roll out behind existing transport configuration defaults where needed, but no compatibility path may silently accept malformed packets or unbounded state. Capability claims remain experimental until their required non-ignored gates pass.
