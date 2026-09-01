# RNode Firmware Provisioning - Delta Spec

## ADDED Requirements

### Requirement: Firmware operations have distinct typed meanings

The system must distinguish application upgrade, fresh installation,
provisioning, and recovery. A capability for one operation must not imply a
capability for another operation.

#### Scenario: Mobile upgrade capability is evaluated
Given a configured RNode has a physically accepted BLE DFU bootloader
When the mobile application evaluates its firmware operations
Then it can offer an application upgrade
And it does not infer fresh installation or recovery capability

#### Scenario: Blank hardware is evaluated
Given a board has no recognized application or compatible bootloader evidence
When the system evaluates available firmware operations
Then it requires a desktop recovery executor
And it does not classify the operation as an upgrade

### Requirement: Firmware selection requires exact target evidence

The planner must select an artifact only from an exact supported board, product,
model, radio variant, hardware revision, and executor combination. USB identity,
BLE name, or MCU family alone must not select an artifact.

#### Scenario: USB identity is the only evidence
Given an attached device reports an Espressif USB vendor and product identifier
When the planner lacks exact board and radio variant evidence
Then it permits read-only inspection
And it rejects artifact selection

#### Scenario: Exact target is supported
Given the observed target fields exactly match one admitted manifest entry
When the planner resolves an operation
Then it selects only that entry's artifact and executor
And it records every target field in the immutable plan

### Requirement: Artifact admission is authenticated and bounded

The system must admit only an artifact whose Styrene-controlled signed manifest,
archive digest, expected members, image regions, and application digest pass
validation. Canonical unsigned release metadata alone is insufficient.

#### Scenario: Canonical archive has no admitted signature
Given an archive digest appears in upstream release metadata
When no valid Styrene-controlled manifest signature admits that digest
Then artifact admission fails before device access

#### Scenario: Archive contains an unsafe member
Given a signed manifest identifies an expected archive
When the archive contains path traversal, duplicate, oversized, unexpected, or
overlapping image content
Then artifact admission fails before device access

### Requirement: Destructive execution requires a confirmed immutable plan

The system must separate inspection and planning from destructive execution. An
execution request must bind to the exact plan digest and target observation.

#### Scenario: Confirmation matches the plan
Given a current target observation and an admitted immutable plan
When the operator confirms the exact plan digest
Then the selected executor can begin its declared destructive phase

#### Scenario: Target changes after planning
Given the operator confirmed a plan for one target observation
When the target generation, identity evidence, or plan digest changes
Then execution is rejected
And the operator must inspect and confirm a new plan

### Requirement: Success requires authoritative post-write verification

The system must reopen the RNode after execution and verify its exact model,
firmware version, and running application hash. A completed byte transfer alone
must not report success.

#### Scenario: Reopened RNode matches the plan
Given an executor reports that all planned writes completed
When the reopened RNode reports the planned model, version, and application hash
Then the operation reaches verified success

#### Scenario: Reopened hash differs
Given an executor reports that all planned writes completed
When the reopened RNode reports a different application hash
Then the operation fails verification
And the system provides the declared recovery path

### Requirement: Executable corpuses precede feature implementation

The capability, artifact, and workflow corpuses and their validators must exist
before an executor, firmware UI action, or device write path is implemented.

#### Scenario: Product implementation is proposed
Given one or more firmware contract corpus gates are absent or invalid
When an implementation task would add a device write path
Then that implementation task remains blocked
And no firmware support claim is enabled
