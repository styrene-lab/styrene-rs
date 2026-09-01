# FreeTAK RNS Hardening Wave Design

## Reassessment

The archived Reticulum 1.5 parity wave already owns and verifies canonical
constant-time `Fernet` and `CachedFernet` authentication. This wave consumes that
result and must not create another implementation or fixture schema.

FreeTAK admission remains incomplete. Open groups cover secure key persistence,
fallback classification, poisoned receipt recovery, Link mutation ordering, and
bound-interface dispatch. Resource lifecycle, task supervision, and
internal-interface policy also remain open. The completed shared RNode engine
boundary does not close those groups.

## Evidence and authority

The reviewed implementation-evidence repository is `https://github.com/FreeTAKTeam/LXMF-rs.git`. The immutable review range is:

```text
3a2d46bbea174a1049d5d3e06f00c6ea20254085..0ed96f7ee33cefe7fe6eb188b8094b02cd536193
```

The range was inspected through its commit list, file changes, and full behaviorally relevant diffs, then mapped against current `styrene-rns`. The evidence endpoint is licensed EPL-2.0 with GPL-2.0-or-later elected as its secondary license. It is not an MIT-compatible patch source for this change.

RNS protocol authority remains the upstream Python Reticulum release applicable to each behavior. The concurrent `reticulum-1-5-parity-wave` pins the current authority to Reticulum 1.5.1 at immutable revision `149e4151095adf098b8f53eab0c03b37169e8559`. For the internal-interface policy, the behavior was introduced in Reticulum 1.5.0 at immutable revision `e32d4df754a7b87b1bf1bb0d08675d12ff505ae6` and must be confirmed unchanged at the 1.5.1 authority. FreeTAKTeam/LXMF-rs commit `e9111b2621afc31329fa403a61696b7a3d8987f1` records its broader RNS 1.5 evidence, and commit `0ed96f7ee33cefe7fe6eb188b8094b02cd536193` supplies implementation evidence for the two internal announce flags. This wave does not adopt the rest of the RNS 1.5 bundle.

## Clean-room rules

1. Engineers may use this design's behavioral statements, the authoritative protocol, public APIs, and independently written black-box observations.
2. No FreeTAKTeam/LXMF-rs source, fixture, test vector, comment, symbol name, module layout, or line-for-line control flow may enter Styrene.
3. Tests use independently generated inputs and Styrene-owned helpers. Cryptographic vectors come from standards or are generated through public APIs, not copied from the evidence repository.
4. Implementers record the evidence repository URL, exact range above, applicable evidence commit IDs, authority revision, observation date, and an explicit `no source or fixture copied` statement in implementation review notes.
5. Provenance is immutable: later upstream movement does not rewrite this range. A later review uses a new range and change record.
6. If required behavior cannot be derived independently from authority or observable outcomes, classify it for investigation rather than consulting or translating the evidence source during implementation.

## Gap decisions

