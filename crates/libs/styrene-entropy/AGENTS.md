# styrene-entropy

Entropy accumulator, HMAC-DRBG, and pluggable source abstraction for Styrene mesh nodes. Callers interact with `Drbg::fill_bytes()` and never see individual entropy sources.

## Module Map

| File | Purpose |
|------|---------|
| `src/lib.rs` | Re-exports, `#![forbid(unsafe_code)]` |
| `src/drbg.rs` | HMAC-DRBG (SP800-90A SHA-256). K/V state, auto-reseed every 1 MiB, backtracking resistance via post-generation update. Panics if not seeded. |
| `src/pool.rs` | Fortuna-style accumulator with 8 SHA-256 pools. Pool i contributes to reseed j if 2^i divides j. Minimum 32 bytes in pool 0 before reseed. |
| `src/health.rs` | SP800-90B health tests: Repetition Count Test (stuck byte), Adaptive Proportion Test (bit bias). `HealthChecker` combines both. |
| `src/source/mod.rs` | `EntropySource` trait: `source_id()`, `health()`, `poll(pool)`. Feature-gated concrete sources below. |
| `src/source/kernel.rs` | `KernelSource` -- `/dev/random` via `getrandom` crate |
| `src/source/jitter.rs` | `JitterSource` -- CPU timing jitter, fallback for constrained devices |
| `src/source/hardware.rs` | `HardwareSource` -- nRF52840 UART entropy coprocessor via `serialport` |
| `src/source/mesh.rs` | `MeshHubSource` -- ENTROPY_REQUEST RPC to Hub (stub) |

## Key Types

- **`Drbg`** -- the abstraction boundary. Owns an `EntropyPool`, exposes `fill_bytes()`, `reseed()`, `add_entropy()`. ZeroizeOnDrop.
- **`EntropyPool`** -- Fortuna accumulator. `add(source_id, data)`, `drain_seed() -> Option<[u8; 32]>`, `ready()`.
- **`SourceId`** -- u8 label: `KERNEL=0x00`, `JITTER=0x01`, `HARDWARE=0x02`, `MESH_HUB=0x03`. 0x00-0x7F reserved for built-in.
- **`EntropySource`** -- trait for pluggable sources. `poll(&mut self, pool: &mut EntropyPool)`.
- **`SourceHealth`** -- enum: `Ok`, `Degraded(String)`, `Unavailable`
- **`HealthChecker`** -- combines RCT + APT tests. Feed source output through `update()`.
- **`RepetitionCountTest`** -- fails after N consecutive identical bytes (default 8)
- **`AdaptiveProportionTest`** -- fails if 1-bit proportion outside 15%-85% over 512-bit window

## Feature Flags

| Feature | Default | Enables |
|---------|---------|---------|
| `kernel` | yes | `KernelSource` via `getrandom` |
| `jitter` | no | `JitterSource` -- CPU timing jitter |
| `hardware-trng` | no | `HardwareSource` -- nRF52840 UART (serialport) |
| `mesh-source` | no | `MeshHubSource` stub -- ENTROPY_REQUEST RPC |

## Test Commands

```bash
cargo test -p styrene-entropy
cargo test -p styrene-entropy --all-features
```

## Gotchas

- **Drbg panics if not seeded**: `fill_bytes()` asserts `self.seeded`. You must call `reseed()` or `reseed_from_pool()` after adding at least 32 bytes to pool 0.
- **Auto-reseed is best-effort**: After `RESEED_INTERVAL` (1 MiB) bytes, `fill_bytes()` tries `reseed_from_pool()`. If the pool is not ready (no fresh entropy), it continues with the existing state. The caller is responsible for feeding the pool.
- **Pool 0 always gets all input**: `EntropyPool::add()` always feeds pool 0 in full. Pools 1-7 only receive data from inputs larger than 16 bytes, split into chunks.
- **Forward separation on drain**: `drain_seed()` runs SHA-256 twice (outer hash of pool digests, then hash the result) to separate consecutive seeds.
- **EntropyPool is not Send**: Wrap in `Mutex` for shared access.
- **SourceId 0x00-0x7F reserved**: Custom sources should use 0x80+.
- **mesh-source is a stub**: The `MeshHubSource` exists but is not wired to styrene-ipc yet.
- **Uses workspace deps**: `hmac`, `sha2`, `zeroize`, `thiserror`, `log`, `tokio` come from workspace Cargo.toml.

## Current Status

- DRBG and pool are complete and tested
- Health monitoring (SP800-90B RCT + APT) is complete
- `KernelSource` is complete and tested
- `JitterSource`, `HardwareSource` exist but are less exercised
- `MeshHubSource` is a stub waiting for styrene-ipc integration
