# Skywave iOS Build 9 Capture Summary

## Evidence Boundary

Skywave `1.0` build `9` was executed on a physical iPhone running iOS `26.6.1`
on 2026-08-31. CoreDevice identified bundle
`co.horsfalldesign.skywave`. The XCUITest runner launched the installed app,
verified foreground-background-foreground recovery, and retained screenshots
and semantic accessibility snapshots.

Raw evidence remains ignored under `target/mobile-integration/skywave-ios/`.
This summary records reviewed observations and artifact digests. It does not
admit a workflow floor because the beta distribution provenance and immutable
artifact publication remain unresolved. Accessibility snapshots are not
VoiceOver evidence.

## Observations

- The app reports Skywave `1.0` build `9` and Reticulum `1.4.2`. No LXMF version
  or exact dependency revision was exposed.
- The stable top-level destinations are Overview, Messages, Calls, Map, and
  Mesh. Settings is reachable from each captured top-level destination.
- Overview distinguishes `MESH UP`, interface count, node count, relay count,
  unread traffic, missed calls, mail waiting, path, and synchronization state.
  During capture it showed one of three interfaces up, zero nodes, zero relays,
  no path, and mail not synchronized.
- Messages exposed an empty conversation state, search, and a new-message entry
  point. New Message supports peer search, direct 32-character LXMF address,
  clipboard paste, QR scanning, showing the local code, and multi-peer group
  selection.
- Identity exposed the display name, LXMF address, rename, copy, QR, encrypted
  backup, restore, and optional iCloud Keychain synchronization. The capture did
  not execute or validate any key operation.
- Interfaces separated Local Network, Bluetooth, notifications, and location
  permission explanations. It also exposed identity announce timing and manual
  announce controls. No permission or announce control was changed.
- Mail Sync kept propagation-node selection separate from interfaces. With no
  node selected, it showed `NO NODE`, `NOT SYNCED`, no background wake, and a
  disabled Sync Now action. Its disclosure says manual checks cost airtime and
  iOS background collection follows system scheduling.
- Mesh showed a listening state with no nodes heard and named Wi-Fi, Bluetooth,
  LoRa, UHF, and Internet as bearer categories.
- Map explicitly showed location sharing off. No location or sharing control
  was invoked.

## Parity Mapping

| Journey | Captured scope | Current result |
|---|---|---|
| `mobile.journey.identity` | Identity presentation, address, backup/restore affordances, iCloud option | Partial observation; custody and persistence not executed |
| `mobile.journey.tcp-setup` | Interface summary and configured Internet endpoint presentation | Partial observation; editing, validation, and reconnect not executed |
| `mobile.journey.discovery` | Zero-node Overview, Mesh listening state, direct-address and QR entry | Empty-state observation only |
| `mobile.journey.conversations` | Empty Messages state and new-message entry | Empty-state observation only |
| `mobile.journey.drafts` | No composer draft was created | Unevidenced |
| `mobile.journey.direct-send` | Recipient-selection entry only | Unevidenced |
| `mobile.journey.receipts` | No message or receipt was produced | Unevidenced |
| `mobile.journey.retry` | No failed message was produced | Unevidenced |
| `mobile.journey.restart` | Foreground-background-foreground recovery passed | Process restart and durable restoration remain unevidenced |
| `mobile.journey.propagation` | Node selection, disabled sync, status, and scheduling disclosure | Partial observation; selection and synchronization not executed |
| `mobile.journey.degraded-state` | Layered mesh, interface, node, path, and sync states | Partial observation; controlled failures not injected |

No parity row is promoted by this summary.

## Reviewed Artifacts

| Artifact | SHA-256 |
|---|---|
| Inventory Xcode log | `c3a2038a29ee414cb8631694993cba8bacc43b608ed68204dec0634036a26eed` |
| Overview screenshot | `7507252a6475f95a0a2adb68407e4a28d166497364deb663b1c951c6e851d000` |
| Overview semantic snapshot | `50eb5471db161bf6ccb81cf41b8afaef5cd0c4e5c7a001c5fc33c237c88e02e5` |
| Messages screenshot | `f99c4639a481970dc3f6e674a2725fb5c87f90aa26243ba11bbfe89e85d90e9a` |
| Messages semantic snapshot | `6984094d816d96c6d4f64dd2bc6c4b9e5976b10584e12c04158c9ffb445c9abe` |
| Calls screenshot | `36f81e2ed3a6ced616d28cc67e16de16fc1c6d21558a8fa1f93e8e74e40e9783` |
| Map screenshot | `f240d436b4958950a0b83d8cafa3661597d3e64f94adf920a35c8f131b39885c` |
| Mesh screenshot | `36d7e569260e7b789c3a08daedade815fb014d910b6c016fe898658e13f7d3a7` |
| Settings summary screenshot | `277cb94c1be874285c68d2c17a1b3bc6db6bc7f473f22be4cfa75596fc7c6e3b` |
| Corrected workflow Xcode log | `5f69556e4d2d51b977b6a3accede642a7810e0e34863f5920b3cf6ccd0aea70a` |
| Identity screenshot | `de31a2e7a220d189e379ca3b172fba26ff5da67a92f0abf1a73e5f03bfb7a2f1` |
| Identity semantic snapshot | `db001fad7c4d2ec7766fce7f9bc4c206d7e2b0e46f0461743df2895410b2f2bf` |
| Interfaces screenshot | `f38be86254de6119d182c5779f9013752849782720178c2bc032b5d8cbba1d1a` |
| Interfaces semantic snapshot | `37c182c706da754c5fd9aa4ab095a04759f8ff31eaa72fac296f81aec173290d` |
| Mail Sync screenshot | `c69a84e991541468d70deef3abce6fb7167ce77306b4d107fec13a671e642380` |
| Mail Sync semantic snapshot | `c1b396c363229c1c1215f8c92ca7874a355cf46244d3527327d5f58e29f57593` |
| New Message screenshot | `d6b6a7d4fd6c6f31ea5d43a930fabad5dba8c83aa027e2c7a08b33bf46011aa7` |
| New Message semantic snapshot | `da101659420cecdacf44250c60e5e9a2dc57116c1a8a4164babe06d71523ad11` |

The earlier smoke screenshot that captured another foreground application and
the workflow-3 settings screenshots taken before destination animation settled
are rejected artifacts and are not listed above.
