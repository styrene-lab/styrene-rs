# Beechat RNS Corrections Wave Design

## Evidence and authority

The Beechat range was reviewed through its full commit list, file-level delta, and behaviorally
relevant full diffs. The range is useful because it is in Styrene's MIT lineage and contains small
Rust reproductions of protocol failures. It is not an architectural upstream.

Protocol decisions come from canonical Python Reticulum 1.5.1 at
`149e4151095adf098b8f53eab0c03b37169e8559`. In particular:

- `Link.validate_proof()` packs RTT with MessagePack from a Python float, and `Link.rtt_packet()`
  consumes that value. The interoperable Rust representation is `f64`.
- `Transport.packet_filter()` rejects every non-announce packet whose transport ID identifies a
  different transport instance before normal processing.
- `UDPInterface.process_outgoing()` enables `SO_BROADCAST`.
- Offline TCP interfaces do not transmit or preserve a store-and-forward backlog.

Earlier canonical revisions that introduced a behavior may be cited as secondary evidence, but
they do not replace the 1.5.1 endpoint. Implementation reconciles with the current consolidation
branch; it does not require or perform a branch rebase.

This change consumes the fixture manifest and corpus established by
`reticulum-1-5-parity-wave` tasks 1.1-1.3. It depends on that change's tasks 2.1-2.3 for the raw
Type-1/Type-2 frame admission and received-hop boundary, and coordinates with tasks 4.1-4.3 so the
next-hop decision runs before bounded queue insertion. It creates no second top-level Reticulum pin
or fixture authority.

## Triage

| Concern in reviewed range | Decision | Current evidence and rationale |
|---|---|---|
| LinkRTT handling | Adopt correction only | Styrene handles RTT in both link directions but writes and reads MessagePack `f32`; Python emits a 64-bit float. |
| Type-2 next-hop admission | Adopt, strengthened to canonical behavior | Styrene suppresses generic LinkRequest/Proof rebroadcast but does not reject a non-announce transport packet addressed to another transport instance before routing. Canonical Python applies the gate to all non-announces, not only Beechat's narrower packet set. |
| Removal of free-form transport broadcast forwarding | Reconcile on the current branch | The current `broadcast` switch can rebroadcast Data generically. The existing directed route and interface policy remain authoritative; no second forwarding path may survive. |
| `no_std` clock | Adopt contract, not Beechat mechanism | The branch declares `no_std` but core destination code imports `std`; Beechat's Embassy clock and panic-before-init API are not adopted. Canonical Reticulum 1.5.1 behavior instead requires explicit `TimeSourceUnavailable` behavior and an embedding-provided clock. |
| UDP broadcast permission | Adopt | Current UDP uses Tokio bind directly and never enables `SO_BROADCAST`. |
| Disconnected TCP transmit queue | Adopt policy | Current TCP reconnect sleeps without draining its channel, allowing the shared dispatcher to fill and wait. Dispatch and the driver require one connection-epoch contract to reject offline or stale traffic without replay. |
| Announce timestamp, current-entry update, local-announce rejection, and retry scheduling | Skip as already present | Current destination and announce-table code includes the five-byte timestamp suffix, current announce replacement, local rejection, and normal/old retransmit scheduling. |
| Link stale/close, teardown, closed-send rejection, touch/keepalive, and identification | Skip as already present | Current `Link` has RTT-driven watchdogs, close packets, closed-state send rejection, activity anchors, and remote identification. |
| Link message proof key and inbound-link proof handling | Skip as already present | Current proof validation uses the peer identity and processes both inbound and outbound links with focused tests. |
| Channels and returned receiver behavior | Skip as superseded | Styrene has a scheduler-integrated typed channel implementation and channel buffer; Beechat's channel API shape is not imported. |
| IFAC-open forwarding fix | Skip as already present | Forwarded path/link packets preserve open IFAC policy; Reticulum 1.5.1 early IFAC admission remains authoritative. |
| Flexible rerouting, automatic out-link restart, and announce-forever options | Skip | These are Beechat product policies, not protocol requirements, and can conflict with canonical route expiry and scheduler behavior. |
| Daemon/config conversion, examples, docs, CI, public serde, crate split, Kaonic removal, typo rename, and lint-only commits | Skip | They are already present, superseded, non-behavioral, incompatible with Styrene structure, or unnecessary public API churn. |

## Decisions

### Ownership and dependency order

1. `reticulum-1-5-parity-wave` tasks 1.1-1.3 establish the sole 1.5.1 fixture provenance and corpus.
2. Its tasks 2.1-2.3 establish raw frame admission and canonical received hops on the current branch.
3. This change solely owns the LinkRTT `f64` codec, accepted numeric domain, complete-consumption
   check, and its fixture/interop assertions.