| Gap | Current Styrene evidence | Decision and immutable implementation evidence |
|---|---|---|
| Cached Fernet verification | `Fernet::verify` uses MAC verification, but `CachedFernet::verify` exits on the first unequal tag byte | Adopt constant-time verification; `844d116a0b676a8f16670b84b021005a3a1aabe1` |
| Key and ratchet persistence | Key files, destination ratchets, and transport ratchets use predictable temporary names and ordinary writes; `StoredKey` debug exposes material | Adopt private directory/file modes where supported, exclusive randomized temporary files, durable atomic replacement, cleanup, and redaction; `a7ca74804b99a1eaaa7bb17c3f7be1d638fac021` |
| Key-manager fallback | `FallbackKeyManager` masks every primary error and may redirect failed writes | Adopt fallback only for not-found reads and explicitly classified availability errors; `696b80f628a6b40cfeb74646d01a533fa3001894` |
| Poisoned receipts | receipt-map operations return `None` or silently drop work forever after poisoning | Adopt recovery because entries have no cross-entry invariant; clear poison after accepting the recovered map; `cdf38df94b6225b217fa6478c4dcb5ef14d00495` |
| Adversarial Link controls | liveness updates precede full validation; corrupt identify, keepalive, close, and channel controls can mutate state or receive proofs | Adopt exact non-RTT framing, validate/decrypt before mutation or proof, retain verified identified identity, and preserve recovery after repeated invalid traffic; `4d7a5eb5375528a3ec6d68c5a5fec6609f00069f`. LinkRTT bytes and validation remain Beechat-owned. |
| Bound-interface Link sends | Link fan-out helpers route ephemeral Link IDs through destination path lookup | Adopt per-Link interface dispatch for all data/channel fan-out helpers while skipping inactive links; `6714767b3aea76e36eda13bde518930e23c7f96c` |
| Resource lifecycle | active arrivals increment retries; requests repeatedly scan from zero; split resources are rejected; packet-build failures and abandoned assemblies can be silent; caps are only partly indirect | Adopt one coherent resource hardening slice: progress-neutral retry counts, round/window requests with bounded re-request, pre-allocation caps, negotiated-MTU sizing, lazy split segments, first-segment metadata, and terminal cleanup/events. Evidence: `d24e8f0bcf75e292669842d2505bfe62eda3e325`, `0b31acdfc1209c16fda143d8f90674f92d47632a`, `9fb079d285430c66ea1df1a3c025063631b96b0e`, `b349ab89a86820d25fad2353346cf32439cf992a`, `c6c86a794c7eb2c2f35fe64f6e320d2160cd44e9`, `e0e7013da9c59d9f0bb37f298864377440df850e`, `138851c749588df771ad1f3abafb402154b055be`, `f67f6267ac9810479153bea21bd9878505a9193c`, `d7c04b0d415ffa7416de3e43505cfda60e4887cc`, `df2f512d0d40b04d01f6cff5ce2fda3484923d78`, `db26e0528861fdc4318f75995d41cb6e65747fd9`, and `fb531fe0fc6fd4a371ddb26a0d7c00d36b5a23a7` |
| Worker supervision | transport drops spawned worker handles; early exit can leave a partial runtime | Adopt named supervision with shared cancellation and quiet normal drain; `54442bb92c972eb49030d606ca6f164021cd2c56` |
| Passive announces | all accepted announces enter a queue drained only when retransmission is enabled | Adopt role/context-gated queueing plus bounded cache retention needed by Styrene path persistence; `9139e8c0c5b65fc3076b4a22503f3e3c1161d392` |
| Internal interface policy | mode exists but per-interface `announces_from_internal` and `announces_to_internal` policy does not | Adopt only the authority-owned RNS 1.5 policy rows and configuration propagation; `0ed96f7ee33cefe7fe6eb188b8094b02cd536193`, introduced at `e32d4df754a7b87b1bf1bb0d08675d12ff505ae6`, current authority `149e4151095adf098b8f53eab0c03b37169e8559` |
| Bearer-neutral bytes | `styrene-rns` has no low-level ordered-byte attempt boundary shared by RNode protocol code | Adopt only a small open/read/write/close attempt trait and one shared RNode/KISS protocol engine; `c244d6f82b95650ed0091a48a9469c7a85d79ffb` and startup clarification `249815e2eed8a3288ca05477b9dcc9f153fb02fd`. Mobile adapters and lifecycle remain externally owned. |

## Ownership, dependencies, and order

Dependencies are stop gates, not advisory cross-references. A gated FreeTAK group may not add its red test or implementation until the prerequisite group's verification item is complete and its resulting contract is present in the reconciled tree. The initial admission record captures the exact prerequisite revisions and completion evidence; if a prerequisite changes the local architecture, this plan is reconciled before work continues.

