# Complete the RNS no_std + alloc profile

## Intent

Resolve [issue #78](https://github.com/styrene-lab/styrene-rs/issues/78) by
making the advertised allocation-backed RNS core usable without the Rust
standard library. Preserve the existing hosted default and protocol behavior.

The starting revision is canonical `main` at
`ab46aa1f13de5663c684225dc27dfcba330d965f`, verified before creating
`fix/rns-no-std-78`. This proposal does not depend on the separate reconciliation
branch, Identity extraction, or unrelated transport work.

At this revision, `styrene-rns` declares conditional `no_std` but includes
`rmp-serde 1.3.0` unconditionally. Other dependency defaults also enable `std`.
Some core paths require `OsRng`, and the embedded clock requires 64-bit atomics.
Existing defaults-disabled CI runs on a hosted target. It does not prove the
target dependency graph is standard-library-free.

## Scope

Included work:

- Isolate `std`, allocation, persistence, and transport dependencies in RNS.
- Complete caller-supplied randomness for the allocation-backed core.
- Preserve RNG-free authenticated decryption and existing time behavior.
- Preserve hosted APIs, security properties, and wire and persistence formats.
- Add an independent consumer, true no-`std` target validation, and CI coverage.
- Document the supported target envelope and embedding responsibilities.

Excluded work:

- Allocation-free RNS or universal MCU/RTOS support.
- Embedded transport drivers, a replacement async runtime, or daemon porting.
- Making LXMF, the UI, or the entire workspace standard-library-free.
- Identity extraction, dependency-pin advancement, or independent Identity work.
- Codec replacement, protocol changes, persistence migration, or dependency upgrades
  that are not required by this fix.
- Host configuration, toolchain installation, deployment, or physical-device work
  as an implicit consequence of preparing this proposal.

## Success criteria

The [delta specification](specs/rns-core-portability.md) defines the required
behavior. [Test criteria](test-plan.md) assign stable IDs `RNS-NOSTD-001` through
`RNS-NOSTD-012` to its acceptance cases.

- Both defaults-disabled profiles compile with an isolated dependency graph on
  the approved standard-library-free target using the pinned toolchain.
- Required randomness comes from the embedding. Decryption does not request entropy.
- Existing clock, authentication, wire, and private-persistence contracts remain valid.
- Default, explicit-`std`, and transport consumers retain their supported behavior.
- An isolated target consumer links and executes the agreed synthetic smoke test.
- CI detects a target-side `std` regression. Evidence distinguishes compilation,
  linking, execution, and hardware acceptance.

## Decisions still required

The user has authorized supervised local implementation and selected the
atomic-capable `aarch64-unknown-none` target. Cortex-M support is excluded.
This target choice is not physical-board acceptance.

[Design decisions](design.md#open-decisions) must resolve the runtime runner and
entropy API contract before their affected implementation tasks proceed.
The [execution plan](execution-plan.md) defines parallel ownership, immutable
handoffs, review, and delivery authority. Missing target or runtime evidence
must remain blocked, not passed.
