# Stabilize Mobile Platform Hosts Tasks

## 1. Change Inventory And Commit Boundaries
<!-- specs: mobile-platform-foundation -->

- [x] 1.1 Inventory committed and uncommitted mobile changes against `origin/main`
- [x] 1.2 Separate runtime, Android, iOS, corpus/deployment, and documentation changes by intent
- [x] 1.3 Confirm every generated binding, native library, application package, and runtime artifact is excluded
- [x] 1.4 Record physical-device evidence and explicit evidence gaps in the mobile documentation

## 2. Embedded Lifecycle
<!-- specs: mobile-platform-foundation -->

- [x] 2.1 Add or retain tests for one-node startup, partial-boot cleanup, explicit shutdown, and repeated lifecycle calls
- [x] 2.2 Verify persisted configuration starts the iOS and Android embedded node without a normal-path manual start action
- [ ] 2.3 Verify channel detachment pauses packet pumping without destroying Bluetooth approval
- [x] 2.4 Run focused `styrened` mobile and mobile FFI tests with warning-denied Clippy

## 3. RNode Bearers
<!-- specs: mobile-platform-foundation -->

- [ ] 3.1 Verify NUS discovery, protected access, fragmented KISS input, and serialized output on iOS
- [ ] 3.2 Verify Android Bluetooth ownership, lifecycle recovery, and explicit USB fallback in unit tests
- [ ] 3.3 Verify bounded outbound retention across bearer reconnect and Android Activity recreation
- [ ] 3.4 Confirm unknown Bluetooth advertisements never auto-connect and only the approved peripheral reconnects

## 4. Integration Corpus And Deployment
<!-- specs: mobile-platform-foundation -->

- [x] 4.1 Validate corpus schema, launch profiles, evidence classes, deadlines, cleanup, and artifact policy
- [x] 4.2 Run local hub and deployment configuration checks without embedding local credentials or addresses
- [x] 4.3 Run runner unit tests and the available cross-platform simulator or emulator scenarios
- [x] 4.4 Preserve partial outcomes where required hardware or reply correlation is unavailable

## 5. Platform Verification
<!-- specs: mobile-platform-foundation -->

- [x] 5.1 Run Android unit tests, lint, assembly, emulator install, cold launch, and fatal-log checks
- [x] 5.2 Run iOS simulator tests, signed device build, install, and cold launch
- [x] 5.3 Verify physical iOS automatically reconnects and configures the approved RNode
- [x] 5.4 Run physical Android Bluetooth and USB checks when hardware is available, or retain the explicit evidence gap
- [x] 5.5 Run formatting, `git diff --check`, documentation lint, and applicable workspace validation
