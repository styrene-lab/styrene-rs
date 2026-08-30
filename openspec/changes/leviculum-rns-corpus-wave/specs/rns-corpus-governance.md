# RNS Corpus Governance - Delta Spec

## ADDED Requirements

### Requirement: Corpus metadata waits for Reticulum 1.5 authority and schema

The evidence wave must consume the Python RNS 1.5.1 authority, fixture schema,
and provenance validation contract owned by `reticulum-1-5-parity-wave`. It
must not define a competing authority, schema, fixture root, or canonical
generator. Leviculum metadata may extend the owner-provided schema only through
its declared extension mechanism.
Canonical controls must load `tests/interop/fixtures/rns/index-v2.json` through
`styrene_interop_runner::rns_fixtures` and select `rns-1.4.2` or `rns-1.5.1`
authority IDs. Leviculum-specific provenance remains category metadata and must
not become a canonical RNS authority record.

#### Scenario: Authority and schema are unavailable
Given the Reticulum 1.5 parity wave has not published its authority record or schema contract
When a Python-derived or metadata-dependent Leviculum case is prepared
Then the case is classified blocked with the Reticulum-wave dependency
And no local authority, schema, or canonical fixture substitute is created

#### Scenario: Schema extension is available
Given the Reticulum 1.5 parity wave publishes a validated extension mechanism
When Leviculum category metadata and an independent case record are admitted
Then the records conform to the owner-provided schema and immutable Python authority
And existing canonical provenance remains owned by the Reticulum wave

### Requirement: Leviculum remains a clean-room category reference

Leviculum at `9d5de12dcb9b236b7ef02dc3b88cd2fafcc8efa1`, licensed
AGPL-3.0-or-later, may provide only plain-language scenario categories and
failure shapes. Leviculum source, tests, harnesses, constants, generated
fixtures, binaries, logs, and pass results must not enter Styrene code,
fixtures, builds, expected results, or software test processes.

#### Scenario: Leviculum-derived input is proposed
Given an executable test, fixture, expected byte sequence, generator, dependency, or build input originates from Leviculum
When clean-room admission runs
Then the input is rejected and classified invalid
And no compatibility, provenance, or corpus-pass claim is emitted

#### Scenario: Independent category case is proposed
Given a case cites only the Leviculum repository, immutable revision, license, and plain-language category
When reviewers inspect its authority, inputs, schedules, names, and assertions
Then admission requires independent authorship from protocol authority and current Styrene design
And unexplained source-level similarity or implementation constants cause invalid classification

### Requirement: Every case has one evidence classification

Every case must terminate as exactly one of `green`, `red-confirmed`, `invalid`,
or `blocked`. The record must include immutable revisions, case and schedule
identity, evidence class, limits, observations, and artifact digests appropriate
to that classification.

#### Scenario: Unchanged Styrene satisfies a valid case
Given a valid independently authored case executes against unchanged Styrene
When all expected and forbidden observations are evaluated reproducibly
Then the case is classified green with its observation digest
And no production edit is authorized

#### Scenario: Reproducible behavior mismatch is confirmed
Given a valid case reproducibly conflicts with current Styrene authority
When harness, provenance, environment, and authority review confirm the mismatch
Then the case is classified red-confirmed and names its behavior owner
And a separate behavior-owned OpenSpec is opened before any production edit

#### Scenario: Case or assumption is wrong
Given a case has invalid provenance, assertions, assumptions, or declared scope
When evidence review detects the defect
Then the case is classified invalid with its rejection reason
And it does not open a production behavior change

#### Scenario: Required dependency is unavailable
Given authority, schema, test seam, platform capability, registration, hardware, or a required decision is unavailable
When the case reaches that dependency
Then the case is classified blocked with an owner and unblock condition
And no pass, behavior mismatch, or claim is inferred

### Requirement: Evidence classifications cannot become claims in this wave

This change must preserve evidence classes and classifications without
registering or enabling live gates or generating support claims. Only
`reticulum-lxmf-nomadnet-parity` task `12.9` may consume the ledger to generate
claims after its required gates exist and pass.

#### Scenario: Evidence ledger is complete
Given all applicable cases have terminal classifications
When the evidence wave publishes its final ledger
Then the ledger is handed to parity task `12.9` without a support-level conclusion
And no lower evidence class is promoted into live Python or physical LNode proof

#### Scenario: Red-confirmed case has no follow-up OpenSpec
Given a case is classified red-confirmed
When final evidence validation checks its behavior ownership
Then validation fails until a separate behavior-owned OpenSpec ID is recorded
And this evidence wave remains unauthorized to edit production behavior
