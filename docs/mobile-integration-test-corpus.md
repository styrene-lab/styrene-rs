# Mobile Integration Test Corpus

## Purpose

The mobile integration corpus connects the shared mobile UI contract to daemon,
UniFFI, native-host, simulator, device, radio, and interoperability evidence. It
is both an acceptance inventory and an implementation backlog.

The source corpus is
`tests/fixtures/mobile-integration-v1/corpus.json`. Generated results belong
under `target/mobile-integration/` and are not committed.

## Evidence Rules

A passing UI fixture proves layout and interaction behavior only. It does not
prove message delivery, route selection, bearer use, persistence, security, or
interoperability.

Every P0 case requires at least one assertion from an authoritative source:

- daemon state or durable storage.
- a typed UniFFI projection.
- a platform service, such as Keychain or USB permission state.
- accessibility semantics.
- pinned upstream interoperability evidence.

Screenshots can supplement these assertions. They cannot replace them.

The corpus uses three maturity values:

- `executable`: all required seams exist for the declared execution lane.
- `partial`: a lower layer can prove part of the behavior, but the native host
  lacks a projection or test seam.
- `blocked`: the owning API or platform integration does not exist.

A blocked case is expected backlog. It must name each missing capability.

## Execution Lanes

### Offline Corpus Validation

`just test-mobile-corpus` parses the committed corpus. It validates structural
schema, repository references, required area and case coverage, evidence-scope
rules, and blocked-capability declarations. It does not validate deadlines,
cleanup execution, artifact redaction, or whether referenced tests ran. It does
not open sockets, launch processes, or start simulators.

This lane is safe for ordinary validation and CI.

### Rust Mobile Runtime

`cargo test -p styrened --test mobile_node` runs listener-backed mobile runtime
tests. It covers boot profiles, direct TCP, peer discovery, bidirectional LXMF,
conversation state, persistence, and shutdown.

This lane opens loopback listeners. It remains in the explicit network test
gate.

### UniFFI Contract

The FFI lane verifies exported records, lifecycle behavior, operation errors,
and generated-language compatibility. It must use the same corpus case IDs as
the Rust and native-host lanes.

The first expansion should cover complete conversation and message records,
delivery lifecycle, requested and actual method, route evidence, attachments,
capabilities, propagation, pages, and diagnostics.

### Native Host Tests

The iOS lane requires an XCTest and XCUITest target. The Android lane requires
JVM state tests and Compose instrumentation tests. Both hosts need deterministic
launch profiles, fixture injection, stable accessibility identifiers, and a
controllable clock.

Native tests assert:

- state and action parity.
- identity, conversation, and composer navigation.
- disabled reasons for unavailable capabilities.
- generation-safe lifecycle behavior.
- persistence owned by the host.
- accessibility semantics and large-text behavior.

### Dual-Simulator Integration

The first simulator topology uses two iOS simulators and a local `styrened` hub.
The next topology uses one simulator as a TCP listener and one as a TCP client.
Each app instance receives a deterministic launch profile and separate identity.

The runner must retain:

- corpus case ID and correlation ID.
- source revision and app artifact hash.
- simulator runtime and device IDs.
- ordered milestones and assertions.
- bounded, redacted logs.
- semantic UI snapshots for both nodes.
- cleanup evidence.

The iOS host accepts an isolated integration profile through these launch
arguments:

```text
--styrene-integration-profile ios-a
--styrene-hub-address 127.0.0.1:4242
--styrene-display-name "iOS A"
--styrene-reset-state
```

The profile ID may contain ASCII letters, digits, `.`, `_`, and `-`. The first
character must be alphanumeric. Each profile stores config, identity, and data
under its own `Application Support/Styrene/Integration/<profile>/` directory.
The reset flag removes only that profile directory before boot. Omit the flag
when a case must preserve identity or messages across relaunches.

Identity fixtures and deterministic time remain runner requirements.

### Local Android Emulator

The local cross-platform lane uses the Android CLI `medium_phone` profile. The
CLI selects an ARM64 Google system image that matches the packaged native
library. Set `ANDROID_HOME` to the SDK root and use these commands:

```sh
just android-emulator-setup
just android-emulator-start
just android-emulator-install
just android-emulator-integration-start
just android-emulator-ui
```

The setup command is idempotent. The start command waits until Android is ready.
The install command expects the current debug APK and ARM64 native library to
exist. Build them with `just android-deploy` before installation.

`android-emulator-integration-start` launches profile `android-a` against
`10.0.2.2:4242` and resets that profile by default. Override its `profile`,
`display_name`, or `reset` parameters when a case needs separate or persistent
state. Integration state lives under the app's private
`files/integration/<profile>/` directory. The production launch path remains
unchanged.

The machine-local AVD, system image, NDK, generated bindings, native library,
and APK are not committed.

### Local Simulator Hub

The simulator hub uses the checked-in `deploy/hub.toml` profile and isolated
state under `target/mobile-integration/hub/`.

```sh
just mobile-hub-start
just mobile-hub-status
just mobile-hub-android-probe
just mobile-hub-logs
just mobile-hub-stop
```

