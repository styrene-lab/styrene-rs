# Interoperability Verification - Delta Spec

## ADDED Requirements

### Requirement: Upstream interoperability is bidirectional and revision pinned

Parity gates must run pinned Python RNS, LXMF, and NomadNet revisions and exercise Python-to-Rust and Rust-to-Python behavior where the protocol has two endpoints.

#### Scenario: LXMF direct gate runs
Given the pinned Python and Rust implementations are available
When the direct LXMF gate executes
Then each implementation sends and receives a canonical message
And content, fields, method, message ID, signature state, and lifecycle evidence are compared

#### Scenario: Reference revision changes
Given a pinned upstream revision is intentionally updated
When interoperability fixtures and live gates run
Then evidence records the old and new revisions
And changed bytes or semantics require explicit review rather than silent fixture replacement

#### Scenario: Reference metadata disagrees for one revision
Given provenance and evidence registries assign different versions to the same upstream revision
When interoperability validation runs
Then dependent compatibility gates fail closed
And the inconsistency must be resolved from the pinned upstream source before release metadata is generated

### Requirement: Parity gates retain structured evidence

Every live scenario must use the shared runner, bounded topology, deadlines, cleanup, assertions, and machine-readable evidence consumed by CLI, CI, and Lab.

#### Scenario: Live scenario succeeds
Given a live interoperability scenario reaches every required milestone
When the runner finalizes the scenario
Then it records topology, revisions, observations, assertions, timings, and artifacts
And process exit alone is not considered protocol success

#### Scenario: Live scenario times out
Given a live scenario exceeds its deadline
When the runner terminates it
Then every owned process and temporary resource is cleaned up
And retained evidence identifies the last completed protocol milestone

### Requirement: Ordinary validation remains deterministic and offline

Unit, component, fixture, and ordinary workspace tests must not require Python processes, external networking, or hardware.

#### Scenario: Ordinary validation runs offline
Given no Python packages, network access, or serial hardware are available
When ordinary workspace validation executes
Then deterministic tests complete without skipping required offline assertions
And live parity scenarios remain isolated to the dedicated interoperability gate

### Requirement: Release claims require passing non-ignored gates

A claim level may become supported only when all required tests are enabled, passing, and mapped to its acceptance scenarios.

#### Scenario: One required gate is ignored
Given all but one required parity scenario pass
And the remaining scenario is ignored
When release support metadata is generated
Then the claim does not become supported
And the missing gate is listed with its reason
