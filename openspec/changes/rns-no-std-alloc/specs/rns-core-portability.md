# RNS core portability - Delta Spec

## ADDED Requirements

### Requirement: Allocation-backed RNS excludes target-side standard-library dependencies

An independent consumer must be able to select the RNS allocation-backed core
without `std`, persistence codecs, hosted transport, or an OS entropy backend.
Both defaults-disabled and explicit-`alloc` selection retain allocation support.
Host-only build tools and procedural macros may use `std`.

#### Scenario: RNS-NOSTD-001 - Independent minimal dependency graph
Given an independent consumer selects `styrene-rns` with default features disabled
When its resolved target dependency graph is inspected
Then no target-side dependency requires `std` or an OS getrandom backend
And `rmp-serde`, `serde_bytes`, and hosted transport dependencies are absent
And workspace siblings and RNS dev-dependencies cannot restore those capabilities

#### Scenario: RNS-NOSTD-002 - Explicit allocation profile
Given an independent consumer enables only the RNS `alloc` feature
When its resolved profile is compared with the defaults-disabled profile
Then both expose the supported allocation-backed core without target-side `std`
And neither profile is described as allocation-free

### Requirement: The allocation-backed core compiles for a documented target contract

The core must compile on the approved standard-library-free target using the
repository's pinned Rust toolchain. The supported contract must state allocator,
atomic, clock, and entropy requirements. A host check is not target acceptance.

#### Scenario: RNS-NOSTD-003 - Standard-library-free target matrix
Given an approved target contract and the pinned toolchain with that target available
When independent consumer libraries are checked for minimal, `alloc`, and `alloc,fernet-aes128` RNS profiles
Then every matrix entry compiles without a target standard library
And the result identifies the target and exact features rather than implying all MCU support

### Requirement: Allocation-backed randomness comes from the embedding

Portable caller-RNG operations must accept mutable, non-`Copy` cryptographic
providers without creating a hidden OS provider. Successive operations through
these paths must advance provider state rather than replay copied state.
Hosted conveniences may supply the OS provider when the hosted feature is enabled.
Legacy trait and stored-RNG compatibility surfaces must document their copyable
provider constraints rather than promise safety for copied PRNG state snapshots.

#### Scenario: RNS-NOSTD-004 - Supplied randomness reaches all core operations
Given an instrumented non-`Copy`, non-`Clone` test RNG and initialized protocol time
When the consumer exercises identity generation, ephemeral encryption, group encryption, announce creation, and ratchet rotation
Then the required key, IV, announce, and ratchet randomness comes from that provider
And successive operations do not reset or replay its consumed state
And the production provider and failure contract are documented separately from the synthetic test RNG
And existing public identity trait implementations and immutable hosted Fernet callers remain source-compatible

### Requirement: Authenticated decryption does not require an entropy source

Core group and ratchet decryption must not construct or invoke an entropy
provider. Removing that dependency must preserve authentication and established
failure outcomes. Plaintext must not be decrypted before authentication succeeds.

#### Scenario: RNS-NOSTD-005 - Decryption remains entropy-free and authenticated
Given valid and invalid group and ratchet tokens with instrumentation that detects entropy requests
When each token is decrypted through the allocation-backed API
Then every operation makes zero entropy requests
And valid tokens return their expected plaintext
And invalid tokens retain the applicable authentication, format, or buffer failure outcome
And failed authentication does not trigger plaintext decryption

### Requirement: Portability profiles preserve existing protocol and hosted contracts

The portability change must preserve the embedded-time, RNS security, and
interop-verification baseline requirements. It must not change RNS wire bytes,
persistence formats, or supported hosted feature relationships.

#### Scenario: RNS-NOSTD-006 - Existing embedded time remains authoritative
Given the existing uninitialized, cleared, supplied, and advanced clock cases
When time-dependent announce and ratchet operations run under the allocation-backed profile
Then missing time returns the typed unavailable-time error
And supplied time governs both announce timestamps and ratchet rotation
And no hidden host clock or synthetic epoch timestamp replaces missing time

#### Scenario: RNS-NOSTD-007 - Protocol and authentication regressions are rejected
Given the existing canonical RNS fixtures and relevant malformed-input cases
When default and applicable alternate crypto profiles are verified
Then canonical wire behavior and failure classifications remain compatible
And authentication uses the existing constant-time verifier before decryption
And tests reuse the shared canonical fixture loader rather than copying its corpus

#### Scenario: RNS-NOSTD-008 - Hosted feature relationships remain supported
Given consumers selecting default, explicit `std`, transport, and supported serial configurations
When their applicable library checks and hosted regression tests execute
Then existing hosted APIs remain usable with the selected features
And transport continues to imply `std`
And no-default selection does not silently enable hosted functionality

#### Scenario: RNS-NOSTD-009 - Standard-feature persistence remains compatible
Given existing key and ratchet persistence data and private-write failure cases
When hosted persistence and key-fallback regression tests execute
Then existing serialized records remain readable without migration
And private atomic replacement and classified fallback behavior remain unchanged

### Requirement: Feature isolation preserves security capabilities and hosted consumers

Disabling dependency defaults must retain the cryptographic and allocation
capabilities used by RNS, including established zeroization features. Direct
hosted consumers must retain their supported dependency contracts.

#### Scenario: RNS-NOSTD-010 - Security features and consumer contracts survive
Given the proposed feature graph and affected LXMF, mesh, tunnel, and daemon consumers
When the dependency feature review and focused consumer checks complete
Then required cryptographic zeroization and allocation capabilities remain enabled
And the checked hosted consumers compile under their documented profiles
And the evidence does not claim those consumers themselves became standard-library-free

### Requirement: Portability acceptance separates compilation from execution evidence

The supported target contract must have an independently linked and executed
synthetic consumer. CI must reject target-side standard-library regressions.
Evidence must identify source state, dependency resolution, toolchain, target,
features, commands, and results. Blocked or unexecuted cases are not successes.

#### Scenario: RNS-NOSTD-011 - An isolated target consumer links and executes
Given the approved target and runner with an allocator, supplied clock, and synthetic test RNG
When the independent no-std executable is linked and run through the agreed smoke workflow
Then the workflow exercises representative core encoding, signing, encryption, decryption, and ratchet behavior successfully
And the record distinguishes target execution from host tests and physical-device acceptance
And synthetic credentials and randomness are not used for live traffic or production identities

#### Scenario: RNS-NOSTD-012 - CI rejects contaminated target builds and incomplete evidence
Given the independent target gate and a disposable negative-control consumer requiring target-side `std`
When the gate evaluates the supported consumer and the negative control
Then the supported consumer passes and the negative control fails for the intended target-side standard-library reason
And retained evidence identifies the exact source state, lockfiles, toolchain, target, features, commands, and results
And missing required checks remain explicitly blocked or unexecuted
