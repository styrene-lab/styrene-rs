# RNode Firmware Provisioning

## Intent

Add a safe RNode firmware lifecycle to Styrene without treating all boards,
bootloaders, transports, or provisioning operations as equivalent. Establish
the executable contract corpus before product implementation starts.

## Scope

This change defines a shared firmware catalog, update planner, typed operation
state, and evidence policy. Desktop applications can later expose inspection,
fresh installation, upgrade, provisioning, and recovery for explicitly
supported USB or serial targets. Mobile applications can later expose only
board-specific BLE application upgrades that have physical acceptance evidence.

The first desktop evidence target is the attached Espressif USB device after its
exact board and radio variant are recorded. The first proposed mobile evidence
target is one RAK4631 with a recorded bootloader revision. These targets are not
support claims until their corpus cases pass on physical hardware.

This change excludes generic iOS USB access, arbitrary firmware images,
unattended writes, mobile bootloader replacement, inferred model selection, and
a custom ESP32 OTA firmware fork. It does not implement a flasher or device write
path until the contract corpuses and their validators are committed.

## Success criteria

- Machine-readable corpuses define platform capability, artifact admission,
  destructive-operation, interruption, recovery, and post-write verification
  behavior before implementation begins.
- Mobile rejects ESP32, AVR, unknown targets, fresh installation, and recovery
  while allowing only physically accepted nRF52 BLE application upgrades.
- Desktop selects an executor only after exact board, radio variant, hardware
  revision, artifact digest, and recovery policy are known.
- Every destructive operation requires an immutable plan and matching explicit
  confirmation.
- No operation reports success until the reopened RNode matches the planned
  model, firmware version, and running application hash.