The start command builds `styrened` and binds the mesh transport to
`0.0.0.0:4242`. It keeps plaintext RPC on `127.0.0.1:4243`. Readiness requires
the following evidence:

- the daemon process remains alive.
- RPC readiness responds on loopback.
- TCP port 4242 accepts a host connection.
- startup logs confirm `role = "hub"`.
- startup logs confirm propagation-store activation.

Use `127.0.0.1:4242` from iOS Simulator and `10.0.2.2:4242` from the Android
emulator. The status output includes the direct delivery destination and the
separate propagation control destination.

Binding `0.0.0.0` can expose port 4242 to LAN and VPN peers. Run this profile
only on a trusted development host. Stop it when integration work is complete.

### Cross-Platform Runner

With the Android emulator booted and the current debug APK built, run:

```sh
just mobile-integration-cross-platform
```

The runner starts or reuses the local hub, installs Android, creates unique
isolated profiles, and runs the `StyreneMobileIntegration` XCUITest scheme. It
uses semantic UI elements on both hosts. It does not use fixed tap coordinates.
Generated evidence can contain local paths, device identifiers, payload text,
and unredacted logs. Treat the ignored evidence directory as sensitive.

The current scenario automates the iOS-to-Android slice of
`mobile.messaging.cross-platform-roundtrip`. It proves:

- both host profiles connect through the local hub.
- iOS discovers the named Android identity.
- iOS queues a correlation-tagged payload.
- Android unread state increases above zero.
- Android displays the exact payload.
- opening the Android thread clears unread state to zero.
- both app processes stop after the run.

The reverse Android-to-iOS reply and native message-ID correlation remain open
before the full round-trip corpus case can pass.

Each run writes ignored evidence under
`target/mobile-integration/runs/<correlation-id>/`. The directory contains JSON
milestones, source identity, an Android artifact hash, and available logs. It also contains
Android semantic UI snapshots, the generated test manifest, and the iOS
`.xcresult` with its kept screenshot attachment. A failed run retains the same
evidence classes and exits nonzero. A pre-existing hub remains running. A hub
started by the runner stops during cleanup.

### Cross-Platform And Device Integration

Cross-platform scenarios pair iOS and Android through direct TCP or a local hub.
Device scenarios add Keychain, Android secure storage, background execution,
notifications, USB permission, and RNode hardware.

Simulator success does not satisfy device, radio, background, or secure-custody
claims.

### Live Interoperability

Pinned Python RNS, LXMF, and NomadNet runs use the supervised interoperability
runner. They remain separate from internal Rust and fixture evidence. A passing
Styrene-to-Styrene case cannot establish upstream parity.

## Topology Progression

1. `offline-single`: one embedded node without a routable interface.
2. `loopback-pair`: two embedded Rust mobile nodes over direct TCP.
3. `hub-pair`: two mobile nodes connected through a local hub.
4. `ios-dual-simulator`: two independently installed iOS app instances.
5. `cross-platform-simulator`: iOS Simulator and Android emulator through a
   local hub.
6. `android-rnode-device`: an Android device and physical RNode.
7. `wireguard-underlay`: ordinary TCP carried through a pre-created tunnel.
8. `propagation-hub`: direct path unavailable with queued delivery and polling.
9. `micron-host`: a controlled native page host with valid and invalid content.
10. `upstream-live`: pinned Python implementations under supervised execution.

## Release Gates

P0 release readiness requires:

- one daemon-authoritative assertion for every P0 case.
- one native-host execution for each shared P0 UI behavior.
- cross-platform parity for shared state and terminology.
- no blocked P0 row without an explicit product decision.
- no preview fixture used as protocol evidence.
- bounded cleanup and retained evidence for simulator and device runs.

P1 covers attachments, structured Micron sessions, device RNode behavior,
accessibility automation, and tunnel-backed bearer evidence. P2 covers scale,
soak, low-memory, low-storage, locale, clock, and repeated background wakeups.

## Filling A Mocked Capability

Use this sequence when implementing a mocked UI element:

1. Identify its corpus case IDs.
2. Add or extend the authoritative daemon projection.
3. Export a typed UniFFI record or operation.
4. Add focused Rust and FFI tests with the same case IDs in test output or
   metadata.
5. Wire both native hosts without parsing display strings.
6. Add semantic host assertions and failure coverage.
7. Change corpus maturity only after the declared evidence exists.
8. Retain previews only for states that cannot be produced deterministically.

## Initial Implementation Order

1. Complete message and conversation projections.
2. Delivery lifecycle, requested method, actual method, and fallback evidence.
3. Interface, route, hop, and bearer observations.
4. Capability availability and disabled reasons.
5. Attachment input, transfer, integrity, and cancellation.
6. Standard propagation snapshots and explicit synchronization.
7. Typed Micron sessions, rendering, links, forms, and downloads.
8. Bounded diagnostics and export metadata.
9. Native background, notification, and secure-custody integration.
10. Physical RNode and upstream interoperability gates.
