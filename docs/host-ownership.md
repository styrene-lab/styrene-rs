# Host Ownership After Consolidation

The hardening and parity corpus is consolidated on `main` (see
`docs/remaining-workstream-delta.md`). The remaining work is split by the
host that can produce its evidence. Each host works on its own long-lived
branch, branched from the consolidated `main` on 2026-09-02, and lands
finished work on `main` through pull requests as before.

| Host | Branch | Owns |
|------|--------|------|
| macOS workstation | `host/macos-ios` | macOS and iOS work |
| Nucleus (Linux) | `host/linux-android` | Linux and Android work |

The same two branches exist in `styrene-ui`.

## Assignment rule

A task belongs to the host named in its text. A task that names both
platforms is split: each host records its own platform's evidence and ticks
the task only when both records exist. A task that names no platform is
shared and is claimed in its pull request before work starts.

## macOS workstation

- iOS App Lock: the physical iPhone matrix and the separate App Lock versus
  Keychain prompt observations (`styrene-ui/openspec/changes/ios-app-lock-policy`).
- Desktop network workflow polish on macOS: keyboard order and labels, fixture
  captures, and the Live-failure and Embedded smoke checks
  (`styrene-ui/openspec/changes/desktop-network-workflow-polish`).
- The iOS and macOS tasks of `complete-mobile-product-workflows`,
  `deliver-mobile-messaging-minimum`, and `shared-dioxus-mobile-ui`: iOS
  custody, App Lock, QR, lifecycle, accessibility, simulator, and packaged
  evidence.
- Serial RNode provisioning evidence gathered on macOS
  (`rnode-firmware-provisioning`).
- The macOS launcher packaging check for operator profiles.

## Nucleus

- Android BLE GATT implementation and every Android Bluetooth claim.
- The Android and Linux tasks of the three mobile changes: emulator,
  packaged, physical, and accessibility evidence on Android.
- Linux desktop workflow checks and the Linux launcher packaging check.
- Firmware executors and physical write and recovery evidence on Linux hosts.

## Shared

- `leviculum-rns-corpus-wave`, `repository-signing-profile` publication, and
  `extract-styrene-ui-repository` governance are host-neutral.

## Working agreement

- Rebase the host branch on `main` before opening a pull request, and keep
  pull requests scoped to one ledger group.
- Do not tick another host's task, and do not carry another host's evidence
  files.
- Pins between the repositories move only on `main`.
