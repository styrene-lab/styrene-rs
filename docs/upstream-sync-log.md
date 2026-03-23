# Upstream Sync Log

Tracks reviews of upstream changes and adoption decisions. See [UPSTREAM.md](../UPSTREAM.md) for strategy.

---

## 2026-03-23 — FreeTAKTeam Upstream Triage (PRs #19–#131)

**Reviewer:** cwilson  
**Range:** `0052218` (PR #18, Repo refactor) → `3a2d46b` (PR #131, current upstream HEAD)  
**Commits reviewed:** 89  
**Branch fix:** upstream remote was tracking `master`; corrected to `main` in `.upstream-tracking.json`

### Summary

Upstream has been in an intensive Python-parity sprint. A 41-issue Rust/Python compatibility
list (`docs/plans/2026-03-18-rust-python-compat-issue-list.md`) was assembled and 15 of 41
issues have been merged, with several more under active PR. The issues directly mirror
our own Tier 1 and Tier 2 gaps in `PARITY_GAPS.md`.

**Also new:** a `rns-embedded-runtime/core/ffi` crate triad (no analog in styrene-rs), and a
complete mixed Rust/Python interop test harness — the infrastructure we need for our interop gate.

Upstream compat issue status as of this review:
- **Fixed (merged):** 1, 2, 5, 6, 7, 8, 9, 11, 12, 13, 14, 15, 16, 17, 19
- **In progress:** 10 (PR #113)
- **Open (25 issues):** 3, 4, 18, 20–43

### FreeTAKTeam / LXMF-rs — Triage Table

#### ADOPT — RNS Transport Parity (maps to `styrene-rns`)

| Commit | PR | Description | Compat Issues Fixed | Action |
|--------|----|-------------|---------------------|--------|
| `af337e9` | #106 | Announce proof gates | 1 (announce hash mismatch), 2 (forged proofs), 11 (pubkey stability), 12 (ratchet parsing), 13 (link-request proof gates) | Port to `styrene-rns/src/transport/announce_table.rs` + `destination.rs` |
| `79250bd` | #107 | Link handshake interface parity | 5 (link proof race), 14 (interface binding), 16 (request/response/identify proofs), 17 (watchdog RTT) | Port to `styrene-rns/src/transport/` |
| `5207ebb` | #109 | Live channel parity | 15 (channel packet semantics) — adds `channel_buffer.rs`, 1987 lines | Port to `styrene-rns/src/transport/links.rs` + new `channel_buffer.rs` |
| `4e8442b` | #123 | Buffer writer parity (live channel) | Follow-up on #109, adds `channel_buffer.rs` 620 lines | Port — stacks on #109 |
| `470593b` | #111 | Buffer callback parity | Follow-up on #109/#123 | Port — stacks on #123 |
| `cbd4702` | #112 | Resource admission, startup, sender timeout | 6, 7, 8, 9 (resource lifecycle), 19 (inbound worker assumes LXMF) | Port to `styrene-rns/src/resource/` |
| `0be07bc` | #124 | Resource receipt status parity | Follow-up: non-terminal until proof arrives | Port — stacks on #112 |
| `6759511` | #115 | Preserve path tags, bound duplicate lifetime | 20 (path response drops tag), 21 (recursive regenerates tag), 22 (duplicate suppression no TTL) | Port to `styrene-rns/src/transport/path_requests.rs` |
| `dcab7a5` | #117 | Interface-scoped announce ingress control | 24 (announce queueing missing), 25 (ingress-limited release) | Port to `styrene-rns/src/transport/announce_table.rs` |
| `4ce06f8` | #121 | Recursive path request throttling interface-aware | 23 (global instead of interface-centric) | Port to `styrene-rns/src/transport/path_requests.rs` |
| `895777a` | #122 | Announce retry timing and completion with Python | 27 (retransmit timing), 28 (rate limiting) | Port to `styrene-rns/src/transport/announce_limits.rs` |
| `786886e` | #125 | Fix announce retry parity (review follow-up) | Follow-up on #122 | Port — stacks on #122 |

#### ADOPT — LXMF Layer Parity (maps to `styrene-lxmf`)

| Commit | PR | Description | Compat Issues Fixed | Action |
|--------|----|-------------|---------------------|--------|
| `27f3922` | #118 | Align propagated link lifecycle with Python LXMF router | Propagation link state machine | Port to `styrene-lxmf/src/` |
| `3abc9e4` | #119 | Retain announced LXMF stamp cost | 34 (stamp cost discarded) | Port to `styrene-lxmf/src/` |
| `4e858b5` | #120 | Align peer lifecycle with Python LXMF router | Peer state machine correctness | Port to `styrene-lxmf/src/` |
| `1a69cfe` | — | LXMF announce display name helpers | Announce display name | Port to `styrene-lxmf/src/` |
| `44918d4` | — | std-gated messaging store exports to lxmf-sdk | SDK/std gating | Port to `styrene-lxmf/src/sdk/` |
| `493fa42` | #129 | Validate propagation stamps on ingest | 33 (propagation stamp validation missing) | Port to `styrene-lxmf/src/` |
| `40d63f9` | #130 | Canonicalize propagation transient IDs | 36 (transient-id lifecycle) | Port to `styrene-lxmf/src/message/` |
| `3a2d46b` | #131 | Model propagated payload identity in wire helper | 36 follow-up, propagation wire shape | Port to `styrene-lxmf/src/message/wire.rs` |

#### ADOPT — Daemon (maps to `styrened-rs`)

| Commit | PR | Description | Compat Issues Fixed | Action |
|--------|----|-------------|---------------------|--------|
| `92e964b` | #114 | Honor LXMF delivery modes in reticulumd bridge | 3 (delivery method ignored — partial), 14-adjacent | Port to `styrened-rs/src/` bridge |
| `4ca2386` | #126 | Wire LXMF stamp and ticket baseline into daemon send path | 30 (stamp/ticket options ignored), 31 (inbound stamp enforcement) | Port to `styrened-rs/src/` |
| `9e88e96` | #128 | Announce ratchet parsing strict by context flag | Propagation ratchet strictness | Port to `styrened-rs/src/` or `styrene-rns/` |
| `da08be1` | #85 | Optimize reticulumd RPC and storage hot paths | — performance | Port to `styrened-rs/src/` |
| `9eb4f33` | #83 | Hot-apply legacy TCP interfaces | — usability | Port to `styrened-rs/src/` |
| `1d0b9f1` | #91 | Single-file TOML config flow for lxmd | — config usability | Port to `styrened-rs/src/` (addresses PARITY_GAPS.md §1.4 partially) |
| `4b4f086` | #90 | Fix live node transport and remote control flows | — correctness | Port to `styrened-rs/src/` |

#### ADOPT — Interop Test Infrastructure (maps to `tests/`)

| Commit | PR | Description | Action |
|--------|----|-------------|--------|
| `23a4481` | #116 | Python compatibility harness scaffold | Port matrix structure + test fixture pattern to `tests/interop-test/` |
| `e9694bc` | #127 | Live mixed Rust/Python harness slice | Port `python_compat_matrix.rs` + `python-lxmd-rust-lxmd-smoke.sh` to `tests/` |
| `6da2327` | #89 | Python daemon parity and interop smoke coverage | Port test patterns to interop suite |

#### ADOPT — Reference Docs (read-only, no code to port)

| Commit | PR | Description | Action |
|--------|----|-------------|--------|
| `eb970ba` | #104 | Rust/Python compatibility issue list | Port `docs/plans/2026-03-18-rust-python-compat-issue-list.md` to `docs/COMPAT_ISSUES.md` — this is our Tier 1/2 bug list |
| `a56ba74` | #100 | Assess LXMF vs Reticulum parity | Port assessment doc to `docs/` |
| `b32953b` | #86 | OpenRPC canonical for SDK RPC contracts | Reference for RPC contract design |

#### DEFER — Embedded Runtime (Phase 5, edge targets)

| Commit | PR | Description | Notes |
|--------|----|-------------|-------|
| `6eb6bed` | #19 | Cross-platform serial/BLE/LoRa interfaces | Relevant for PARITY_GAPS §1.2 but scope is large; assess when Serial/KISS gap is prioritized |
| `57c1e28` | #21 | Embedded runtime proof + standalone node | Relevant for styrene-edge ARM targets |
| `6ec28ff` | #22 | Standalone embedded node phase 0 | TCP runtime + BLE provisioning |
| `672283b` | #24 | Implement public embedded node API | New `rns-embedded-core` crate triad — no analog in styrene-rs yet |
| `fb6ad1a` | #26 | Tighten embedded node FFI contract | Stacks on #24 |
| `1518f71` | #44 | Docs: choose reticulum-compatible embedded path | Decision doc, read before embedded work |
| `f62cdfa` | #87 | Python parity benchmark suite | Good after correctness work |
| `69a1b80` | #88 | Report-grade benchmarks | Good after correctness work |

#### SKIP — Flutter/TAK Wrappers (not relevant to styrene-rs)

PRs #37, 38, 45, 47, 50, 56, 57, 58, 59, 60, 61, 62, 64, 65, 66, 67, 68, 69, 70, 71, 77, 78, 79, 80, 81, 82, 84 — Flutter SDK wrappers, TAK-specific workflows, r3akt mission, domain command watchers. Not carried.

#### SKIP — gRPC (intentionally dropped)

| Commit | PR | Description | Notes |
|--------|----|-------------|-------|
| `9d853bb` | #93 | gRPC API surface, Bruno collections, peer diagnostics | gRPC not in styrene-rs by design (UPSTREAM.md §What Changed) |

#### SKIP — Build/CI (their platform, not ours)

PRs #92, 94, 95, 96, 97, 105 — Windows daemon bundles, cross-platform SQLite, Windows CI shell, PR CI simplification. Not applicable to our Argo Workflows CI.

#### SKIP — Docs/Misc

PRs #101 (DeepWiki badge), #108 (contributor guide), plain docs commits.

### Unmerged Upstream Feature Branches — Assessment

| Branch | Commits | Assessment |
|--------|---------|------------|
| `codex/propagated-python-rust-harness-case` | +7 | **review** — stacks on interop harness work, has Python propagation test cases we want |
| `codex/python-path-response-interop` | +12 | **review** — path response parity; addresses compat issue 20/21 territory |
| `codex/inbound-propagation-envelope-bridge` | +8 | **review** — propagation envelope handling; relevant to issues 36/37 |
| `codex/reticulum-compatibility-matrix` | +2 | **review** — compatibility matrix expansion |
| `codex/cross-platform-interfaces-rollout` | +7 | **defer** — serial/BLE/LoRa; see DEFER table above |
| `corvo/fix-high-priority-bug-in-python-harness-tests` | +2 | **review** — bug fix in harness, check before adopting harness |
| `codex/ci-fix-pr18` | +193 | skip — their CI |
| `codex/client-compatibility-fixes` | +3 | **review** — propagation parity fixes |
| `codex/enterprise-platform-split-phase1` | +33 | skip — TAK enterprise split |
| `codex/review-comment-fixes` | +10 | skip — review cleanup |
| `rust-lxmf` | +24 | **review** — unclear scope, inspect before deciding |

### Adoption Priority Queue

| Priority | Commit Group | Maps To | Compat Issues Closed |
|----------|-------------|---------|----------------------|
| 1 | #106 announce proof gates | `styrene-rns` announce/destination | 1, 2, 11, 12, 13 |
| 2 | #107 link handshake parity | `styrene-rns` transport/links | 5, 14, 16, 17 |
| 3 | #109 + #123 + #111 channel parity stack | `styrene-rns` channel_buffer | 15 |
| 4 | #112 + #124 resource lifecycle stack | `styrene-rns` resource/ | 6, 7, 8, 9, 10, 19 |
| 5 | #115 path tag preservation | `styrene-rns` path_requests | 20, 21, 22 |
| 6 | #117 + #121 + #122 + #125 announce parity stack | `styrene-rns` announce_limits | 23, 24, 25, 27, 28 |
| 7 | #116 + #127 interop harness | `tests/interop-test/` | — infra |
| 8 | #118–#120 LXMF parity stack + #128–#131 propagation | `styrene-lxmf` | 33, 34, 36 |
| 9 | #114 + #126 daemon delivery modes + stamps | `styrened-rs` | 3, 30, 31 |
| 10 | #83 + #85 + #90 + #91 daemon fixes | `styrened-rs` | — perf/config |
| 11 | Compat issue list doc | `docs/COMPAT_ISSUES.md` | — reference |

---

## 2026-02-24 — Initial Assessment

**Reviewer:** cwilson

### Beechat / Reticulum-rs

**Range:** fork baseline → `beechat/main` HEAD

New commits since FreeTAKTeam forked (approx. Jan 23 2026):

| Commit | Description | Author | Decision | Notes |
|--------|-------------|--------|----------|-------|
| `797df2a` | feat: send proofs for received messages in links | jomuel | **adopt** | Link message proofs — needed for correct link behavior |
| `b965725` | test: send proofs for received messages in links | jomuel | **adopt** | Test coverage for above |
| `6299302` | fix: consider new LinkEvent variant in examples | jomuel | skip | Examples not carried |
| `08a7157` | docs: add guidelines for contributors | jomuel | skip | Doc-only |
| `7374a74` | Merge: link-rtt | spearman | **adopt** | Link RTT measurement |
| `aa39a9a` | Merge: contributing-guide | jomuel | skip | Doc-only |
| `6802f99` | Merge: link-message-proofs | spearman | **adopt** | (merge commit for 797df2a) |
| `8b4b95e` | refactor: fix compiler warnings in integration tests | jomuel | skip | Test infra only |
| `91c4380` | feat: add get in/out destinations to transport | spearman | **defer** | Useful but not blocking |
| `5407b91` | Merge: get-destinations | max-ost | **defer** | (merge commit for 91c4380) |
| `6203b6b` | Merge: link-lifecycle | max-ost | **review** | Link lifecycle (stale→close). Need to assess scope |

**Unmerged feature branches of interest:**

| Branch | Commits | Decision | Notes |
|--------|---------|----------|-------|
| `flexible-link-behavior` | +4 | **defer** | Routing strategy options, announce retransmit config. Nice-to-have |
| `channel` | +5 | **defer** | Channel abstraction for typed messages. Already partially absorbed |
| `transport-tests` | +1 | **adopt** | Regression test for stalling transport |

### FreeTAKTeam / LXMF-rs

**Range:** `0052218` (fork point) → `upstream/master` HEAD

No new commits on `master` since fork point.

**Unmerged feature branches of interest:**

| Branch | Commits | Decision | Notes |
|--------|---------|----------|-------|
| `codex/cross-platform-interfaces-rollout` | +7 | **review** | Serial/HDLC, BLE/GATT, LoRa interfaces. Valuable for edge targets |
| `codex/enterprise-platform-split-phase1` | +5 | skip | SDK split, not relevant to our structure |
| `codex/client-compatibility-fixes` | +3 | **review** | Propagation parity, test fixes |
| `codex/ci-fix-pr18` | +many | skip | CI fixes for their pipeline |
| `codex/lxmf-mainline-20260211` | +0 | skip | Empty/merged |

### Security Fixes (Already Applied)

Both of these were independently applied in styrene-rs before the upstream adopted them:

- Constant-time HMAC verification (styrene-rs `f8fc996`, Beechat PR #41)
- Identity.encrypt() double-ephemeral fix (styrene-rs `77ce75d`, Beechat PR #42)

### Priority Adoption Queue

1. **Link message proofs** (beechat `797df2a`, `b965725`) — required for correct link behavior
2. **Link RTT** (beechat `7374a74`) — needed for link quality assessment
3. **Transport stall regression test** (beechat `transport-tests`) — test coverage
4. **Link lifecycle** (beechat `6203b6b`) — stale→close behavior, assess scope first
5. **Client compatibility fixes** (upstream `codex/client-compatibility-fixes`) — propagation parity
6. **Cross-platform interfaces** (upstream `codex/cross-platform-interfaces-rollout`) — serial/BLE/LoRa for edge
