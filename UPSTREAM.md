# Styrene Lineage and Compatibility References

Styrene is an independent Rust mesh communications project built on RNS and
LXMF. It owns its product architecture, runtime, storage, IPC, identity model,
interfaces, release policy, and user experience.

The repository contains MIT-licensed code descended from
[FreeTAKTeam/LXMF-rs](https://github.com/FreeTAKTeam/LXMF-rs), which incorporated
work from [BeechatNetworkSystemsLtd/Reticulum-rs](https://github.com/BeechatNetworkSystemsLtd/Reticulum-rs). That ancestry remains part of the project history and its attribution is preserved. These repositories are lineage and compatibility references, not Styrene's architectural upstreams.

## Lineage

```text
Reticulum / LXMF specifications and Python implementations (protocol authority)
        │
        ▼
BeechatNetworkSystemsLtd/Reticulum-rs   (historical Rust RNS lineage)
        │
        ▼
FreeTAKTeam/LXMF-rs                     (historical Rust LXMF lineage)
        │
        ▼
styrene-lab/styrene-rs                  (independent project)
```

The arrows describe code ancestry, not an ongoing fork or merge hierarchy.
Styrene became an independent distribution on 2026-04-19. Protocol and
security review against reference implementations continues; product feature
development happens here.

## Historical Import Date

The initial code import occurred on 2026-02-24.

## Reference State at Import

- Working: TCP/UDP transport, identity management (X25519 + Ed25519), destinations, links, resources, ratchets
- Working (legacy location): LXMF router, propagation, stamps, delivery pipeline
- Known issues: IFAC bug (multi-hop broken), HMAC timing oracle, `Identity.encrypt()` double-ephemeral

## What Changed

The imported code was restructured for the [Styrene](https://github.com/styrene-lab) mesh communications project:

- Crates renamed from `rns-*`/`lxmf-*` to `styrene-*` namespace
- Legacy crates (`crates/internal/`) merged into main library crates
- Transport layer feature-gated behind `features = ["transport"]`
- LXMF SDK types feature-gated behind `features = ["sdk"]`
- Added `styrene-mesh` crate implementing Styrene wire protocol
- Added `styrene-ipc` crate for daemon interface boundary traits
- CI replaced (55-job → 4-job pipeline)
- Security fixes applied (constant-time HMAC, double-ephemeral fix)

---

## Reference Review Strategy

### Remotes

The canonical `origin` is Styrene. The two lineage remotes are fetch-only;
their names identify the projects rather than implying governance:

```text
origin       https://github.com/styrene-lab/styrene-rs.git                 (fetch + push)
freetakteam  https://github.com/FreeTAKTeam/LXMF-rs.git                    (fetch only)
beechat      https://github.com/BeechatNetworkSystemsLtd/Reticulum-rs.git  (fetch only)
```

### Reference Roles

| Reference | Role | Relevant to |
|-----------|------|-------------|
| Reticulum specification + Python RNS | RNS protocol authority | `styrene-rns` |
| Python LXMF | LXMF protocol authority | `styrene-lxmf`, `styrened` |
| **beechat** (`beechat/main`) | Historical Rust lineage and implementation evidence | `styrene-rns` |
| **freetakteam** (`freetakteam/main`) | Rust behavioral reference and selectively ported fixes | `styrene-rns`, `styrene-lxmf`, `styrened` |

No reference implementation controls Styrene's product architecture. Where
implementations disagree, protocol specifications, canonical behavior,
interoperability evidence, and security analysis decide the local behavior.

### Why Not Merge/Rebase

Styrene's structural divergence makes direct merge/rebase unsuitable:

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
  "freetakteam": { "last_reviewed": "<sha>" }
}
```

### Automated Weekly Review (CI)

A GitHub Actions workflow (`.github/workflows/upstream-sync.yml`) runs every Monday at 06:00 UTC:

1. Fetches both references and checks for new commits since the last review
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

**Merging the PR advances the tracking markers** — the branch includes an updated `.upstream-tracking.json` pointing to the current reference heads.

### Manual Review (Local)

```bash
# Review new reference changes
just upstream-review           # or: ./scripts/upstream-review.sh

# Review a specific reference only
just upstream-review beechat
just upstream-review freetakteam

# Generate the same report CI would create
just upstream-sync-report

# Show current tracking state
just upstream-status
```

### Triage Process

For each batch of reference changes (whether from the weekly PR or local review):

1. **Review** — read the commit list and diff summary
2. **Triage** — classify each change:
   - **adopt** — apply the equivalent change to styrene-rs
   - **skip** — not relevant (CI, kaonic, naming, docs-only, etc.)
   - **defer** — relevant but not needed yet
3. **Apply** — for adopted changes, create a commit with the equivalent fix/feature, citing the reference commit:
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
| `libs/rns-rpc/` | `apps/styrened/src/rpc/` | Absorbed into daemon |
| `apps/reticulumd/` | `apps/styrened/` | Renamed |
| `apps/lxmf-cli/` | *(removed)* | Not carried |
| `apps/rns-tools/` | *(removed)* | Not carried |

### Review Log

Each sync review (automated or manual) is recorded in `docs/upstream-sync-log.md` with:
- Date and reviewer
- Commit range reviewed
- Decisions (adopt/skip/defer per commit)
- styrene-rs commits that ported reference changes

---

## License

MIT — retained from the imported lineage. See repository history and notices for attribution.
