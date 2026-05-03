`F58a`cStyrene Architecture`f

`[< Back to Index`:/page/index.mu]

-

>Protocol Stack

>>Reticulum Network Stack (RNS)
The transport layer. Provides cryptographic identity
(X25519 + Ed25519), destination addressing, link
establishment, and multi-hop mesh routing.

Styrene uses a Rust implementation (styrene-rns)
that is wire-compatible with the Python reference.

>>LXMF (Long-range eXtensible Message Format)
The messaging layer. Encrypted, store-and-forward.
Messages propagate through the mesh via propagation
nodes (hubs) for offline delivery.

Compatible with Sideband and NomadNet.

>>Styrene Wire Protocol
Application-layer extensions on top of LXMF.
Encoded as CBOR (RFC 8949) for deterministic
serialization. Handles fleet operations, page
requests, tunnel negotiation.

-

>Crate Map

    styrene-rns     RNS protocol core
    styrene-lxmf    LXMF messaging
    styrene-mesh    Styrene wire protocol
    styrene-ipc     IPC type definitions
    styrened        Daemon + RPC server
    styrene-dx      Desktop app (Dioxus)
    styrene-tui     Terminal UI (Ratatui)

-

>Node Roles

`F5afFull Node`f -- Routes packets, maintains announce
tables, accepts connections. Default role.

`F5afHub`f -- Full node + propagation store. Relays
messages for offline peers. Runs at rns.styrene.io.

`F5afPropagation Client`f -- Lightweight, no routing.
Connects to a hub for message delivery. Used on
mobile devices.

-

>Identity Model

Keys are generated locally. No central authority.
Each node has:
  - X25519 encryption key pair
  - Ed25519 signing key pair
  - Address hash (16 bytes, derived from public keys)
  - Delivery destination (LXMF endpoint)

Optional persistence via:
  - File-based identity (~/.config/styrene/identity)
  - YubiKey PIV slot (hardware-bound)
  - iOS Keychain / Android Keystore

-

`[< Back to Index`:/page/index.mu]
