# RNS Live Case Handoff - Delta Spec

## ADDED Requirements

### Requirement: Existing parity tasks solely own live registration and claims

`reticulum-lxmf-nomadnet-parity` tasks `4.7` and `5.7` must solely own live
scenario registration, task `12.6` must solely own gate enablement, and task
`12.9` must solely own support claims. This evidence wave may provide only
runner-compatible schedules, assertions, cases, metadata, and classified
evidence for those owners.

#### Scenario: Live case package is ready
Given an independently authored live case has authority, provenance, schedule, assertions, limits, and required evidence classes
When this wave completes its case package
Then the package is handed to parity task `4.7` or `5.7` as applicable
And this wave does not register, enable, schedule, or claim the live gate

#### Scenario: Parity owner cannot represent the case
Given the existing runner or parity-owned catalog cannot represent a required case contract
When the handoff owner reviews the package
Then live evidence for the case is classified blocked with the owner and missing capability
And no duplicate runner, catalog, topology allocator, wrapper, or report is created

### Requirement: Live case packages fit the existing runner contract

Live case packages must declare revisions, topology needs, ordered milestones,
assertions, deadlines, bounded artifacts, cancellation, and cleanup through the
existing `styrene-interop-runner` contract. Contract validation in this wave is
not catalog registration.

#### Scenario: Runner-compatible manifest validates
Given a live case manifest declares all existing runner contract fields
When evidence-wave contract validation runs
Then the manifest is accepted as a handoff artifact
And no runner scenario ID or production catalog entry is added by this wave

### Requirement: Live evidence classes remain separate

Pinned Python RNS and physical LNode executions must be separate parity-owned
gates. Their results must remain separate from in-memory and PTY evidence and
from each other. Missing Python, registration, platform capability, or hardware
must produce `blocked`, not green.

#### Scenario: Python raw serial evidence is returned
Given parity task `5.7` registered a pinned Python RNS SerialInterface case and task `12.6` enabled it
When parity-owned execution returns bidirectional announce and data evidence
Then this wave records the result as live Python raw-HDLC evidence
And it does not label the result physical LNode evidence

#### Scenario: Physical LNode evidence is returned
Given parity-owned execution uses the recorded LNode board, firmware digest, transport port, baud, and topology
When announce, path, and data assertions complete
Then this wave records only the declared physical LNode evidence
And it does not treat the device as canonical authority or substitute Python evidence

#### Scenario: Required live dependency is absent
Given registration, enablement, pinned Python, PTY support, or matching LNode hardware is unavailable
When the corresponding evidence is requested
Then the live case is classified blocked with the missing dependency
And no lower evidence class is promoted to satisfy it

### Requirement: Ordinary validation remains isolated from live capabilities

Ordinary corpus validation must not allocate a PTY, launch Python or network
processes, access serial hardware, or invoke a live runner scenario. It must run
the pure in-memory ledgers and validate handoff manifests only.

#### Scenario: Ordinary validation runs offline
Given PTY support, Python, network access, serial hardware, and live gate registration are unavailable
When ordinary evidence validation executes
Then all required in-memory cases and handoff-contract checks run without those capabilities
And unavailable live cases are not executed or counted as green
