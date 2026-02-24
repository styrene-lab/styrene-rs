# Upstream Fork Attribution

This repository is a fork of [FreeTAKTeam/LXMF-rs](https://github.com/FreeTAKTeam/LXMF-rs), the most complete Rust implementation of both [RNS](https://reticulum.network/) and [LXMF](https://github.com/markqvist/LXMF) protocols.

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
- Added `styrene-mesh` crate implementing Styrene wire protocol
- CI replaced (55-job → 4-job pipeline)
- Security fixes applied (constant-time HMAC, double-ephemeral fix)

## Upstream Tracking

The `upstream` remote tracks the original repository (push disabled):

```bash
git remote -v
# upstream  https://github.com/FreeTAKTeam/LXMF-rs.git (fetch)
# upstream  DISABLE (push)

# Fetch upstream changes
git fetch upstream

# Cherry-pick specific fixes
git cherry-pick <commit>
```

## License

MIT — preserved from upstream.
