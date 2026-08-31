# RNode Firmware Provisioning Design

## Boundary

The product needs one firmware policy and multiple constrained executors. The
shared Rust layer owns target facts, catalog admission, plan construction,
operation state, authorization, and verification. Renderer code owns no firmware
selection or destructive policy.

The desktop host can use full-machine USB, serial, process, and recovery
capabilities. The iOS host can use CoreBluetooth only. Mobile support is limited
to nRF52 application upgrades through physically accepted BLE DFU bootloaders.
Normal RNode NUS traffic and bootloader DFU are separate sessions.

`docs/hardware-evidence-boundary.md` assigns generic flashing and hardware
lifecycle tooling to Nex. RNode provisioning is a narrow product exception.
Styrene owns exact target and artifact admission, immutable plans, product
confirmation, RNode metadata semantics, recovery presentation, and verified
product success. Nex owns reusable discovery, reset, programmer, byte-delivery,
and low-level read backends.

Until a versioned Nex provisioning API exists, Styrene can retain an exact
bounded RNode executor adapter. It must accept only an admitted immutable plan.
It must not accept arbitrary commands, device paths, images, offsets, erase
ranges, reset sequences, or programmer options. Renderer code remains outside
the policy and execution boundary.

## Operation Model

The contract uses four operations:

- `upgrade` replaces an application on an already identified and configured RNode.
- `fresh_install` installs an application on compatible bootstrapped hardware.
- `provision` writes required RNode product, model, radio, identity, signature,
  console, or target-hash state.
- `recovery` repairs a failed or absent application, bootloader, partition, or
  other declared prerequisite.

An operation plan is immutable. It binds target evidence, artifact digests,
image regions, executor, destructive boundary, preservation policy, recovery
path, and expected post-write observations. Confirmation binds to the plan
digest and current target generation.

## Firmware Trust

Canonical RNode release metadata supplies useful SHA-256 values but is not a
complete signed update chain. Styrene must verify a Styrene-controlled signed
manifest before device access. The manifest admits exact upstream revisions,
artifacts, archive members, image regions, application hashes, targets, and
executors.

Archive processing is bounded and rejects path traversal, duplicate names,
unexpected files, oversized expansion, and overlapping or protected regions.
Firmware distribution must satisfy `docs/rnode-firmware-distribution.md`. Each
published binary requires matching corresponding source, notices, immutable
digests, and retention records. Artifact admission does not establish license
compliance.

## Executor Matrix

The planned executor classes are:

| Executor | Host | Initial operation scope |
|---|---|---|
| `host_serial_esp` | Desktop | ESP32 inspection, upgrade, fresh install, provisioning, recovery |
| `host_serial_avr` | Desktop | AVR inspection, upgrade, fresh install, provisioning, recovery |
| `host_serial_nrf_dfu` | Desktop | nRF52 inspection, application install or upgrade, recovery where supported |
| `ios_nrf_ble_dfu` | iOS | Accepted configured nRF52 application upgrades only |

The current attached device reports Espressif USB JTAG/serial with VID/PID
`303a:1001`. This proves an ESP-family USB interface only. The planner must not
select a board or radio artifact until exact device metadata is captured.

A second candidate on the `nucleus` host identifies a RAK4631 core on a RAK19003
carrier from the 915 MHz population. Normal USB mode reports `239a:8029`. A
non-writing 1200-baud reset reports serial DFU mode as `239a:002a`.

A physical double-reset exposes UF2 mode as `239a:0029`. Read-only
`INFO_UF2.TXT` reports UF2 Bootloader `0.4.3`, Board-ID
`WisBlock-RAK4631-Board`, build date 2023-05-20, and SoftDevice `S140 6.1.1`.
A final reset returns the device to normal mode. No firmware write occurred.

The candidate RNode 1.86 application ZIP has SHA-256
`3a61632282b7668b6c937646a891a604e6db236ed35473831a7e1575680600d7`.
The exact hardware revision remains unknown, and BLE DFU was not observed. See
`reference-evaluation.md`. These facts do not enable a target or mobile support
claim.

## State And Failure

The operation state machine separates non-destructive and destructive phases:

```text
inspect -> planned -> confirmed -> preparing -> writing -> restarting
        -> verifying -> succeeded
```

Before `writing`, cancellation can end without recovery. After a target erase or
write starts, interruption produces a failed state with an explicit recovery
requirement. A process exit, BLE disconnect, stale generation, or completed byte
transfer cannot produce success. Only authoritative post-write verification can
produce success.

## Corpus-First Gate

Three corpuses precede implementation:

- Capability cases define which host, target, executor, and operation
  combinations are allowed, denied, experimental, or unverified.
- Artifact cases define manifest, digest, archive, layout, and target-admission
  outcomes.
- Workflow cases define confirmation, cancellation, interruption, stale-event,
  recovery, and post-write outcomes.

The corpus validator checks schema, identifiers, decisions, reason codes,
digests, platform restrictions, destructive-phase recovery, and success
verification. Product tests will consume the same fixtures. Synthetic cases
prove contract behavior only. Physical evidence is required for support claims.

## Rollout

1. Commit and validate the OpenSpec change and all three corpuses.
2. Add failing Rust contract tests that consume the corpuses.
3. Implement read-only target observation and immutable planning.
4. Add one exact desktop ESP executor and physical acceptance.
5. Add one exact RAK4631 BLE DFU executor and physical acceptance.
6. Investigate fresh BLE installation separately without enabling a claim.
7. Expand the catalog only through new corpus and physical evidence rows.
