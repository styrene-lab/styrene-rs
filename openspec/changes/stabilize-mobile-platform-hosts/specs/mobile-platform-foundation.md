# Mobile Platform Foundation - Delta Spec

## ADDED Requirements

### Requirement: Mobile hosts own deterministic embedded-node lifecycle

Each mobile host must restore valid persisted configuration, start one embedded
node, expose its actual lifecycle state, and shut down all owned work explicitly.

#### Scenario: Persisted configuration is valid
Given a mobile host has valid persisted node configuration
When the application starts normally
Then the host starts one embedded node without a manual start action
And the displayed lifecycle state comes from that node

#### Scenario: Boot fails after partial composition
Given mobile boot creates one or more owned runtime resources
When a later boot stage fails
Then the host shuts down every resource created by that attempt
And exposes a recoverable failure instead of a running state

### Requirement: Bluetooth is the default mobile RNode bearer

Mobile hosts must use the approved Bluetooth RNode as the default radio bearer.
Android USB must remain an explicit fallback and must not preempt Bluetooth.

#### Scenario: Approved Bluetooth RNode is available
Given the user approved an RNode and Bluetooth is available
When the mobile host starts or the peripheral becomes reachable
Then the host reconnects only to the approved peripheral
And attaches its KISS packet channel to the embedded node

#### Scenario: Android USB device is attached
Given Android has an approved or connected Bluetooth RNode
When a compatible USB RNode is attached
Then Android does not replace Bluetooth automatically
And USB remains available through an explicit fallback action

### Requirement: Bearer interruption preserves bounded outbound work

A temporary mobile bearer interruption must not discard accepted outbound
packets or allow an unbounded pending queue.

#### Scenario: Bearer reconnects
Given the host accepted outbound packets before a bearer interruption
When the approved bearer reconnects within the retention policy
Then the host submits the retained packets in order
And does not submit a retained packet more than once through that queue

#### Scenario: Pending queue reaches capacity
Given the pending outbound queue has reached its configured bound
When another packet is offered
Then the host returns an explicit capacity outcome
And does not allocate an unbounded queue

### Requirement: Mobile evidence identifies its execution boundary

Validation records must distinguish simulator, emulator, physical iOS, physical
Android, and fixture evidence. A lower evidence class must not imply a higher
one.

#### Scenario: Physical Android hardware is unavailable
Given Android tests pass without a connected physical Android device
When mobile validation is reported
Then the report identifies Android as build, unit, and emulator validated
And does not claim physical Android Bluetooth or USB success

#### Scenario: Generated artifacts are produced
Given a mobile build generates bindings, libraries, packages, or runtime logs
When source-control status is inspected
Then those generated artifacts are untracked or ignored
And source inputs remain reviewable without generated content
