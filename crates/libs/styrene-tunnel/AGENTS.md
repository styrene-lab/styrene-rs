# AGENTS.md -- styrene-tunnel

## What It Does

Post-quantum cryptographic (PQC) tunnel establishment for Styrene mesh communications. Provides ML-KEM-768 + X25519 hybrid key exchange, AES-256-GCM session encryption, and a 3-message handshake state machine matching the Python `styrened` implementation. Tunnel backends (strongSwan IPsec, WireGuard) and an orchestrator are feature-gated but currently stubs.

## Current Status

**Core crypto and session state machine work. Backends are stubs.** The PQC key exchange (ML-KEM + X25519 hybrid), AEAD encryption, and KDF are implemented and functional. The strongSwan VICI backend and WireGuard netlink backend are scaffolded with trait impls that return `Unimplemented` errors. The orchestrator exists but cannot do anything useful without a working backend.

## Module Map

```
src/
  lib.rs              # Crate root, re-exports TunnelError + TunnelBackend
  error.rs            # TunnelError enum
  traits.rs           # TunnelBackend trait definition
  crypto/
    mod.rs            # Crypto module root
    kem.rs            # ML-KEM-768 + X25519 hybrid key exchange
    aead.rs           # AES-256-GCM authenticated encryption
    kdf.rs            # HKDF-SHA256 key derivation
  session/
    mod.rs            # 3-message PQC handshake state machine
  strongswan/         # (feature = "strongswan")
    mod.rs            # StrongswanBackend -- TunnelBackend impl (STUB)
    vici.rs           # VICI protocol types
    sa.rs             # Security Association types
  wireguard/          # (feature = "wireguard")
    mod.rs            # WireguardBackend -- TunnelBackend impl (STUB)
  orchestrator/       # (requires any backend feature)
    mod.rs            # Tunnel selection, failover, health monitoring
```

## Key Types and Traits

- `TunnelBackend` (trait) -- async trait for tunnel lifecycle: establish, teardown, rekey, status, list
- `TunnelError` -- crate-level error enum (thiserror)
- Crypto types in `crypto/` -- `MlKemKeyPair`, `SessionCipher`, AEAD encrypt/decrypt functions
- Session state machine in `session/` -- drives the 3-message PQC handshake

## Feature Flags

| Feature | What it enables |
|---------|----------------|
| `default` | `std` |
| `std` | Standard library support |
| `strongswan` | strongSwan VICI backend (stub) + tokio + async-trait + log |
| `wireguard` | WireGuard netlink backend (stub) + tokio + async-trait + log |
| `tunnel` | Both backends + orchestrator |

## Test Commands

```bash
# Core crypto + session tests (default features)
cargo test -p styrene-tunnel

# With strongSwan backend (stubs)
cargo test -p styrene-tunnel --features strongswan

# With WireGuard backend (stubs)
cargo test -p styrene-tunnel --features wireguard

# Everything
cargo test -p styrene-tunnel --all-features
```

## Gotchas

- Depends on both `styrene-rns` and `styrene-mesh` (with `pqc` feature). Changes to either can break this crate.
- The strongSwan and WireGuard backends are **entirely stubs** -- every trait method returns `TunnelError::Unimplemented` or equivalent. Do not assume any backend functionality works.
- Uses `async-trait` (boxed futures) for backends, unlike styrene-content which uses AFIT. This is because backends need `dyn TunnelBackend` dispatch.
- The `ml-kem` crate (v0.2.3) is pinned. Check for updates if you hit issues with PQC key exchange.

## Known Issues

- 10 TODOs across strongswan/ and wireguard/ -- all backend methods are unimplemented.
- strongSwan VICI backend needs: connection initiation, SA termination, rekeying, SA listing.
- WireGuard backend needs: tunnel establishment, peer removal, PSK update, peer status query, peer listing.
- Orchestrator exists but is inert without a working backend.
- No integration tests (would require actual strongSwan/WireGuard infrastructure).