| FreeTAK work | Owner here | Required predecessor | Explicitly not owned here | Order |
|---|---|---|---|---|
| Immutable provenance and no-copy admission | This wave | None | Updating refs, review markers, or other plans | First |
| Cached Fernet verifier | Cached implementation path only | `reticulum-1-5-parity-wave` group 10, canonical token authentication, verified complete | Canonical token vectors and aggregate token gate | After provenance and Reticulum group 10 |
| Secret persistence, fallback, poisoned receipts | Full behavior | Provenance admission | External key-store platform implementations | After provenance |
| Adversarial Link state mutation | Non-RTT identify, keepalive, close, and channel state/proof behavior | `beechat-rns-corrections-wave` group 2, LinkRTT wire precision, verified complete | RTT bytes, float width, parsing, validation, and Python RTT interoperability | After provenance and Beechat group 2 |
| Bound-interface Link sends | Full helper behavior | Provenance admission | Generic route architecture | After provenance |
| Resource retry/round accounting | Full behavior | Provenance admission | Generic scheduler ownership | After provenance |
| Resource continuation/outstanding window | Full behavior except canonical receive-part search offset | Provenance admission | `reticulum-1-5-parity-wave` group 8 requested-window offset | After retry/round accounting |
| Resource admission caps/effective sizing | Cap enforcement and consumption of effective MTU | `reticulum-1-5-parity-wave` group 7, Link MTU discovery, verified complete | MTU signaling, negotiation, clamping, or proof | After continuation and Reticulum group 7 |
| Split construction/metadata/terminal cleanup | Split-only lifecycle | `reticulum-1-5-parity-wave` group 8, generic Link/resource regressions, verified complete | Generic Link-close cancellation and receive-part offset | After sizing and Reticulum group 8 |
| Worker supervision | Full behavior | Provenance admission | Product process supervision | After provenance |
| Passive announce and internal-interface policy | Queue/cache and two internal-policy flags only | `reticulum-1-5-parity-wave` groups 2-6, final admission/path architecture, all verified complete | Raw admission, priority queues, path batching/limits, and adaptive deadlines | After Reticulum groups 2-6 |
| Ordered-byte attempt and shared RNode engine | Low-level `styrene-rns` trait and protocol engine only | Provenance admission | `shared-dioxus-mobile-ui` group 5 and `deliver-mobile-messaging-minimum` groups 6-7 own adapters, reconnect, permissions, lifecycle, bridge integration, and physical acceptance | After provenance; before mobile adapters consume it |
| Aggregate verification and no-copy audit | This wave | Every FreeTAK behavior group complete | Prerequisite waves' final release claims | Last |

## Design decisions

### Secret persistence

One Styrene-owned helper will provide private directory creation and atomic private-file replacement for every raw key or private ratchet path in this crate. Temporary siblings are randomized and created exclusively so a predictable symlink is never followed. The helper writes all bytes, synchronizes the file, atomically replaces the destination with platform-correct semantics, best-effort synchronizes the parent where supported, and removes abandoned temporary files. Unix directories and files are constrained to `0700` and `0600`. Existing data format remains unchanged.

### Failure classification

Fallback is a policy decision, not a catch-all. A missing primary read and a concrete backend-unavailable error may consult the secondary. Invalid arguments, decode/integrity failures, and all unclassified errors surface unchanged. A failed non-availability write never lands in the secondary. Delete semantics require a separately specified policy and are not broadened by inference.

Receipt poisoning has the opposite policy because the map stores independent correlations. Recovery accepts the inner map, clears poison, warns without packet/message secrets, and allows track, lookup, resolve, and prune to continue.

### Link and interface invariants

A non-RTT Link control packet cannot refresh activity, reactivate stale state, emit semantic identity, or receive a proof until its exact frame, authentication, decryption, bounds, and Link binding pass. Keepalive controls are exactly one byte. Repeated invalid traffic must not prevent a later valid control from succeeding. This state-mutation work starts only after Beechat LinkRTT group 2 is verified and treats that resulting RTT path as an input; it neither defines nor tests RTT wire bytes, float precision, parsing, validation, or Python RTT interoperability.

