# Upstream Sync Log

Tracks reviews of upstream changes and adoption decisions. See [UPSTREAM.md](../UPSTREAM.md) for strategy.

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
