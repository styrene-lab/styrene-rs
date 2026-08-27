# Operator Safety - Delta Spec

## ADDED Requirements

### Requirement: Activity and evidence are correlated and redacted

The console must provide a bounded activity timeline and exportable diagnostics with correlation, provenance, severity, and redaction.

#### Scenario: Correlated operation crosses domains
Given one operator action produces request, network, and terminal events
When the activity timeline renders
Then related events can be inspected by correlation ID
And their domain-specific details remain available

#### Scenario: Sensitive data enters diagnostics
Given an event or response contains a secret or policy-redacted field
When it enters UI state or evidence export
Then the field is removed or redacted before rendering or persistence

### Requirement: The desktop console is operable and testable

The console must support keyboard operation, reduced motion, deterministic fixtures, and bounded desktop smoke tests without requiring Python or network access for ordinary validation.

#### Scenario: Operator uses keyboard navigation
Given the application is open
When the operator navigates routes, lists, inspectors, dialogs, and primary actions by keyboard
Then focus remains visible and ordered
And controls expose meaningful accessible labels

#### Scenario: Reduced motion is enabled
Given the platform or operator requests reduced motion
When the Network page renders
Then continuous force animation and non-essential transitions are disabled
And topology remains usable

#### Scenario: Ordinary validation runs
Given Python references and external network access are unavailable
When ordinary workspace validation runs
Then Fixture and component tests execute
And pinned live scenarios remain confined to the dedicated interop gate

### Requirement: Runtime and mutation safety boundaries are enforced

Operate, Lab, Embedded, and Fixture behavior must preserve explicit capability, network, storage, and confirmation boundaries.

#### Scenario: Fixture attempts external networking
Given the active profile is Fixture
When fixture behavior requests an external network operation
Then the operation is rejected before opening an interface
And a safety diagnostic is recorded

#### Scenario: Destructive action requires confirmation
Given the caller has the capability for a destructive action
When the operator initiates the action
Then the console displays target, parameters, and consequence before submission
And records the confirmed request and terminal outcome
