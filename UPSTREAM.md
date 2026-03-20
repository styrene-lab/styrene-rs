# Upstream Fork Attribution

This repository is a fork of [FreeTAKTeam/LXMF-rs](https://github.com/FreeTAKTeam/LXMF-rs), which itself incorporated [BeechatNetworkSystemsLtd/Reticulum-rs](https://github.com/BeechatNetworkSystemsLtd/Reticulum-rs) — the original and most mature community Rust implementation of [RNS](https://reticulum.network/).

## Lineage

```
BeechatNetworkSystemsLtd/Reticulum-rs  (RNS core, ~185 stars, multi-contributor)
        │
        ▼  (incorporated Jan 2026)
FreeTAKTeam/LXMF-rs                    (added LXMF layer, workspace split, daemon)
        │
        ▼  (forked Feb 24 2026)
styrene-lab/styrene-rs                  (renamed, restructured, security-hardened)
```

## Fork Date

2026-02-24

## Upstream State at Fork

- Working: TCP/UDP transport, identity management (X25519 + Ed25519), destinations, links, resources, ratchets
- Working (legacy location): LXMF router, propagation, stamps, delivery pipeline
- Known issues: IFAC bug (multi-hop broken), HMAC timing oracle, `Identity.encrypt()` double-ephemeral

## What Changed

The fork was restructured for the [Styrene](https://github.com/styrene-lab) mesh communications project:

- Crates renamed from `rns-*`/`lxmf-*` to `styrene-*` namespace
- Legacy crates (`crates/internal/`) merged into main library crates
- Transport layer feature-gated behind `features = ["transport"]`
- LXMF SDK types feature-gated behind `features = ["sdk"]`
- Added `styrene-mesh` crate implementing Styrene wire protocol
- Added `styrene-ipc` crate for daemon interface boundary traits
- CI replaced (55-job → 4-job pipeline)
- Security fixes applied (constant-time HMAC, double-ephemeral fix)

---

## Upstream Tracking Strategy

### Remotes

Three remotes, two fetch-only upstreams with push disabled:

```
origin    https://github.com/styrene-lab/styrene-rs.git       (fetch + push)
upstream  https://github.com/FreeTAKTeam/LXMF-rs.git          (fetch only)
beechat   https://github.com/BeechatNetworkSystemsLtd/Reticulum-rs.git  (fetch only)
```

### What Each Upstream Provides

| Upstream | Tracks | Relevant to |
|----------|--------|-------------|
| **beechat** (`beechat/main`) | Core RNS protocol: identity, destinations, links, transport, interfaces, crypto | `styrene-rns` crate |
| **upstream** (`upstream/master`) | LXMF layer + daemon/RPC + workspace-level changes | `styrene-lxmf`, `styrened-rs` crates |

**Beechat is authoritative for RNS protocol correctness** — larger contributor base, longer history, broader review. FreeTAKTeam is relevant for LXMF-specific features and daemon patterns.

### Why Not Merge/Rebase

The fork performed heavy structural changes that make `git merge` / `git rebase` produce 100% conflicts:

1. **Namespace rename**: `rns-*` → `styrene-rns`, `lxmf-*` → `styrene-lxmf`
2. **Directory restructure**: flat `src/` → feature-gated `src/transport/`
3. **Crate merges**: `rns-transport` absorbed into `styrene-rns`, `lxmf-sdk` absorbed into `styrene-lxmf`
4. **Kaonic removal**: gRPC interface not carried over

The sync strategy is **review-and-apply**, not merge-and-resolve.

### Tracking State

Last-reviewed commit SHAs are stored in `.upstream-tracking.json` (committed to repo). This file is the source of truth for both local tooling and CI workflows.

```json
{
  "beechat": { "last_reviewed": "<sha>" },
  "upstream": { "last_reviewed": "<sha>" }
}
```

### Automated Weekly Review (CI)

A GitHub Actions workflow (`.github/workflows/upstream-sync.yml`) runs every Monday at 06:00 UTC:

1. Fetches both upstreams and checks for new commits since last review
2. If no drift — exits silently, no PR created
3. If drift exists:
   - Generates a structured report with per-commit triage table
   - Creates branch `upstream-review/YYYY-MM-DD` with updated tracking file + sync-log skeleton
   - Opens a PR labeled `upstream-review`, assigned to `styrene-lab/styrene-admin`
   - Closes any superseded older review PRs

The PR contains:
- Commit tables with empty Decision/Notes columns for the reviewer to fill in
- File-level diff stats (collapsed)
- Unmerged feature branch summary
- Reviewer checklist

**Merging the PR advances the tracking markers** — the branch includes an updated `.upstream-tracking.json` pointing to the current upstream HEADs.

### Manual Review (Local)

```bash
# Review new upstream changes
just upstream-review           # or: ./scripts/upstream-review.sh

# Review a specific upstream only
just upstream-review beechat
just upstream-review upstream

# Generate the same report CI would create
just upstream-sync-report

# Show current tracking state
just upstream-status
```

### Triage Process

For each batch of upstream changes (whether from the weekly PR or local review):

1. **Review** — read the commit list and diff summary
2. **Triage** — classify each change:
   - **adopt** — apply the equivalent change to styrene-rs
   - **skip** — not relevant (CI, kaonic, naming, docs-only, etc.)
   - **defer** — relevant but not needed yet
3. **Apply** — for adopted changes, create a commit with the equivalent fix/feature, citing the upstream commit:
   ```
   fix(rns): correct path_request decoding hash step

   Port of beechat/Reticulum-rs@f0636bd
   ```
4. **Advance markers** — update tracking to record what's been reviewed:
   - **Via PR:** merge the weekly review PR (tracking file is already updated)
   - **Locally:** `just upstream-advance` (updates `.upstream-tracking.json`, commit the change)

### Path Mapping (Beechat → styrene-rns)

For manually applying Beechat changes to `styrene-rns`:

| Beechat `src/` | styrene-rns `src/` | Notes |
|---|---|---|
| `identity.rs` | `identity.rs` | Direct |
| `destination.rs` | `destination.rs` | Direct (styrene-rns also has `destination/` subdir) |
| `destination/link.rs` | `transport/destination_ext/link/` | **Relocated** behind transport feature |
| `destination/link_map.rs` | `transport/destination_ext/link_map.rs` | **Relocated** behind transport feature |
| `packet.rs` | `packet.rs` | Direct |
| `hash.rs` | `hash.rs` | Direct |
| `buffer.rs` | `buffer.rs` | Direct |
| `crypt.rs` | `crypt.rs` | Direct |
| `crypt/fernet.rs` | `crypt/fernet.rs` | Direct |
| `serde.rs` | `serde.rs` | Direct |
| `error.rs` | `error.rs` + `transport/error.rs` | Split across feature boundary |
| `transport.rs` | `transport/core_transport/` (14 files) | **Decomposed** from monolith |
| `transport/announce_table.rs` | `transport/core_transport/announce_table.rs` | Nested deeper |
| `transport/announce_limits.rs` | `transport/core_transport/announce_limits.rs` | Nested deeper |
| `transport/link_table.rs` | `transport/core_transport/link_table.rs` | Nested deeper |
| `transport/packet_cache.rs` | `transport/core_transport/packet_cache.rs` | Nested deeper |
| `transport/path_table.rs` | `transport/core_transport/path_table.rs` | Nested deeper |
| `transport/path_requests.rs` | `transport/core_transport/path_requests.rs` | Nested deeper |
| `iface.rs` | `transport/iface/mod.rs` | **Relocated** behind transport feature |
| `iface/hdlc.rs` | `transport/iface/hdlc.rs` | Under transport |
| `iface/tcp_client.rs` | `transport/iface/tcp_client.rs` | Under transport |
| `iface/tcp_server.rs` | `transport/iface/tcp_server.rs` | Under transport |
| `iface/udp.rs` | `transport/iface/udp.rs` | Under transport |
| `iface/kaonic.rs` | *(removed)* | gRPC interface not carried |
| `utils.rs` | `transport/utils/mod.rs` | Under transport |
| `utils/cache_set.rs` | `transport/utils/cache_set.rs` | Under transport |
| `channel.rs` | `transport/channel.rs` | Under transport |

### Path Mapping (FreeTAKTeam → styrene-rs)

| FreeTAKTeam `crates/` | styrene-rs `crates/` | Notes |
|---|---|---|
| `libs/rns-core/` | `libs/styrene-rns/` (core modules) | Renamed |
| `libs/rns-transport/` | `libs/styrene-rns/src/transport/` | Merged into styrene-rns behind feature gate |
| `libs/lxmf-core/` | `libs/styrene-lxmf/` | Renamed |
| `libs/lxmf-sdk/` | `libs/styrene-lxmf/src/sdk/` | Merged into styrene-lxmf behind feature gate |
| `libs/rns-rpc/` | `apps/styrened-rs/src/rpc/` | Absorbed into daemon |
| `apps/reticulumd/` | `apps/styrened-rs/` | Renamed |
| `apps/lxmf-cli/` | *(removed)* | Not carried |
| `apps/rns-tools/` | *(removed)* | Not carried |

### Review Log

Each sync review (automated or manual) is recorded in `docs/upstream-sync-log.md` with:
- Date and reviewer
- Commit range reviewed
- Decisions (adopt/skip/defer per commit)
- styrene-rs commits that ported upstream changes

---

## License

MIT — preserved from upstream.