4. This change solely owns the Type-2 transport-ID next-hop decision, integrated into that admission
   boundary before queue insertion; `reticulum-1-5-parity-wave` tasks 4.1-4.3 consume the admitted
   result and do not duplicate the decision.
5. Embedded time, UDP broadcast capability, and TCP connection-epoch discard policy follow after
   those shared boundaries and remain owned here.

### Encode LinkRTT as f64

RTT remains represented internally as `Duration`. Only the encrypted LinkRTT wire payload changes:
encode `Duration::as_secs_f64()` with MessagePack `f64` and decode the canonical numeric value into
`Duration::from_secs_f64`. Non-finite, negative, malformed, or trailing payloads are rejected without
activating or refreshing a link. Tests consume immutable 1.5.1 vectors from the shared fixture
authority and use the existing dedicated live pinned-Python gate where live evidence is required;
this change adds no independent pin or manifest. Rust-only round trips are insufficient
interoperability evidence.

### Admit transported packets before mutable state

This change owns the next-hop check inside the Reticulum admission boundary. Any non-announce
packet with a transport field unequal to the local transport identity is dropped before queue or
duplicate-cache insertion, path/link-table mutation, cryptography, delivery, or egress. Announces
retain their
separate transported-announce semantics. Matching Type-2 packets continue through the single
canonical routing path.

No special exception is added for Proof, Link, or a locally hosted destination: canonical Python's
transport identity gate is authoritative. Shared-instance handling must follow the canonical 1.5.1
decision and tests rather than Beechat's local-destination workaround.

### Keep forwarding single-owner and directed

Route lookup selects the next interface and rewrites Type-1/Type-2 headers. Interface policy decides
which broadcast egress is allowed. The ingress worker must not also rebroadcast packets generically.
The regression topology uses three nodes on one shared medium plus a destination behind the relay;
an overhearing non-next-hop node emits nothing and the relay emits one directed forwarding action.

### Use the canonical embedded-time contract

The core must compile without `std`. Announce creation and ratchet rotation share one wall-clock
source. In `std`, system time is the default. In `no_std`, the embedding application supplies or
refreshes whole-second Unix time; before initialization, timestamp-dependent operations return a
typed error. The plan rejects Beechat's hidden Embassy dependency, boot-offset underflow, and panic
on missing initialization.

### Treat UDP broadcast as socket capability, not routing policy

An IPv4 UDP socket with a forwarding target enables `SO_BROADCAST` before the first send. This does
not add a transport-wide broadcast switch and does not affect IPv6 or receive-only sockets.
Configuration and interface-mode rules still decide the target address and whether a packet may be
sent.

### Discard disconnected TCP traffic by connection epoch

TCP is not a persistence boundary. Each client owns a checked, monotonically increasing `u64`
`connection_epoch`. A successfully established stream receives the next epoch before carrier state
is published online. Carrier admission is the atomic tuple `(online, connection_epoch)`. Every
accepted transmit item is tagged with the observed epoch, and a writer may send it only on the
stream with that same epoch.

Disconnect atomically publishes offline for the active epoch before pending items are discarded.
New dispatches then fail immediately. Reconnect publishes a new epoch and cannot consume items from
an older epoch. Checked increment must not wrap and alias stale traffic to a future connection.
Broadcast dispatch may still succeed on healthy interfaces and reports per-interface sent/failed
counts truthfully. Durable application retries remain owned above the interface.

Deterministic barrier-controlled tests cover dispatch racing disconnect before enqueue, an old
writer racing a new stream publication, reconnect racing queue drain, and simultaneous stale/fresh
items at the epoch boundary. Every race must either reject or discard the stale item, preserve fresh
same-epoch traffic, and avoid blocking healthy interfaces.

## Risks

- Applying the gate after duplicate-cache insertion can poison retries even if no packet is
  forwarded.
- Copying Beechat's proof/link exceptions would remain weaker than canonical Python behavior.
- Treating a disconnected queue enqueue as success can produce false delivery evidence.
- Checking `online` separately from `connection_epoch` permits stale traffic to cross a reconnect.
- A process-global test clock can create parallel-test races; time tests need the canonical clock
  isolation mechanism.
- UDP broadcast tests can be platform-sensitive; socket-option assertions are deterministic and a
  network send is supporting evidence, not the sole gate.

## Validation

Focused validation runs before broader checks:

- Link tests against the shared 1.5.1 fixture corpus and the existing pinned-Python gate
- Reticulum 1.5.1 ingress/admission and shared-medium topology tests
- `cargo check -p styrene-rns --no-default-features` (or the current-branch owning core package)
- UDP socket and TCP offline-queue tests
- `cargo test -p styrene-rns --features transport`
- Workspace formatting, warning-denied Clippy, offline tests, and the repository's pinned-Python
  interoperability gate

OpenSpec validation proves artifact structure only and is reported separately.
