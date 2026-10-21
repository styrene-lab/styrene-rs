# Styrene RNS core profiles

The package is `styrene-rns`. Its Rust library name is `rns_core`.

This source provides hosted and allocation-backed `no_std` profiles. Use a
reviewed revision containing the portability fix. This documentation does not
assert that an earlier published package contains the fix.

## Feature selection

| Selection | Contract |
|---|---|
| Default features | Enables `std` and hosted conveniences |
| `default-features = false` | Allocation-backed protocol core without the standard library |
| Defaults disabled, `alloc` enabled | The same allocation-backed core; `alloc` is an explicit compatibility marker |
| `std` | Hosted OS-randomness conveniences, filesystem persistence, and dependency `std` capabilities |
| `transport` | Hosted transport/runtime dependencies; implies `std` |
| `serial` | Hosted serial/KISS support; implies `transport` |
| `fernet-aes128` | Existing alternate Fernet format; does not itself enable `std` |

Set `default-features = false` on the RNS dependency for a no-std consumer.
Enabling `alloc` without disabling defaults does not disable `std`.
Neither minimal selection is allocation-free. Features are additive: another
consumer in the same Cargo graph can re-enable RNS or dependency `std` features.

## Supported embedding contract

The initial target contract is `aarch64-unknown-none` with the repository's
pinned Rust 1.97 toolchain. The embedding must supply:

- An allocator, startup/linking, and panic/allocation-failure policy.
- A healthy, initialized cryptographic randomness provider.
- Current Unix time and updates through the RNS time interface.
- 64-bit and pointer-width atomic support with suitable memory attributes.

The crate uses heap-backed collections and atomically reference-counted callbacks.
It does not promise recoverable allocation failure. Cortex-M targets without the
required atomics are outside this profile's current support envelope.

This is a protocol-core contract, not an embedded daemon or transport port.
Tokio transport, filesystem persistence, and hosted lock-based key-manager
implementations remain outside the allocation-backed profile. Embeddings can
implement the available generic key-backend contracts where appropriate.

## Randomness and compatibility

Pass `&mut rng` through the portable APIs to advance one provider state. A
non-`Copy`, non-`Clone` cryptographic provider is supported by these paths:

- `group_encrypt_with_rng` for group encryption.
- Inherent `encrypt_with_rng` methods on concrete identity types.
- Existing key-derivation, ratchet-encryption, announce, and path-response methods.
- `CachedFernet::encrypt` for encryption with externally owned provider state.

Ratchet rotation uses the caller's provider. Group and ratchet decryption require
no entropy. Concrete identity `decrypt_with_derived_key` methods also require no
RNG. Authentication still precedes plaintext decryption.

The old `group_encrypt` convenience is available with `std` and uses OS randomness.
Existing identity trait signatures and immutable `Fernet::encrypt` remain for
compatibility. Their `Copy`-bounded provider contract is for handles to advancing
sources, not copies of a PRNG state snapshot. Use the portable borrowed-state
entry points for a stateful generator.

Infallible RNG operations may panic if the provider fails, even when an outer
API returns `Result`. These APIs do not promise typed entropy-failure recovery.
Do not replace unavailable production entropy with a deterministic or weak source.
Synthetic providers in the validation fixture are not production examples.

Wire framing, cryptographic key slicing, authentication errors, and existing
hosted persistence formats remain unchanged. No migration is required by this fix.

## Protocol time without std

In the allocation-backed profile, use `time_source::set_unix_time` to supply current Unix seconds. Refresh that
value or use `advance_unix_time` as time progresses. `clear_unix_time` marks time
unavailable again.

Before time is supplied, time-dependent announce and ratchet operations return
`RnsError::TimeUnavailable`. They do not substitute an epoch timestamp or host
clock. Both operations use the same supplied time source.

Coordinate clock reset with concurrent protocol operations. Reset is not a
transaction across the clock's separate initialization and value atomics.

With `std` enabled, RNS uses the hosted system clock. The three embedding-controlled
clock functions above are not available in that profile.

## Validation and evidence limits

The [independent consumer fixture](../../../tests/compile/rns-no-std/README.md)
defines the isolated dependency, target, execution, and negative-control gates.
It avoids workspace/dev-dependency feature restoration. Host-side build tools
may use `std`; target-side libraries may not.

For focused package checks, first select the pinned Cargo/rustc and ensure target
libraries are available:

```sh
cargo check --offline --locked --lib -p styrene-rns \
  --no-default-features --target aarch64-unknown-none
cargo check --offline --locked --lib -p styrene-rns \
  --no-default-features --features alloc --target aarch64-unknown-none
cargo check --offline --locked --lib -p styrene-rns \
  --no-default-features --features alloc,fernet-aes128 --target aarch64-unknown-none
```

These checks do not replace the independent consumer and execution gates.
Likewise, defaults-disabled tests on a native host do not prove a no-std graph.
The complete hosted interop suite uses `--features interop-tests,transport` and
the declared native-library environment.

QEMU execution tests synthetic protocol operations, not physical hardware,
radio links, live interoperability, or the portability of the whole Styrene stack.
The fixture-only boot package contains a separately reviewed unsafe boundary.
Product code and the smoke library retain `unsafe_code = "forbid"`.
