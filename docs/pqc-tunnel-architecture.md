# Post-Quantum Tunnel Architecture

Research analysis evaluating strongSwan (IPsec/IKEv2) vs WireGuard for establishing PQC-secured tunnels between RNS-authenticated nodes.

**Date:** 2026-02-25
**Status:** Research / Design Phase

---

## Threat Model

The primary threat is **harvest-now-decrypt-later (HNDL)**: a state-level adversary records encrypted traffic today and decrypts it with a cryptographically relevant quantum computer in the future. For sovereign communications, this means every session that lacks post-quantum key exchange has a shelf life on its confidentiality.

## The Discovery-to-Tunnel Pipeline

The use case is a three-phase flow:

1. **Sneaky discovery** — RNS link establishment over any bearer (LoRa, TCP, UDP, I2P) proves peer liveness and identity
2. **PQC tunnel establishment** — a post-quantum-secured tunnel is spun up over traditional internet between the discovered peers
3. **High-bandwidth data plane** — maximum throughput over the secured tunnel for LXMF, IP traffic, or any payload

This aligns with the BATMAN-adv / 802.11s L2 mesh architecture where the tunnel could carry mesh traffic over internet backhaul.

## RNS Security Model (What It Already Provides)

RNS links provide strong *classical* cryptographic security:

| Property | Implementation |
|----------|---------------|
| Mutual authentication | Ed25519 signatures over 3-packet handshake |
| Key agreement | Ephemeral X25519 ECDH per link, HKDF-SHA256 derivation |
| Forward secrecy | Fresh keypairs generated per link (not reused from identity) |
| Encryption | AES-256-CBC per packet with random IV (modified Fernet) |
| Integrity | HMAC-SHA256, constant-time comparison |

**Critical limitation:** X25519 and Ed25519 are **not post-quantum secure**. An adversary who harvests the RNS link handshake can recover the shared secret with a quantum computer, decrypting all link traffic including any metadata exchanged to bootstrap the subsequent tunnel.

### Design Rule

Treat the RNS link as a **signaling channel with a shelf life**, not a secrets channel. Exchange only:

- Peer IP endpoints (not secret — the tunnel exposes them anyway)
- Capability flags (PQC support, tunnel preferences)
- Nonces for PSK derivation (binding, not secrecy)

Never derive the tunnel's long-term key material solely from the RNS link secret.

## Option A: strongSwan IKEv2 with ML-KEM (Recommended for PQC Tier)

### How It Works

