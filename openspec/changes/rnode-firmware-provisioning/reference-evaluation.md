# Nucleus RAK4631 Reference Evaluation

## Reference

On 2026-08-31, the `nucleus` host contained an uncommitted RAK4631 flasher
variant in `/home/wilson/workspace/styrene-lab/styrene-rs`. The checkout was on
branch `linux-android` at base revision
`e499853b104786e8291681406726dafa4c03e8df`.

The uncommitted files are implementation evidence. They are not a merge source
or an immutable upstream revision. The reviewed file digests were:

| File | SHA-256 |
|---|---|
| `scripts/rak4631_rnode.py` | `bf3dc41a0e0be8976cbab3d480acb3e33046f69884cc67118c419c38b0b2b10d` |
| `product/flashers/rak4631-rnode-v1.toml` | `1b4b906b196f16e454991ae0ccfbaa6b9cf0ed5d8e0002dff95e7e0b04cbef8b` |
| `scripts/test_rak4631_rnode.py` | `1e2e22e2579d378577ea16861588ca245f08ec63414dd076f11b6c5c765e9e57` |
| Reference design | `df3bc4f7a4504f42dfdc565d5825968581282d027b86dda098d631591e0666f9` |

No stable USB serial number is retained in this evaluation.

## Observed Candidate

The retained external record and a read-only host inspection identify:

| Fact | Observation |
|---|---|
| Core | RAK4631 |
| Carrier | RAK19003 |
| Radio population | 915 MHz SX1262 population, operator recorded |
| Attached module | RAK1910 GPS, outside the RNode target |
| Normal USB identity | VID `239a`, PID `8029`, RAK4631 product string |
| Serial DFU identity | VID `239a`, PID `002a`, RAK4631 product string |
| UF2 identity | VID `239a`, PID `0029`, label `RAK4631` |
| USB device revision | `1.00` in both observed modes |
| DFU family | Adafruit-compatible nRF52 serial DFU |
| Exact bootloader revision | UF2 Bootloader `0.4.3`, dated 2023-05-20 |
| Bootloader board ID | `WisBlock-RAK4631-Board` |
| SoftDevice | `S140 6.1.1` |
| Exact hardware revision | Unknown |
| BLE DFU service and identity | Not observed |

The approved non-writing 1200-baud reset changed the device from normal USB
mode to serial DFU mode. Serial DFU exposed CDC interfaces only. It did not
expose an UF2 mass-storage volume or `INFO_UF2.TXT`.

A physical double-reset then exposed the read-only `RAK4631` UF2 volume. Its
`INFO_UF2.TXT` reported bootloader `0.4.3`, Board-ID
`WisBlock-RAK4631-Board`, build date 2023-05-20, and SoftDevice `S140 6.1.1`.
No firmware file was copied to the volume. A final physical reset returned the
board to normal USB mode as `239a:8029`.

## Artifact Evidence

The external artifact directory retained a canonical RNode Firmware 1.86 build:

| Field | Value |
|---|---|
| Source | `https://github.com/markqvist/RNode_Firmware.git` |
| Revision | `d39339f8ecd5145b248c18bac7b6ea0f82faf85a` |
| Build target | `release-rak4631` |
| DFU ZIP SHA-256 | `3a61632282b7668b6c937646a891a604e6db236ed35473831a7e1575680600d7` |
| ZIP size | 243109 bytes |
| Application size | 242220 bytes |
| DFU device type | `0x0052` |
| SoftDevice requirement | `0xfffe` |

The ZIP integrity check passed. Its manifest contains one application binary and
one init packet. It contains no bootloader or SoftDevice image. This makes the
archive a useful candidate for application-only admission. It is not admitted
by Styrene because no Styrene-signed manifest or complete compliance bundle was
present.

## Validation

The following external checks passed without flashing or configuring a radio:

- four declarative flasher contracts.
- seven flasher-validator tests.
- seven RAK4631 helper tests.
- candidate ZIP integrity, structure, device type, and SHA-256 validation.

These checks prove only the reference implementation's offline behavior. They
do not prove BLE DFU behavior, serial delivery, provisioning, recovery, or
post-write verification.

## Decisions

| Reference behavior | Decision | Rationale |
|---|---|---|
| Pinned RNode 1.86 source and `release-rak4631` target | Adopt | Matches the canonical source already pinned by this change. |
| Candidate application ZIP and digest | Investigate | Structure and digest pass, but signed admission and GPL release records are absent. |
| RAK4631, RAK19003, and 915 MHz physical observations | Investigate | Useful candidate facts, but hardware revision and retained photographic evidence are absent. |
| Separate normal and serial-DFU USB identities | Adopt as evidence | The same attached candidate was observed in both modes without a firmware write. |
| Exact bootloader observation | Adopt as evidence | Read-only UF2 metadata identifies bootloader `0.4.3` and SoftDevice `S140 6.1.1`. |
| BLE application upgrade claim | Defer | The reference exercises USB serial DFU only. It contains no BLE DFU service evidence. |
| `delivery-approved` status | Skip | The retained provenance says delivery is blocked, and physical flash verification is incomplete. |
| Committed stable USB serial | Skip | Stable device identifiers are prohibited in retained Styrene evidence. |
| Hash-only ZIP admission | Skip | Styrene requires an authenticated, bounded manifest before device access. |
| Python flashing helper | Skip | It accepts runtime paths and tool commands outside the exact immutable-plan executor boundary. |
| Fresh provisioning sequence | Investigate | It sets product, model, and revision state, but no physical run proves identity, signature, or final hashes. |

## Remaining Gates

Task 4.1 is complete for the recorded RAK4631 board and bootloader. Before mobile
acceptance, retain the exact hardware revision, BLE DFU service, identity
transition, MTU, packet receipt notification, interruption, recovery, and
post-write RNode observations.

Before desktop delivery, produce a Styrene-signed manifest and bounded executor
for the exact target. Test application-only delivery, provisioning, interruption,
explicit recovery, and final model, version, and running-hash verification.
