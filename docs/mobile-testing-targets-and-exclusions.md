# Mobile Testing Targets And Exclusions

## Purpose

This note defines the target inventory and claim boundary for the mobile testing
corpus. It prevents an unavailable or unexercised target from becoming an
implicit release claim. It also preserves excluded targets as future corpus
lanes instead of treating them as unsupported behavior.

The authoritative case inventory remains
`tests/fixtures/mobile-integration-v1/corpus.json`. The application workflow
ledger remains
`tests/fixtures/mobile-application-parity-v1/corpus.json`. This note does not add
execution evidence or change a corpus row status.

## P0 Claim Boundary

The P0 release-evidence target includes these areas:

- physical iOS Keychain and Android Keystore custody
- packaged iOS and Android installation, cold launch, fatal-log inspection,
  restart, forced termination, and upgrade preservation
- offline-ready, TCP lifecycle, discovery, conversation, draft, direct text,
  receipt, retry, restart, propagation-client, and degraded-state journeys
- Android USB RNode configuration, interruption, replay, and bidirectional RF
  correlation on the recorded hardware and firmware
- a provenance-locked Sideband Android run as application workflow evidence
- a provenance-locked Skywave beta run on physical iOS as application workflow evidence

The P0 target does not establish a general mobile, RNode, radio, accessibility,
notification, or application-parity claim. Each claim remains limited to the
target and scenarios that produced retained evidence.

## Available Target Inventory

Local identifiers, device identifiers, network addresses, credentials, and
signing values are runtime configuration. They must not enter this note or a
committed corpus fixture.

| Lane | Current resource class | P0 role | Evidence limit |
|---|---|---|---|
| Host runtime | macOS development host | Build, orchestration, local hub, bounded logs, and local USB RNode control | Does not prove packaged mobile behavior or RF reception |
| iOS Simulator | Current iOS Simulator | Deterministic fixture replay, layout, navigation, and basic package integration | Does not prove Keychain custody, physical lifecycle, Bluetooth, notifications, or VoiceOver |
| Physical iOS | Paired physical iPhone | Keychain custody, signed package launch, restart, upgrade, TCP lifecycle, and applicable P0 journeys | Does not prove BLE or RF without a separately recorded physical bearer run |
| Android emulator | Current arm64 Android emulator | Deterministic fixture replay, Android package automation, process control, and basic TCP integration | Does not prove hardware Keystore properties, USB, Bluetooth, notifications, TalkBack, or RF |
| Physical Android | Android 15 / API 35 tablet | Keystore custody, signed package launch, process death, upgrade, TCP lifecycle, Sideband execution, and USB RNode hosting | Does not prove unexercised Android versions, chipsets, BLE, or accessibility services |
| Android USB RNode | USB-attached RNode on the physical Android tablet | RNode detection, configuration readback, bounded writes, interruption, replay, and RF message correlation | Applies only to the recorded board, firmware, USB chipset, application revision, and radio profile |
| Host USB RNode | USB-attached RNode on the development host | Controlled peer, packet observation, and independent local radio endpoint | Does not prove a mobile USB or BLE implementation |
| Station radio peer | LAN-managed Station G2 RNode with an echo bot | Stable remote RF reception, echo reply, packet counters, and end-to-end correlation | Management-network reachability does not prove that a message crossed RF |
| Controlled propagation hub | Runner-owned local Styrene hub | Deterministic upload, offline retrieval, durable-before-ACK, repeat sync, and hub restart | Does not prove public Brutus availability or public-network interoperability |
| Reference application | SHA-256-pinned Sideband 2.1.0 Android APK | Executed application workflow floor on the physical Android device | Does not prove Styrene behavior, protocol authority, source reproducibility, or interoperability |
| Reference application | Skywave 1.0 build 9 iOS beta | Physical iOS workflow capture candidate | Does not become a workflow floor until distribution provenance and reviewed execution artifacts are admitted |

## P0 Exclusions

The following lanes are excluded from the P0 release-evidence target. An
exclusion means that the P0 release does not make the corresponding claim. It
does not mean that the behavior is unsupported or removed from future scope.

