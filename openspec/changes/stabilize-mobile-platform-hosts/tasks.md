# Stabilize Mobile Platform Hosts Tasks

## 1. Change Inventory And Commit Boundaries
<!-- specs: mobile-platform-foundation/spec -->

- [x] 1.1 Inventory committed and uncommitted mobile changes against `origin/main`
- [x] 1.2 Separate runtime, Android, iOS, corpus/deployment, and documentation changes by intent
- [x] 1.3 Confirm every generated binding, native library, application package, and runtime artifact is excluded
- [x] 1.4 Record physical-device evidence and explicit evidence gaps in the mobile documentation

## 2. Embedded Lifecycle
<!-- specs: mobile-platform-foundation/spec -->

- [x] 2.1 Add or retain tests for one-node startup, partial-boot cleanup, explicit shutdown, and repeated lifecycle calls
- [x] 2.2 Verify persisted configuration starts the iOS and Android embedded node without a normal-path manual start action
- [x] 2.3 Verify channel detachment pauses packet pumping without destroying Bluetooth approval
- [x] 2.4 Run focused `styrened` mobile and mobile FFI tests with warning-denied Clippy

## 3. RNode Bearers
<!-- specs: mobile-platform-foundation/spec -->

- [ ] 3.1 Add injectable iOS CoreBluetooth tests for NUS discovery, characteristic properties, protected-access failure, negotiated write limits, arbitrary KISS fragmentation, and serialized write-with-response output
- [ ] 3.2 Add injectable Android GATT and USB tests for NUS validation, MTU fallback, fragmented notifications, serialized chunk writes, permission denial, disconnect races, and explicit USB fallback
- [x] 3.3 Bound host event, write, notification, and outbound-retention queues and return an explicit capacity outcome without losing accepted work
- [x] 3.4 Verify retained packets remain ordered and single-release across bearer reconnect, channel detachment, channel replacement, and Android Activity recreation
- [x] 3.5 Confirm unknown Bluetooth advertisements never auto-connect, approval survives detachment, and only the approved peripheral reconnects

## 4. Integration Corpus And Deployment
<!-- specs: mobile-platform-foundation/spec -->

- [x] 4.1 Validate corpus schema, launch profiles, evidence classes, deadlines, cleanup, and artifact policy
- [x] 4.2 Run local hub and deployment configuration checks without embedding local credentials or addresses
- [x] 4.3 Run runner unit tests and the available cross-platform simulator or emulator scenarios
- [x] 4.4 Preserve partial outcomes where required hardware or reply correlation is unavailable

## 5. Platform Verification
<!-- specs: mobile-platform-foundation/spec -->

- [x] 5.1 Run Android unit tests, lint, assembly, emulator install, cold launch, and fatal-log checks
- [x] 5.2 Run iOS simulator tests, signed device build, install, and cold launch
- [x] 5.3 Verify physical iOS automatically reconnects and configures the approved RNode
- [x] 5.4 Run physical Android Bluetooth and USB checks when hardware is available, or retain the explicit evidence gap
- [x] 5.5 Run formatting, `git diff --check`, documentation lint, and applicable workspace validation
- [ ] 5.6 Run complete physical iOS BLE acceptance with NUS properties, write limit, fragmented traffic, bidirectional correlation, interruption, retained replay, and clean shutdown
- [ ] 5.7 Authorize the connected Android phone for ADB and run physical BLE acceptance with approval, NUS properties, MTU, fragmented traffic, bidirectional correlation, Activity recreation, and reconnect
- [ ] 5.8 Run physical Android USB acceptance for permission denial and grant, explicit fallback, Bluetooth non-preemption, bidirectional KISS traffic, detach, retained replay, and reconnect
- [ ] 5.9 Commit sanitized physical evidence summaries without device credentials, generated packages, native libraries, or raw runtime logs
