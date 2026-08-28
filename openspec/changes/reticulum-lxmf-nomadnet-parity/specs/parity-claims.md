# Parity Claims - Delta Spec

## ADDED Requirements

### Requirement: Compatibility claims are evidence scoped

The product must publish separate support claims for RNS primitives, Reticulum operations, LXMF direct messaging, LXMF propagation, Micron rendering, and native NomadNet transport.

#### Scenario: Primitive evidence exists without workflow evidence
Given Python-generated fixtures prove an RNS packet primitive
When product support claims are generated
Then the primitive may be marked supported
And the related operator workflow remains unverified until its end-to-end gate passes

#### Scenario: Styrene transport differs from an upstream protocol
Given a workflow uses a Styrene-specific CBOR envelope
When compatibility status is displayed
Then the workflow is identified as Styrene-specific
And it is not presented as native LXMF propagation or NomadNet transport

#### Scenario: Application workflow observation exists without a protocol gate
Given the application-parity corpus records an observed external workflow
When protocol support claims are generated
Then the observation may support only its recorded product-workflow scope
And no RNS, LXMF, propagation, receipt, or NomadNet claim is promoted without its pinned interoperability gate

### Requirement: Unsupported parity remains explicit

Unsupported, partial, ignored, or manually verified behavior must be visible to operators and release tooling.

#### Scenario: Required interoperability gate is ignored
Given a required parity test is ignored or requires manual execution
When release parity is evaluated
Then the claim remains partial or unverified
And the recorded manual result is not treated as a passing automated gate

#### Scenario: Capability is unavailable at runtime
Given the connected daemon does not advertise a required capability
When the related control or status view renders
Then the operation is unavailable with the daemon reason
And the UI does not emulate or infer the missing behavior