| Excluded lane or claim | P0 treatment | Future corpus requirement |
|---|---|---|
| iOS BLE RNode acceptance | Unevidenced; publish no iOS BLE claim | Record NUS properties, approval, pairing expiry, write limit, fragmentation, readback, interruption, replay, reconnect, packet correlation, and RF outcome |
| Android BLE RNode acceptance | Unevidenced; publish no Android BLE claim | Implement and verify the Android GATT adapter before running the same physical BLE matrix |
| CP210x and additional Android USB chipsets | Outside the tested USB claim | Add one lane per chipset with permission, open, byte transport, detach, reconnect, and bounded failure evidence |
| General RNode compatibility | Limit any claim to the tested board, firmware, bearer, and radio profile | Add explicit board and firmware classes; do not infer compatibility from KISS or NUS conformance alone |
| Public Brutus execution | Deferred when the controlled propagation hub satisfies the P0 scenario | Retain a separate public-network lane with endpoint provenance, deadlines, cleanup, and availability classification |
| Notification delivery and open routing | Excluded | Add authorization, foreground suppression, preview privacy, delivery, badge, open-route, denial, and revocation scenarios on physical devices |
| Background scheduling guarantees | Excluded; foreground opportunities remain best-effort | Add separate iOS task and Android service contracts before claiming scheduled execution |
| VoiceOver, TalkBack, and WCAG conformance | Excluded from P0 | Add physical screen-reader runs and a criterion-by-criterion WCAG 2.2 Level AA evidence matrix |
| Attachments and Paper delivery | Outside the mobile messaging minimum | Add focused product, persistence, transfer, failure, and packaged-target corpora before enabling either workflow |
| NomadNet and structured page workflows | Outside the mobile messaging minimum | Keep page execution and mobile presentation evidence separate from text messaging acceptance |
| Propagation hosting, peering, capacity, and expiry administration | Excluded; mobile remains a propagation client | Add operator and server acceptance lanes outside the mobile product corpus |
| Broad application parity | Blocked until every required row has executed floor and Styrene evidence | Preserve `matched`, `intentionally_different`, `deferred`, `unsupported`, and `unevidenced` outcomes per row |

## Radio Lane

Physical RF execution uses the recorded `US_915_DEVELOPMENT` test profile only
after the operator confirms that the profile is legal at the test location. The
current profile is `915 MHz`, `125 kHz`, `17 dBm`, spreading factor `7`, and
coding rate `5`.

Each RF run must identify its jurisdiction, hardware, software, and radio
configuration. The evidence must retain message and packet correlation,
transmit, remote receive, reply, counter, deadline, and duplicate-delivery
observations. A local KISS write, USB bulk transfer, or management-network
response is not RF reception evidence.

## Reference Application Lane

The Sideband lane must verify the APK SHA-256 before installation. The retained
record must identify the package version, build, Android device class, OS,
artifact hash, workflow, and runtime artifacts.

The APK hash can identify the executed binary for application workflow evidence.
It cannot resolve the bundled source revisions or promote Sideband to protocol
authority. Sideband execution cannot replace packaged Styrene or Python
interoperability evidence.

The Skywave lane uses the installed `co.horsfalldesign.skywave` beta without
extracting private application data. Run metadata, screenshots, semantic XCUITest
snapshots, test results, and bounded process logs belong under the ignored
`target/mobile-integration/skywave-ios/` directory. Raw CoreDevice metadata and
logs can contain device identifiers, destinations, message text, or network
details; review and redact them before publishing a summary or digest. An
installed version and build do not resolve TestFlight provenance or identify the
bundled RNS and LXMF revisions.

## Common Evidence Record

Every non-fixture run must retain these fields:

- corpus and case IDs
- UI, backend, runner, hub, and reference-application revisions or artifact hashes
- package identifier, package hash, platform, OS, runtime, and target class
- topology, endpoint class, bearer, RNode board, firmware, and radio profile when applicable
- action milestones, deadlines, correlation IDs, typed observations, and terminal outcome
- bounded redacted logs, semantic UI snapshots, packet counters, and artifact digests when applicable
- cleanup ownership and cleanup outcome
- explicit unexecuted scenarios and exclusions

Evidence artifacts belong under `target/mobile-integration/` or another ignored
run directory. Commits may contain validated summaries and digests. Commits must
not contain raw device logs, identifiers, credentials, signed packages, generated
native scaffolding, local addresses, or private application data.

## Claim Rule

A lane closes only when its required assertions pass on the recorded target and
all required artifacts are retained. A lower evidence class cannot close a
higher class. In particular:

- component evidence cannot close package execution
- simulator or emulator evidence cannot close a physical-device requirement
- USB or GATT connection cannot close RF transmission or remote reception
- an echo-bot reply cannot close RF evidence without correlated remote radio observations
- static reference-application inspection cannot close executed workflow evidence
- displayed delivery text cannot close authenticated receipt evidence

If a target is unavailable, classify the lane as `unevidenced` or `blocked` with
the reason. Do not mark the behavior as passing, failing, or unsupported without
the evidence required for that classification.