[RFC 9370](https://www.rfc-editor.org/rfc/rfc9370.html) defines multiple key exchanges in IKEv2. [strongSwan 6.0+](https://strongswan.org/blog/2024/12/03/strongswan-6.0.0-released.html) implements this natively with hybrid proposals:

```
x25519-ke1_mlkem768-aes256gcm16-sha384
```

X25519 ECDH runs first (small payloads, fits in IKE_SA_INIT), then ML-KEM-768 in an encrypted IKE_INTERMEDIATE exchange (where fragmentation works). The final SA key derives from **both** algorithms — if either holds, the key is secure.

### Properties

| Property | Detail |
|----------|--------|
| PQC algorithm | ML-KEM-768 or ML-KEM-1024 (FIPS 203, NIST standardized Aug 2024) |
| Hybrid mode | Classical ECDH + ML-KEM — belt and suspenders |
| Standards track | RFC 9370 + [draft-ietf-ipsecme-ikev2-mlkem](https://www.ietf.org/archive/id/draft-ietf-ipsecme-ikev2-mlkem-03.html) |
| Key size overhead | ML-KEM-768: 1184B pubkey, 1088B ciphertext (handshake only) |
| Handshake cost | 4+ packets, at tunnel setup only |
| Data plane | ESP with AES-256-GCM — kernel-level, wire-speed |
| Maturity | Enterprise-grade, Palo Alto/Cisco interop tested |

### Strengths

- **FIPS 203 ML-KEM** is NIST-standardized — sovereign comms credibility requires standards
- **RFC 9370 interop** — not locked into a single implementation
- **Overhead is acceptable** on internet bearers where this tunnel runs — 62-96 bytes ESP overhead is noise on Ethernet/WiFi/fiber
- **Credential bridge is solvable** — RNS identity-derived PSK for classical binding, ML-KEM for PQC

### Weaknesses

- IKEv2 complexity and configuration surface area
- X.509/PSK credential model doesn't naturally map to RNS identities (requires bridge)
- Heavier resource footprint on constrained nodes
- Per-packet ESP overhead (62-96 bytes) would be prohibitive on LoRa — but that's not the target bearer

### RNS-to-strongSwan Credential Bridge

The integration pattern:

1. RNS link established — proves peer identity (Ed25519)
2. Daemon extracts shared secret from RNS link
3. Derives a PSK from RNS shared secret (classical binding to RNS identity)
4. Injects PSK into strongSwan's credential store
5. IKEv2 uses PSK + ML-KEM hybrid → PQC security is independent of RNS

The PSK provides *binding* to the RNS identity exchange. The ML-KEM provides PQC. Neither alone is sufficient; together they ensure the tunnel is authenticated (via RNS identity chain) and quantum-resistant (via ML-KEM).

## Option B: WireGuard + Rosenpass (Alternative PQC Path)

### How It Works

[Rosenpass](https://rosenpass.eu/) runs alongside WireGuard as a separate Rust daemon. Every ~2 minutes it performs a PQC key exchange (Classic McEliece + Kyber) and [injects the resulting symmetric key into WireGuard's PSK field](https://github.com/rosenpass/rosenpass). WireGuard's Noise IK handshake mixes this PSK into its key derivation. WireGuard itself is untouched.

### Properties

| Property | Detail |
|----------|--------|
| PQC algorithms | Classic McEliece 460896 + Kyber 512 (two independent PQC schemes) |
| Hybrid mode | WireGuard X25519 Noise IK + Rosenpass PQC PSK |
| Standards track | No (NLnet-funded research, formally verified in ProVerif) |
| Key size overhead | Classic McEliece: **261KB public key** (one-time at handshake) |
| Handshake cost | 1 RTT (WireGuard) + Rosenpass negotiation in parallel |
| Data plane | WireGuard kernel module — 32 bytes overhead |
| Maturity | [NetBird integrated](https://netbird.io/knowledge-hub/how-we-integrated-rosenpass), Mullvad exploring, not enterprise-hardened |

### Strengths

- WireGuard's minimal data-plane overhead (32B vs 62-96B)
- Curve25519 key alignment with RNS for the classical layer — direct key reuse
- Two independent PQC algorithms (hedging against single-algorithm breaks)
- Formally verified protocol, Rust implementation
- Simpler WireGuard data plane

### Weaknesses

- Classic McEliece's 261KB public key is brutal on constrained links
- Not FIPS-certified, not standards-track (yet)
- Adds a second daemon (operational complexity)
- Less battle-tested than strongSwan

## Option C: Plain WireGuard (Classical Degradation Path)

For nodes that don't need PQC or can't run strongSwan/Rosenpass — constrained devices, low-threat environments, or interop with existing WireGuard infrastructure.

The X25519 key reuse from RNS identity makes this trivially easy:

1. RNS link activates
2. Extract peer's X25519 public key from link identity
3. Configure WireGuard peer entry directly
4. Tunnel comes up — no additional key exchange needed

32 bytes per-packet overhead. No PQC. Useful as the "less interesting" fallback.

## Recommended Architecture

```
┌─────────────────────────────────────────────────────┐
│ Phase 1: Sneaky Discovery (RNS)                     │
│                                                     │
│  RNS Link over any bearer (LoRa, TCP, UDP, I2P)    │
│  ├─ Proves peer liveness + identity (Ed25519)       │
│  ├─ Exchanges: IP endpoint, capabilities, nonces    │
│  ├─ Confidential TODAY (X25519), not PQ-safe        │
│  └─ Minimal data exchanged — limits harvest value   │
│                                                     │
│  Rule: treat as eventually public signaling channel  │
├─────────────────────────────────────────────────────┤
│ Phase 2: PQC Tunnel Establishment                   │
│                                                     │
│  Tier 1 (PQC required):                             │
│    strongSwan IKEv2: X25519 + ML-KEM-768 (RFC 9370)│
│    Triggered by RNS link activation                 │
│    PSK seeded from RNS link (binding, not PQC)      │
│                                                     │
│  Tier 2 (PQC not required / degradation):           │
│    WireGuard: RNS X25519 pubkey → peer config       │
│    Direct key reuse, minimal overhead               │
│                                                     │
│  Watch list:                                        │
│    Rosenpass + WireGuard if it reaches NIST stdlib  │
├─────────────────────────────────────────────────────┤
│ Phase 3: High-Bandwidth Data Plane                  │
│                                                     │
│  ESP (strongSwan) or WireGuard tunnel               │
│  ├─ Carries LXMF, IP traffic, any payload           │
│  ├─ Kernel-accelerated encryption                   │
│  └─ BATMAN-adv L2 mesh can run inside tunnel        │
└─────────────────────────────────────────────────────┘
```

## IPsec Overhead Context

The overhead objection from the earlier analysis (strongSwan adds 62-96 bytes per packet against RNS's 500-byte MTU) dissolves in this architecture because:

- The PQC tunnel runs over **traditional internet** (Ethernet, WiFi, fiber), not over constrained RNS bearers
- RNS handles discovery over whatever bearer is available (including LoRa)
- The tunnel is a separate channel — it doesn't wrap RNS packets in ESP
- On a 1500-byte Ethernet MTU, 62-96 bytes of ESP overhead is ~5% — negligible

## Open Questions

1. **Credential bridge implementation** — how exactly does the RNS daemon signal strongSwan to establish an SA? Options: `swanctl --initiate` via CLI, VICI protocol (programmatic), or systemd socket activation.
2. **Capability negotiation** — how do peers advertise PQC support during RNS discovery? A capability byte in the link metadata could indicate: `0x00` = no tunnel, `0x01` = WireGuard only, `0x02` = strongSwan PQC, `0x03` = either.
3. **RNS link as metadata risk** — should the RNS discovery phase itself run over I2P or Tor when internet-connected, to reduce HNDL exposure of the signaling channel?
4. **Rosenpass trajectory** — monitor for NIST standardization of Classic McEliece and potential adoption of ML-KEM in Rosenpass, which would make it a more compelling alternative.

## References

- [strongSwan 6.0.0 Release](https://strongswan.org/blog/2024/12/03/strongswan-6.0.0-released.html)
- [RFC 9370: Multiple Key Exchanges in IKEv2](https://www.rfc-editor.org/rfc/rfc9370.html)
- [draft-ietf-ipsecme-ikev2-mlkem: ML-KEM in IKEv2](https://www.ietf.org/archive/id/draft-ietf-ipsecme-ikev2-mlkem-03.html)
- [strongSwan Algorithm Proposals Docs](https://docs.strongswan.org/docs/latest/config/proposals.html)
- [Rosenpass Project](https://rosenpass.eu/)
- [Rosenpass GitHub](https://github.com/rosenpass/rosenpass)
- [NetBird Rosenpass Integration](https://netbird.io/knowledge-hub/how-we-integrated-rosenpass)
- [Integrating PQC into strongSwan (Kivicore)](https://kivicore.com/en/embedded-security-blog/integrating-pqc-into-strongswan)
- [Reticulum Manual](https://markqvist.github.io/Reticulum/manual/understanding.html)
