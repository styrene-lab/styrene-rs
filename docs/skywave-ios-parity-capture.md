# Skywave iOS Parity Capture

## Scope

This lane records operator-visible Skywave workflows on a paired physical
iPhone. It can establish an application workflow floor after provenance and
artifact review. It does not establish protocol interoperability, receipt
authenticity, Styrene package behavior, VoiceOver behavior, or background
execution guarantees.

The current candidate is `co.horsfalldesign.skywave`, version `1.0`, build `9`.
Do not carry Reticulum `0.9.4` or any build `5` observation forward to build `9`
without new evidence.

## Safety Boundary

- Keep the phone unlocked and awake while capturing.
- Do not uninstall, re-pair, signal the app, alter location, or apply device
  conditions.
- Use a test identity and non-sensitive message content where possible.
- Keep raw output under `target/mobile-integration/skywave-ios/`.
- Review screenshots, semantic snapshots, and logs for identifiers, addresses,
  destinations, and message content before publishing them.
- A screenshot or displayed `Delivered` label does not prove an LXMF receipt.

## Prepare

Discover the current CoreDevice identifier and hardware UDID at run time. Never
write either identifier into tracked files.

```sh
xcrun devicectl list devices
pymobiledevice3 remote browse
export SKYWAVE_COREDEVICE_ID='<CoreDevice identifier>'
export SKYWAVE_DEVICE_UDID='<hardware UDID>'
./scripts/capture-skywave-ios-parity.sh prepare
```

The command records raw CoreDevice metadata plus a sanitized `manifest.json`.
Confirm that the manifest says version `1.0`, build `9`, physical iOS, Developer
Mode enabled, and native RemoteXPC connectivity before continuing.

## Manual Milestone Captures

Open Skywave and navigate to the intended state before each screenshot. Use
stable labels such as `identity`, `tcp-settings`, `discovery`,
`conversation-list`, `draft-preserved`, `direct-send`, `receipt-detail`,
`retry`, `restart-restored`, `propagation`, and `degraded-state`.

The screenshot service captures the device screen, not a named process. Inspect
every PNG and reject it if Skywave is not visibly foregrounded; a running
Skywave PID is only a transport precondition.

```sh
export SKYWAVE_CAPTURE_RUN_ID='<one UTC run ID for the journey>'
./scripts/capture-skywave-ios-parity.sh snapshot identity
SKYWAVE_LOG_SECONDS=30 ./scripts/capture-skywave-ios-parity.sh logs identity
```

The log command is bounded and filters DVT OSLog by the running Skywave PID. It
does not capture network packets or prove that traffic crossed an RNS bearer.

## XCUITest Smoke Capture

The independent UI runner in `.local/styrene-ui/tests/xcui` contains the opt-in
`testSkywaveParitySmokeCapture`. It launches the installed bundle, retains a
screenshot and semantic accessibility snapshot, backgrounds and reactivates the
app, and verifies that it returns to the foreground. It does not tap controls or
change Skywave data.

Build the signed physical runner using the repository's existing signing setup,
then execute only the Skywave test with `SKYWAVE_PARITY_CAPTURE=1`. Retain the
`.xcresult` under the same ignored run directory. Do not invoke the Styrene
fixture tests against Skywave.

## Admission Checklist

For each proposed workflow floor, retain:

- exact application version, build, bundle identifier, platform, and OS;
- distribution provenance or an explicit unresolved-provenance finding;
- capture date, device class, tool versions, and connection path;
- journey ID, preconditions, actions, deadlines, and terminal observation;
- reviewed screenshot, semantic snapshot, bounded logs, test result, and their
  SHA-256 digests where applicable;
- explicit unexecuted steps, sensitive-data redactions, and cleanup outcome;
- an explanation of what the evidence cannot prove.

Only reviewed, immutable artifacts may be cited from the committed corpus. Keep
the build `9` reference classified as a candidate and parity rows `unevidenced`
until those citations resolve and corpus validation accepts the observation.
