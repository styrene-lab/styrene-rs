# styrene-rbac

Role-based access control for the Styrene mesh. Hierarchical role model with fine-grained capabilities, roster-based identity binding, and pure policy evaluation. Shared by `styrened` (device RBAC) and `aether` (agent-to-agent RBAC).

## Module map

| File | Purpose |
|------|---------|
| `role.rs` | `Role` enum: Peer, Monitor, Operator, Admin. Cumulative hierarchy. |
| `capability.rs` | `Capability` newtype (dot-separated strings). Per-role capability sets. |
| `policy.rs` | `RbacPolicy` — roster of `RosterEntry` items, `evaluate()` returns allow/deny. |
| `warning.rs` | `PolicyWarning` — non-fatal issues detected during policy evaluation. |

## Key types

- `Role` — four-tier enum, each tier inherits capabilities from below
- `Capability` — `chat.send`, `rpc.exec`, `vpn.handshake`, etc.
- `RbacPolicy` — roster + blocked prefixes, pure evaluation (no I/O)
- `RosterEntry` — identity hash + role + optional orthogonal grants

## Feature flags

| Feature | What it enables |
|---------|-----------------|
| `std` | *(default)* Empty — reserved for future std-dependent features |
| `config` | serde deserialization from YAML/TOML/JSON config files |

## Test commands

```bash
cargo test -p styrene-rbac
cargo test -p styrene-rbac --all-features
```

## Security

See `SECURITY.md` in this directory for the threat model, input validation strategy, and accepted risks (O(n) roster lookups acceptable at mesh scale).

## Status

Stable. 37 tests. No known issues.
