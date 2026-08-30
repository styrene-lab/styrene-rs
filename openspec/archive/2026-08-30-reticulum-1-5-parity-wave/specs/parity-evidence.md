# Parity Evidence - Delta Spec

## ADDED Requirements

### Requirement: Reticulum 1.5 evidence has immutable provenance

This wave must own shared RNS fixture index schema version 2 and the `rns-1.5.1` authority record. Every canonical fixture and differential result must identify an authority ID, canonical repository, full revision, release, generator, source symbols, typed expected outcome, artifact path, and SHA-256.

#### Scenario: Fixture provenance is validated
Given a committed Reticulum 1.5 fixture is loaded
When fixture validation runs
Then both full commit hashes and the canonical repository URL match the wave manifest
And the fixture checksum, source symbols, and typed expected outcome match retained evidence

#### Scenario: Existing 1.4.2 fixture is indexed
Given an existing RNS 1.4.2 fixture has a stable artifact path, ID, bytes, and checksum
When fixture index schema version 2 is introduced
Then the fixture references authority `rns-1.4.2` without changing those existing values
And new 1.5.1 vectors reference the separate `rns-1.5.1` authority

### Requirement: Cross-wave fixture consumers share one authority

Beechat, FreeTAK, and Leviculum waves must consume this wave's RNS fixture index and authority IDs rather than create competing canonical RNS schemas or authority records.

#### Scenario: Consumer wave adds fixture coverage
Given a Beechat, FreeTAK, or Leviculum wave needs canonical RNS evidence
When it registers or references a fixture
Then it uses fixture index schema version 2 and an authority ID owned by this wave
And implementation-specific provenance remains distinct from canonical RNS authority

### Requirement: Ordinary validation is offline

Normal unit, component, adversarial, fixture, formatting, lint, and OpenSpec validation must not use network access, launch Python, mutate upstream refs, or require hardware.

#### Scenario: Normal tests run in isolation
Given network access, Python packages, serial hardware, and mutable upstream checkouts are unavailable
When ordinary validation executes
Then all required offline assertions run deterministically
And live differential scenarios remain isolated to the dedicated pinned interoperability gate

### Requirement: Existing parity work is not duplicated

This wave must consume the existing shared interoperability runner and cross-reference unfinished broad Reticulum, LXMF, propagation, and NomadNet gates instead of creating competing claims or runners.

#### Scenario: Live parity evidence is planned
Given a behavior requires a live Python and Rust topology
When its release gate is registered
Then it uses the existing revision-pinned shared runner and retained evidence format
And dependency on the corresponding `reticulum-lxmf-nomadnet-parity` task is explicit