Established Link packets bypass destination routing and use the interface bound during Link establishment. Fan-out captures `(link, packet)` pairs without holding the global handler during dispatch. This requirement is narrower than generic route correctness in the existing parity wave.

### Resource lifecycle

This wave strengthens, rather than re-owns, the existing requirement that resources are scheduled and terminal. Retry counters advance only for timeout-driven requests, never for active progress. Requests operate in drained rounds with an adaptive bounded window, track outstanding fragments, request hashmap continuation only when the active window needs it, and expire a lost continuation gate. Receiver caps are checked before allocations and use negotiated Link/interface capacity.

Split sends build the first segment before dispatch and later segments on demand outside the global transport lock. Only the first segment strips metadata. Every packet-build failure, size mismatch, remote cancellation, link loss, timeout, or segment-build failure removes all state for the original resource and emits exactly one terminal failure; completion occurs only after verified assembly/proof. This wave does not duplicate the existing live Python/Rust resource gate.

`reticulum-1-5-parity-wave` owns how an effective Link MTU is negotiated, generic cancellation of every resource when a Link closes, and the RNS 1.5 receive-part search offset. This wave consumes the effective MTU, does not add another negotiation policy, and limits terminal cleanup ownership to split-resource state plus its own retry/request failure paths. Its cached Fernet work likewise supplies the uncovered implementation path while the RNS 1.5 wave owns canonical authority vectors and the aggregate constant-time gate.

### Worker and announce bounds

All long-lived transport workers are retained under one named supervisor. Any panic or unexpected return before shutdown records the worker identity, cancels siblings, and drains them. Normal cancellation drains without being reported as failure.

An announce enters the retransmission queue only if this node can drain that queue: transport is enabled or the ingress represents the local/shared-instance exception, and the packet is not a path response. Other accepted announces remain available to Styrene path persistence in the bounded announce cache. Rate-limited packets do not bypass the bound.

### RNS 1.5 internal policy boundary

The two per-interface flags retain authority defaults: absent `announces_from_internal` is permissive and absent `announces_to_internal` grants no override. An outgoing interface that explicitly disables announces from internal blocks non-local announces learned through an internal next hop. An internal outgoing interface blocks a non-local announce learned through a boundary next hop unless that next hop explicitly permits announces to internal. Local announcements remain allowed. Startup, runtime child inheritance, and hot apply carry the same fields.

No other RNS 1.5 behavior is implied by this delta or may be claimed from its completion.

### Bearer-neutral byte ownership

`styrene-rns` owns only a low-level one-attempt trait exposing ordered bytes through `open`, `read`, `write`, and idempotent `close`, plus bearer kind and negotiated MTU metadata, and one shared protocol engine owning RNode detection, radio validation, KISS framing, MTU/write-cap enforcement, flow control, and shutdown frames. The trait has no discovery, permission, reconnect, application-session, or mobile lifecycle API.

Cancellation of a single attempt's open/read/write must leave `close` callable. No Reticulum payload is written before required startup validation. Empty read and end/error remain distinguishable. Fake backends prove protocol-engine neutrality without implementing BLE, Classic, USB, or serial adapters.

`shared-dioxus-mobile-ui` group 5 and `deliver-mobile-messaging-minimum` groups 6-7 remain authoritative for platform implementations, reconnect, permissions, application lifecycle, host integration, physical-device acceptance, and support claims. This wave must not add a mobile bridge, platform service, native handle wrapper, reconnect loop, or duplicate mobile event contract.

## Verification strategy

Each task group is intentionally ordered red test, minimal implementation, focused verification. Focused tests use deterministic clocks, in-memory interfaces, generated keys, and fake byte backends. After all focused checks pass, run `cargo test -p styrene-rns --all-features`, warning-denied Clippy for the crate, formatting, and the repository's offline OpenSpec validation. Existing live interoperability tasks remain in `reticulum-lxmf-nomadnet-parity`.
