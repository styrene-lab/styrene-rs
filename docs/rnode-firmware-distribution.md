# RNode Firmware Distribution

## Scope

This document defines the release gate for RNode firmware that Styrene admits,
hosts, bundles, downloads for an operator, or writes to a device. It does not
change the license of upstream firmware. It is an engineering policy, not legal
advice.

The current reference is RNode Firmware 1.86:

| Field | Value |
|---|---|
| Upstream repository | `https://github.com/markqvist/RNode_Firmware.git` |
| Pinned revision | `d39339f8ecd5145b248c18bac7b6ea0f82faf85a` |
| Upstream release | `1.86` |
| Primary license | GNU General Public License, version 3 |
| Primary notice | Copyright 2024 Mark Qvist / unsigned.io |

The upstream `README.md` and `LICENSE` files are authoritative for this pinned
revision. The source tree also contains components with separate notices. These
include MIT-licensed radio-driver code credited to Sandeep Mistry, Mark Qvist,
and Jacob Eva. A release must inventory the files that enter each binary and
preserve every applicable copyright and license notice.

## Distribution Gate

Styrene currently commits no RNode firmware binary or source bundle. A signed
manifest admits bytes for product use, but it does not prove license compliance.
Do not publish or enable a firmware artifact until its release record satisfies
this document.

Treat each of these actions as firmware distribution for this gate:

- placing a firmware binary in a Styrene release.
- hosting a firmware binary or archive on Styrene infrastructure.
- downloading firmware through a Styrene service for delivery to an operator.
- shipping firmware preinstalled on a device.
- providing a modified binary to another person or organization.

For every distributed object-code artifact, publish the complete corresponding
source through the same release channel. Do not rely only on an upstream URL.
The source bundle must match the exact distributed bytes and include:

- the pinned upstream source revision.
- all Styrene or vendor patches, with modifications identified.
- board, radio, partition, bootloader, and build configuration.
- scripts and other source used to control compilation and installation.
- reproducible build instructions and required tool versions.
- the GPLv3 license text.
- all applicable third-party notices and license texts.
- the source bundle digest and its relationship to the binary manifest.

If a release uses a written source offer instead of accompanying source, obtain
legal review before publication. The offer must satisfy GPLv3 section 6,
including its duration and access requirements. Styrene release policy prefers
accompanying source and does not use a written offer by default.

## Installation Information

Styrene manifest signatures authenticate admitted artifacts for the host. They
must not prevent an operator from installing modified GPL-covered firmware.

GPLv3 can require Installation Information for a distributed User Product. In
that case, provide the methods, procedures, authorization keys, and other
information needed to install and run a modified version. Escalate the release
for legal review if secure boot, locked bootloaders, signing keys, fuses, or
device policy can restrict installation of modified firmware.

## Retention

Retain each immutable compliance bundle for as long as Styrene distributes or
supports the corresponding binary. Retain it for at least three years after the
last distribution. A bundle contains:

- the distributed binary and archive.
- the signed Styrene manifest and verifying-key identifier.
- the complete corresponding source archive.
- license and notice files.
- build inputs, instructions, logs, and digests.
- upstream and patch revisions.
- the release record and publication dates.

Do not delete a source bundle while its binary remains downloadable. If an
artifact is withdrawn, keep its compliance bundle and record the withdrawal.
Artifact retention for licensing is separate from physical acceptance evidence.
Neither record should contain USB serial numbers, BLE peripheral identifiers,
private signing keys, device secrets, or provisioned identity material.

## Manifest And Release Records

Each production manifest catalog entry must resolve to a release record with:

- upstream repository, revision, release, and source archive digest.
- binary archive and application-image digests.
- license expression and notice-bundle digest.
- patch-set and build-instruction revisions.
- build environment and toolchain identifiers.
- publication location for corresponding source.
- retention owner and last distribution date.
- legal-review reference when required.

The firmware admission code verifies signatures, target identity, archive
bounds, image layout, and hashes. It does not replace this release gate. A
technically admitted artifact remains unpublished if its source or notice record
is missing.

## Release Check

Before publication:

1. Rebuild the firmware from the retained source bundle.
2. Verify that the rebuilt application and archive match the recorded process.
3. Verify the binary, source, manifest, and notice digests.
4. Confirm that modifications are identified.
5. Confirm that corresponding source is available beside the binary.
6. Confirm that all applicable notices are present.
7. Confirm that installation information is available when required.
8. Record the release and retention owners.

Stop publication if any check fails. Do not substitute a successful firmware
write or physical-device test for source and notice compliance.
