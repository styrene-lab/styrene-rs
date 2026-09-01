# Mobile BLE Firmware Update - Delta Spec

## ADDED Requirements

### Requirement: Mobile firmware capability is BLE upgrade only

The iOS application must offer firmware updates only for an exact nRF52 target
whose BLE DFU bootloader and canonical application package have physical
acceptance evidence. It must not offer ESP32, AVR, generic USB, bootloader, or
recovery operations.

#### Scenario: Configured accepted nRF52 RNode is available
Given the application inspected an accepted nRF52 RNode over normal BLE
When the matching DFU bootloader becomes discoverable
Then the application can execute the admitted application upgrade
And it identifies desktop recovery as the fallback

#### Scenario: ESP32 RNode is available over BLE
Given a configured ESP32 RNode exposes the normal RNode BLE service
When the application evaluates firmware capability
Then mobile firmware update is unavailable
And the application directs the operator to a desktop USB workflow

### Requirement: Normal BLE and DFU are separate sessions

The application must stop normal RNode traffic before DFU. It must treat the
DFU bootloader service and identity as a separate bounded session.

#### Scenario: Operator enters DFU mode
Given the approved RNode is connected through the normal NUS session
When the operator starts an admitted firmware upgrade
Then the application closes the NUS session before scanning for DFU
And no mesh payload traffic shares the DFU session

#### Scenario: A stale DFU callback arrives
Given a newer firmware session generation replaced an earlier session
When the earlier session emits progress or completion
Then the application rejects the stale event
And the current session state does not change

### Requirement: Mobile interruption fails closed

The mobile application must remain foregrounded during DFU and must distinguish
safe pre-write cancellation from interruption after a destructive phase starts.

#### Scenario: Operator cancels before transfer
Given DFU has not entered its destructive transfer phase
When the operator cancels the operation
Then the operation ends without a recovery requirement

#### Scenario: BLE disconnects during transfer
Given DFU entered its destructive transfer phase
When the BLE connection is interrupted
Then the operation does not report cancellation or success
And it requires the board-specific recovery workflow

### Requirement: Fresh BLE installation is evidence gated

A board with a factory BLE DFU bootloader must not receive a fresh-install
support claim until application installation and complete RNode provisioning are
physically verified for that exact board and bootloader revision.

#### Scenario: Factory bootloader advertises DFU
Given a board has no configured RNode application but advertises a compatible DFU service
When fresh-install acceptance evidence is incomplete
Then the mobile application does not offer fresh installation
And desktop provisioning remains the supported path
