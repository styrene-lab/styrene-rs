# RNode Provisioning Evidence - Delta Spec

## ADDED Requirements

### Requirement: Firmware support claims are corpus derived

Every platform, board, bootloader, operation, and firmware support claim must map
to an executable corpus case and retained acceptance evidence. A related board,
shared MCU, matching BLE service, or successful byte transfer is insufficient.

#### Scenario: Related board passes acceptance
Given one RAK4631 bootloader revision passes mobile BLE upgrade acceptance
When another nRF52 board has no matching physical evidence
Then the second board remains unsupported
And the RAK4631 claim remains limited to the exercised revision and workflow

#### Scenario: Transfer completes without post-verification
Given a firmware executor transfers all bytes without error
When no matching version and running-hash observation is retained
Then the corpus case does not pass
And no support claim is enabled

### Requirement: Evidence distinguishes synthetic and physical cases

Corpus records must identify synthetic contract cases separately from physical
hardware acceptance. Synthetic fixtures can validate policy and state machines
but cannot establish a hardware support claim.

#### Scenario: Synthetic DFU case passes
Given the deterministic mobile DFU state-machine corpus passes
When no physical board evidence is attached
Then implementation conformance can advance
And mobile hardware support remains unverified

### Requirement: Firmware provenance remains immutable

Acceptance evidence must record immutable application, firmware manifest,
upstream source, artifact, executor, application build, device class, bootloader,
and final device hash revisions without committing stable device identifiers.

#### Scenario: Physical acceptance is recorded
Given an exact board completes an accepted update and post-write verification
When the evidence bundle is finalized
Then it records immutable software and firmware provenance
And it omits stable USB serial numbers, BLE peripheral identifiers, and secrets
