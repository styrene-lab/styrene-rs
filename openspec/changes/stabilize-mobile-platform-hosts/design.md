# Stabilize Mobile Platform Hosts Design

## Role In The Migration

The current SwiftUI and Compose applications remain temporary reference hosts.
They prove embedded runtime and native hardware behavior before Dioxus replaces
their presentation layers. This change does not make either native UI an
architectural authority.

## Commit Boundaries

The work is separated by intent:

1. Embedded runtime and UniFFI lifecycle fixes.
2. Android host, Bluetooth bearer, USB fallback, and retention behavior.
3. iOS host, CoreBluetooth bearer, startup restoration, and tests.
4. Mobile integration corpus, runner, hub fixtures, and deployment support.
5. Documentation, CI, and generated-artifact exclusions.

Each commit must preserve the behavior of earlier commits. Generated UniFFI
bindings, XCFrameworks, JNI libraries, APKs, derived data, and runtime evidence
must not enter a commit.

## RNode Boundary

Rust owns packet-channel semantics and the RNode KISS protocol. Swift and Kotlin
own CoreBluetooth, Android Bluetooth, Android USB permission, and host lifecycle
integration. Only the explicitly approved Bluetooth peripheral reconnects.

Outbound retention remains host-independent in behavior. A bearer coordinator
ensures one active bearer. Android USB activation requires an explicit action.

## Startup And Shutdown

Normal launch loads validated persisted configuration and starts one embedded
node. Bluetooth discovery and approval can operate before a packet channel is
available. Channel detachment pauses packet pumping without clearing peripheral
approval or causing an unrelated Bluetooth disconnect.

Partial boot failure and explicit shutdown release workers, packet channels,
interfaces, and temporary storage owned by the attempt. Repeated lifecycle calls
must not create concurrent embedded nodes.

## Evidence Policy

Automated tests prove protocol framing, state transitions, retention, lifecycle,
and launch-profile behavior. Simulator and emulator smoke tests prove packaging
and host startup. Physical evidence is reported only for the device and bearer
that were exercised.

An operator-observed iOS Bluetooth run reached reconnect, configuration, and
packet transmission. It lacks the complete committed acceptance record required
for physical acceptance. Physical iOS, Android Bluetooth, and Android USB
acceptance remain open. An Android phone is connected by USB, but ADB must be
authorized on the phone before it can provide physical execution evidence.

## RNode Evidence Closure

The host link layers must be injectable before physical acceptance. Fake iOS
CoreBluetooth and Android GATT or USB adapters must drive service discovery,
characteristic validation, protected-access failure, negotiated write limits,
fragmented notifications, serialized writes, disconnects, and retries without
physical timing dependencies.

Every host queue must have a declared bound. A full queue returns an explicit
capacity outcome. A failed or interrupted write retains accepted outbound work
until the same packet channel reconnects. Channel detachment pauses packet
pumping and preserves Bluetooth approval. Replacing the packet channel does not
replay work owned by the previous channel.

A physical acceptance record identifies the mobile device and OS, RNode board
and firmware, advertised name, NUS characteristic properties, negotiated MTU or
write limit, radio profile, jurisdiction, packet hashes or correlation IDs,
bidirectional packet counts, deadlines, disconnect and reconnect result, and
terminal outcome. Raw logs, generated applications, bindings, and native
libraries remain outside source control. Only a sanitized evidence summary can
be committed.

The temporary SwiftUI and Compose hosts establish hardware-boundary evidence.
An RNode product claim also requires the Dioxus release candidate to repeat the
applicable physical scenario through the same Rust-owned packet-channel and
bearer-state contracts.

## Dependency And Rollout

This change lands before frontend API extraction. Later changes may replace the
native presentation layers, but they must preserve these platform and hardware
behaviors. The native applications remain available as comparison targets until
the Dioxus migration reaches physical parity.
