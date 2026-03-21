# Entropy Architecture

**Date:** 2026-03-21  
**Status:** Design — implementation not started  
**Prerequisite for:** PQC integration (Gap 3.2), edge identity provisioning, Hub entropy pool

---

## Why This Exists

The Rust daemon and Hub binary generate cryptographic key material in several places:

- **RNS identity creation** — 32-byte X25519 seed + 32-byte Ed25519 seed, generated once per device
- **Ephemeral link keypairs** — per-link X25519, generated at link setup
- **ML-KEM-768 key generation** — ~800+ bytes of entropy required per key, ~25× the classical cost (see Gap 3.2)
- **IKEv2 PSK derivation** — HKDF input for strongSwan PQC tunnel credential bridge
- **WireGuard session keys** — 32-byte ephemeral per session

The quality of these operations depends entirely on entropy quality at generation time. On constrained edge hardware this is not guaranteed:

| Hardware | TRNG | Risk |
|---|---|---|
| Pi Zero 2W (BCM2710A1) | Hardware RNG in SoC | Low — adequate for classical crypto |
| RP2040 | None — ROSC side-channel only | **High** — no designed entropy source |
| ESP32 (RF off) | Weak without RF subsystem | **High** — fails Dieharder without RF |
| nRF52840 | Thermal noise TRNG, SP800-90B | Low — best in commodity MCU class |
| x86 Hub (RDRAND) | On-die Intel/AMD TRNG | Low — but supply-chain opacity concern |

Fleet provisioning over LXMF store-and-forward makes this worse: devices generate identities offline, autonomously, with no operator present. A weak identity keypair at generation time cannot be fixed later — it is the node's authentication root.

---

## Design: Source → Pool → DRBG → Consumer

The architecture follows Fortuna (Schneier & Ferguson, 2003) adapted for embedded mesh deployment.

```
┌─────────────────────────────────────────────────┐
│  ENTROPY SOURCES  (physical — slow, real)       │
│                                                 │
│  HardwareSource  ──┐                            │
│  KernelSource    ──┼──► EntropyPool             │
│  MeshHubSource   ──┘    (accumulator)           │
└─────────────────────────────────────────────────┘
               │
               │  seed / reseed (256 bits,
               │  infrequent — triggered by policy)
               ▼
┌─────────────────────────────────────────────────┐
│  DRBG  (ChaCha20 — fast, SP800-90A)             │
│                                                 │
│  Unlimited output at CPU speed                  │
│  Forward secrecy on reseed                      │
│  Backtracking resistance                        │
└─────────────────────────────────────────────────┘
               │
               │  fill_bytes(buf)  ← callers never see sources
               ▼
┌─────────────────────────────────────────────────┐
│  CONSUMERS  (black box — indifferent to source) │
│                                                 │
│  RNS identity generation                        │
│  ML-KEM key generation                          │
│  IKEv2 PSK derivation                           │
│  Session key material                           │
└─────────────────────────────────────────────────┘
```

The DRBG is the abstraction boundary. Everything above it calls `drbg.fill_bytes()`. Nothing above it knows whether the seed came from a hardware TRNG, the kernel, or the mesh Hub.

---

## Crate: `styrene-entropy`

New lib crate at `crates/libs/styrene-entropy/`.

### Public API

```rust
/// The only interface consumers touch.
pub struct Drbg { /* ChaCha20-based DRBG, seeded from EntropyPool */ }

impl Drbg {
    pub fn fill_bytes(&mut self, dest: &mut [u8]);
    pub fn reseed_if_due(&mut self, pool: &EntropyPool);
}

/// Accumulates contributions from all configured sources.
pub struct EntropyPool { /* Fortuna-style multi-pool accumulator */ }

impl EntropyPool {
    pub fn add(&mut self, source_id: u8, data: &[u8]);
    pub fn ready(&self) -> bool;           // enough entropy to seed DRBG?
    pub fn drain_seed(&mut self) -> [u8; 32];
}

/// Implemented by each entropy source.
pub trait EntropySource: Send + Sync {
    fn source_id(&self) -> u8;
    fn available(&self) -> bool;
    fn health(&self) -> SourceHealth;
    fn poll(&self, pool: &mut EntropyPool); // contributes bytes if ready
}

pub enum SourceHealth { Ok, Degraded(String), Unavailable }
```

### Sources

```
crates/libs/styrene-entropy/src/
  lib.rs
  pool.rs           # EntropyPool — Fortuna accumulator
  drbg.rs           # ChaCha20-DRBG, forward secrecy, reseed policy
  health.rs         # SourceHealth, stuck-bit detection, bias tests
  source/
    hardware.rs     # nRF52840 via UART or SPI — feature = "hardware-trng"
    kernel.rs       # /dev/random — always available on Linux
    jitter.rs       # CPU jitter — fallback for constrained devices
    mesh.rs         # MeshHubSource — requests entropy from Hub via RPC
```

### Feature flags

