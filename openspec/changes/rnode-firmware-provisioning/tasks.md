# RNode Firmware Provisioning Tasks

## 1. Contract Corpuses
<!-- specs: firmware-provisioning, mobile-ble-firmware-update, desktop-hardware-provisioning, provisioning-evidence -->

- [x] 1.1 Define separate capability cases for upgrade, fresh installation, provisioning, and recovery
- [x] 1.2 Define artifact admission cases for signatures, hashes, archive bounds, target matching, and protected regions
- [x] 1.3 Define workflow cases for immutable confirmation, cancellation, interruption, stale generations, recovery, and post-write verification
- [x] 1.4 Add a standard-library corpus validator and mutation tests without adding a device write path

## 2. Shared Firmware Contract
<!-- specs: firmware-provisioning, provisioning-evidence -->

- [x] 2.1 Add failing Rust corpus-consumer tests for target observations, capabilities, plans, manifests, and outcomes
- [x] 2.2 Add canonical RNode target, operation, plan, progress, recovery, and evidence types outside renderer crates
- [x] 2.3 Retain RNode platform, MCU, model, hardware revision, firmware version, running hash, and target hash observations
- [x] 2.4 Implement signed manifest and bounded archive admission against the artifact corpus
- [x] 2.5 Implement immutable plan confirmation, generation checks, and post-write verification against the workflow corpus

## 3. Desktop Full-Machine Provisioning
<!-- specs: desktop-hardware-provisioning, firmware-provisioning, provisioning-evidence -->

- [ ] 3.1 Record the attached Espressif device's exact board, radio variant, revision, bootloader, and recovery method without committing its serial number
- [x] 3.2 Add failing desktop contract tests for inspection, dry-run plans, confirmation, preserved regions, and recovery
- [ ] 3.3 Implement one exact bounded ESP USB executor with no arbitrary command or image input
- [ ] 3.4 Expose inspect, plan, upgrade, fresh-install, provision, and recovery actions according to executor capability
- [ ] 3.5 Verify power loss, corrupted artifacts, model mismatch, protected regions, post-write hash mismatch, and explicit recovery
- [ ] 3.6 Retain physical acceptance evidence before enabling the exact desktop target claim

## 4. Mobile BLE Upgrade
<!-- specs: mobile-ble-firmware-update, firmware-provisioning, provisioning-evidence -->

- [x] 4.1 Record one RAK4631 board and bootloader revision for acceptance without committing stable peripheral identity
- [x] 4.2 Add failing mobile and Apple-bridge tests for NUS shutdown, DFU discovery, generation rejection, bounded progress, interruption, and recovery
- [x] 4.3 Implement the bounded iOS nRF52 BLE DFU transport without firmware selection policy in the Apple bridge
- [ ] 4.4 Expose application upgrade only for the exact accepted board and bootloader combination
- [ ] 4.5 Verify foreground behavior, manual DFU entry, identity change, MTU and PRN behavior, interruption, reconnect, and post-write hash
- [ ] 4.6 Retain physical acceptance evidence before enabling the mobile BLE upgrade claim

## 5. Fresh BLE Investigation
<!-- specs: mobile-ble-firmware-update, provisioning-evidence -->

- [x] 5.1 Add a denied-by-default corpus row for a factory DFU bootloader without configured RNode state
- [ ] 5.2 Test application install and complete RNode provisioning on one exact factory-bootstrapped board
- [ ] 5.3 Enable fresh BLE installation only if physical evidence proves application, metadata, identity, signature, and post-write verification

## 6. Safety And Release
<!-- specs: firmware-provisioning, mobile-ble-firmware-update, desktop-hardware-provisioning, provisioning-evidence -->

- [x] 6.1 Reconcile the Nex and Styrene ownership boundary before adding an executor
- [x] 6.2 Document firmware GPL source, notices, artifact retention, and distribution obligations
- [ ] 6.3 Run corpus, unit, integration, warning-denied Clippy, desktop package, iOS package, and physical-device gates
- [ ] 6.4 Publish only exact platform, board, bootloader, firmware, executor, and scenario claims supported by retained evidence
