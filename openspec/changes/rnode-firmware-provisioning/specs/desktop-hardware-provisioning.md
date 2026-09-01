# Desktop RNode Hardware Provisioning - Delta Spec

## ADDED Requirements

### Requirement: Desktop exposes full-machine operations by executor capability

The desktop application can expose inspection, upgrade, fresh installation,
provisioning, and recovery only when a bounded executor declares support for the
exact target and operation.

#### Scenario: Known ESP32 target is inspected
Given an ESP32 target has exact board, radio, revision, and serial bootloader evidence
When the desktop application creates an upgrade plan
Then it selects the admitted ESP serial executor and image layout
And it presents the expected destructive regions and recovery procedure

#### Scenario: Unknown serial target is inspected
Given a serial device responds with incomplete or ambiguous target metadata
When the desktop application evaluates firmware operations
Then it presents the observed facts without selecting an artifact
And it performs no bootloader reset or write

### Requirement: Desktop fresh installation includes provisioning

Fresh installation must include the board-specific application and all required
RNode product, model, radio, hardware revision, identity, signature, console,
and target-hash steps declared by the admitted plan.

#### Scenario: Fresh installation omits required metadata
Given a board application was written successfully
When required RNode metadata or signature steps remain incomplete
Then the operation does not report provisioned success
And the device remains unavailable for automatic runtime use

### Requirement: Desktop executors preserve undeclared regions

An executor must write only manifest-declared regions and must preserve
provisioned configuration unless the confirmed operation explicitly replaces it.

#### Scenario: Upgrade plan preserves configuration
Given an existing configured RNode has an admitted application upgrade plan
When the desktop executor writes the declared application regions
Then it does not alter undeclared identity or configuration regions

#### Scenario: Image layout overlaps a protected region
Given a candidate plan includes an image region that overlaps protected state
When the planner validates the layout
Then it rejects the plan before target reset or erase

### Requirement: Desktop recovery remains explicit

Recovery operations must identify the required physical mode, tool, artifact,
power condition, and post-recovery verification. Recovery must not run as an
automatic retry after a failed upgrade.

#### Scenario: Upgrade fails after erase
Given a desktop upgrade fails after a destructive phase starts
When the operation reaches a terminal failure
Then the application presents the exact admitted recovery workflow
And it requires a new explicit confirmation before recovery begins