```toml
[features]
default = ["kernel"]
kernel = []           # /dev/random — always on Linux
hardware-trng = []    # nRF52840 UART/SPI driver — opt-in
jitter = []           # CPU jitter entropy — for constrained devices without hardware TRNG
mesh-source = []      # MeshHubSource — requires styrene-ipc dep
```

Edge builds targeting RP2040 or ESP32 enable `hardware-trng` (nRF52840 coprocessor attached via UART). Hub builds enable all sources.

### Integration into AppContext (Gap S5)

When `AppContext` (the service registry from structural gap S5) is implemented, `Drbg` lives there:

```rust
pub struct AppContext {
    pub entropy: Arc<Mutex<Drbg>>,   // ← add this
    pub transport: Arc<dyn MeshTransport>,
    pub messages: MessagesStore,
    // ...
}
```

All crypto operations draw from `ctx.entropy`. No caller constructs its own RNG.

---

## Hardware Coprocessor: nRF52840

For edge hardware without a reliable on-chip TRNG (RP2040, ESP32 RF-off), an nRF52840 module attached via UART or SPI provides a verified entropy stream.

**Why nRF52840:**
- Genuine thermal noise TRNG — hardware peripheral, not a side-channel
- Nordic claims NIST SP800-90B compliance; PSA Level 2 certified
- Raw throughput: ~13–21 kB/s (thermal noise limited, ~38–60 µs/byte)
- On-chip CryptoCell CC310 can run CTR-DRBG for conditioned output at MB/s
- UART/SPI native, 1.7–5.5V, $5–15 as a module (XIAO nRF52840, E104-BT5040U, etc.)
- Open firmware possible — no proprietary blob required

**Wire protocol (UART, 1 Mbaud):**

```
Frame: [0xAA] [LEN:u8] [TYPE:u8] [PAYLOAD:LEN bytes] [CRC8:u8]

TYPE = 0x01  ENTROPY_DATA   — N bytes of conditioned DRBG output
TYPE = 0x02  HEALTH_REPORT  — { ok: bool, raw_bias: f32, stuck_bits: u8 }
TYPE = 0x03  REQUEST        — host requests N bytes (response: ENTROPY_DATA)
TYPE = 0x04  RESET          — host requests TRNG reseed cycle
```

The firmware runs a continuous loop:
1. Raw TRNG → accumulate 256 bytes
2. SHA-256 condition → 32-byte seed
3. ChaCha20-DRBG expand → stream to UART
4. Health check every 1024 output bytes — stuck-bit, bias, repetition count test
5. On health failure: emit HEALTH_REPORT(ok=false), halt output, await RESET

The `HardwareSource` in `styrene-entropy` drives this protocol. A health failure transitions the source to `SourceHealth::Degraded` and the pool falls back to kernel/jitter sources — never silently produces degraded output.

**BOM note:** The coprocessor is a natural component of the `styrene-edge` hardware specification for any edge node deployed on RP2040-class hardware. See `styrene/research/entropy-coprocessor.md`.

---

## Reseed Policy

The pool reseeds the DRBG:
- At startup (blocks until pool is ready — never starts with zero entropy)
- Every 1 MB of DRBG output (conservative — can be tuned per device class)
- After any key generation event (ML-KEM, identity creation, IKEv2 PSK)
- On explicit request from the mesh Hub source

This keeps entropy fresh without hammering the hardware TRNG. At ~15 kB/s raw nRF52840 throughput, a 32-byte reseed costs ~2 ms. At the 1 MB reseed interval, that's 2 ms per 1 MB of DRBG output — negligible overhead.

---

## Relationship to PQC (Gap 3.2)

Gap 3.2 notes the `ml-kem` crate is in workspace deps but unused. Before activating it, the entropy layer must be in place:

- ML-KEM-768 key generation requires ~800+ bytes of entropy per key
- The raw nRF52840 TRNG delivers this in ~50 ms — acceptable for key generation events
- Without a quality entropy source, ML-KEM key generation silently degrades
- **The `styrene-entropy` crate is a prerequisite for activating Gap 3.2**

See also: `docs/pqc-tunnel-architecture.md` — the IKEv2 PSK derivation is the second entropy-critical path.

---

## Implementation Priority

This crate is **independent of the service layer migration** (Gaps S5, 2.x). It is a pure lib with no dependency on `AppContext`, `RpcDaemon`, or any application code. It can be built and tested standalone.

Suggested order:
1. `pool.rs` + `drbg.rs` + `kernel.rs` — kernel-backed DRBG, testable on any Linux
2. `health.rs` — health check primitives
3. `source/hardware.rs` — nRF52840 UART driver (requires hardware for integration test)
4. `source/mesh.rs` — MeshHubSource (requires IPC, implement after Hub entropy pool)
5. Wire into `AppContext` when S5 lands

Add to `PARITY_GAPS.md` priority table after 3.2 (PQC Integration) — entropy unblocks PQC, so it should precede it.
