+++
id = "rg35xxsp-nixos-bringup"
kind = "design_node"

[data]
title = "RG35XXSP Minimal NixOS SD Image Bring-up"
status = "exploring"
issue_type = "architecture"
priority = 1
dependencies = ["5dc53189-e2e3-44fb-ba23-e95b7b053f0b", "constrained-communicator-constraints"]
open_questions = [
  "[assumption] The operator's RG35XXSP is an H700/1 GiB production unit compatible with the current upstream RG35XXSP board support lineage",
  "[assumption] Upstream or ROCKNIX-derived U-Boot can initialize the RG35XXSP sufficiently to load a NixOS kernel and initrd from TF1",
  "[assumption] The current mainline H700 kernel and RG35XXSP device tree can initialize the internal panel, controls, storage, and power-off path",
  "[assumption] HDMI or another observable channel can provide diagnostics when the internal panel does not initialize",
  "Exact board revision, RAM type, stock firmware version, and device-tree compatible strings of the unit in hand",
  "Which upstream U-Boot revision and RG35XXSP defconfig/DTB combination is currently proven by ROCKNIX?",
  "Which kernel revision and patch set are required for panel, Hall switch, input, Wi-Fi, audio, HDMI, PMIC, and suspend?",
  "Which firmware blobs are required, under what licenses, and may they be redistributed in a Styrene image?",
  "Does upstream NixOS stage 1 fit the bootloader, memory, and partition constraints without device-specific initrd changes?",
  "Which serial UART pads and voltage levels are available if both panel and HDMI fail?",
]
+++

## Overview

The first physical constrained-device milestone is a **minimal NixOS SD image that boots on the ANBERNIC RG35XXSP from a new TF1 card without modifying the device or its original media**. The image will use NixOS for the complete root userspace and service model. It may import proven board-support source, patches, device trees, and redistributable firmware from upstream Linux/U-Boot and handheld projects such as ROCKNIX. It will not use ArkOS, KNULLI, or ANBERNIC stock as the root filesystem.

This is a staged board bring-up, not yet a constrained communicator release. The first success condition is an observable, repeatable NixOS boot with controlled shutdown and enough hardware enumeration to support later Styrene integration. Compact UI, suspend policy, production updates, dual-card persistence, and field support follow only after the base system is proven.

## Goal

From the Styrene repository on the developer machine:

```text
nix build .#rg35xxsp-image
        ↓
result/styrene-nixos-rg35xxsp-<revision>.img.zst
        ↓
non-destructive validation and manifest
        ↓
explicit operator-approved write to a named removable SD device
        ↓
RG35XXSP boots minimal NixOS
        ↓
observable boot evidence + hardware inventory
```

The path from materialization to delivery is intentionally split. Building or validating an image must never write removable media. Flashing must name and re-confirm the target device, reject the developer machine's system disk, and remain a separate Nex-owned delivery action.

## Non-goals for the first boot

- A desktop environment or gaming frontend.
- The final Styrene compact communicator UI.
- On-device Nix evaluation, builds, channels, or garbage collection.
- Replacing or erasing either original TF card.
- TF2 data-card integration.
- Suspend/resume or lid-close product policy.
- Audio, Bluetooth, HDMI audio, or GPU acceleration as boot gates.
- Automatic Wi-Fi provisioning.
- Production signing, OTA updates, or A/B rollback.
- Generalizing support to all H700 devices or all RG35XXSP revisions.

## Upstream footholds

### NixOS foundation

Use the standard `aarch64-linux` NixOS system and SD-image machinery as the userspace foundation. The target consumes a prebuilt closure; Nix is not enabled as an on-device package manager.

Relevant upstream contracts:

- NixOS ARM SD-image and generic extlinux/U-Boot support;
- `hardware.deviceTree` for explicit board DTB selection;
- NixOS initrd/stage 1, systemd stage 2, immutable `/nix/store`, and declarative services;
- Nix cross/native ARM image construction through a Linux builder.

### H700 NixOS precedent

A published RG35XX-H NixOS bring-up demonstrates the useful pattern:

1. standard NixOS AArch64 SD image;
2. H700-compatible upstream U-Boot written at the Allwinner SD offset;
3. explicit `sun50i-h700-anbernic-rg35xx-h.dtb` selection;
4. mainline kernel and NixOS userspace.

The RG35XX-H is not the SP, but it proves the H700-to-NixOS boundary. The SP work should be modeled as board specialization rather than a new distribution.

### Exact-device board support

Current ROCKNIX releases list the RG35XXSP in their H700 target family. Treat ROCKNIX as the primary exact-device implementation reference for:

- U-Boot revision, defconfig, and patches;
- Linux revision, config, and patches;
- SP DTS/DTB;
- input, Hall switch, PMIC, display, Wi-Fi/BT, audio, HDMI, and suspend integration;
- firmware selection and boot partition layout.

Import only files with understood provenance and acceptable licenses. Distribution images and non-commercial assets are not blanket dependencies.

### Vendor/KNULLI evidence

ANBERNIC stock and KNULLI remain behavioral and fallback oracles. Vendor boot blobs may be used only as an explicitly classified temporary bring-up dependency if the source-built path cannot reach an observable boot. Such a fallback does not replace the source-first target and must have a removal plan.

## Architecture

### Repository shape

```text
nix/
├── flake.nix
├── modules/
│   ├── minimal-appliance.nix
│   ├── image-layout.nix
│   ├── first-boot-evidence.nix
│   └── styrene.nix
├── hardware/
│   └── rg35xxsp/
│       ├── default.nix
│       ├── u-boot.nix
│       ├── kernel.nix
│       ├── device-tree.nix
│       ├── firmware.nix
│       └── provenance.toml
└── tests/
    ├── image-layout.py
    └── qemu-userspace.nix
```

The actual implementation may place `flake.nix` at repository root if Styrene adopts Nix broadly. The module boundary remains the same.

### Image layout

The first image uses one card and minimizes variables:

```text
TF1
├── reserved Allwinner boot area
│   └── source-built SPL/U-Boot at board-required offset
├── FIRMWARE (FAT)
│   ├── extlinux/extlinux.conf or boot.scr
│   ├── kernel
│   ├── initrd
│   └── allwinner/sun50i-h700-anbernic-rg35xx-sp.dtb
└── NIXOS (ext4)
    ├── immutable Nix store/system closure
    ├── machine configuration
    └── temporary first-boot evidence area
```

A separate persistent data partition is deferred until boot reliability is established. The first image can persist diagnostic evidence on the root filesystem because it is disposable test media. The production design will separate immutable system and writable Styrene state.

### Minimal NixOS profile

The first system includes only:

- NixOS stage 1 and systemd stage 2;
- kernel, required modules, DTB, and firmware;
- console/getty on every proven diagnostic output;
- BusyBox/core diagnostic commands or their minimal Nix equivalents;
- `udev`, input enumeration, mount tools, and network inspection;
- SSH only in a development variant and only after explicit key injection;
- a first-boot evidence collector;
- poweroff/reboot commands.

It excludes:

- GUI/display manager;
- EmulationStation/RetroArch;
- compiler/toolchain;
- Nix daemon and local build support;
- password login and default credentials;
- general package collection;
- Styrene until the base boot gate passes.

### Evidence collector

On each boot, a bounded oneshot service writes a redacted report including:

- image and Git revision;
- `/proc/device-tree/model` and `compatible`;
- kernel command line and version;
- memory and CPU inventory;
- block devices and mounted filesystems;
- DRM/framebuffer connectors and modes;
- input device names/capabilities;
- power supplies and Hall/lid input if present;
- network interfaces, drivers, and firmware failures;
- audio cards;
- failed systemd units and relevant kernel errors.

The report must exclude secrets, Wi-Fi credentials, private keys, and stable identifiers not required for debugging.

## Provenance model

Every non-nixpkgs board-support input is recorded in `provenance.toml` with:

```toml
name = "..."
kind = "upstream-source | community-source-patch | redistributable-firmware | vendor-binary | observed-configuration | temporary-bringup-binary"
source_url = "..."
revision = "immutable revision or digest"
license = "..."
sha256 = "..."
required_for = ["boot", "display"]
replacement_plan = "..." # mandatory for binary/vendor fallbacks
```

Nix fetchers must pin immutable revisions and hashes. A reproducible Nix derivation does not turn an opaque blob into source; the manifest preserves that distinction.

## Work packages

### WP0 — Preserve and identify the physical unit

Before writing test media:

1. Image and hash both original TF cards.
2. Record slot labels and boot behavior with each card absent.
3. Capture stock firmware version and device self-reporting.
4. Photograph external labels and ports.
5. Identify any accessible serial pads without modifying the board.
6. Obtain at least one known-good new microSD card for experiments.

**Gate:** original media can be restored independently, and the experimental card is unambiguously identified.

### WP1 — Pin the upstream board-support bill of materials

Research and record immutable revisions for:

- nixpkgs/NixOS release;
- U-Boot and TF-A if required;
- Linux kernel;
- RG35XXSP DTS;
- ROCKNIX patches/configuration used for exact-device support;
- Wi-Fi/BT and other firmware;
- image partition offsets and boot command.

Compare upstream H700 support against ROCKNIX. Classify each delta as:

- already upstream;
- patch required;
- configuration only;
- firmware required;
- unresolved/vendor-only.

**Gate:** every boot-critical input has a source, revision, hash, license, and role.

### WP2 — Materialize a generic minimal AArch64 NixOS image

Implement a Nix flake and minimal module that produces an image without handheld board support. Validate:

- evaluation/build on the developer machine through a Linux builder;
- deterministic partition table and filesystem labels;
- expected system closure and no accidental desktop/toolchain content;
- kernel/initrd/DTB placement contract;
- image manifest and compression;
- no write-to-device behavior.

QEMU may validate NixOS userspace and first-boot service logic, but not H700 hardware support.

**Gate:** the generic image boots under an applicable virtual AArch64 machine or its system closure passes equivalent VM tests.

### WP3 — Build the source-first RG35XXSP boot stack

Package U-Boot, TF-A where required, kernel, DTS, modules, and firmware as Nix derivations. Apply the smallest pinned patch set needed from exact-device upstreams.

Static validation must check:

- bootloader exists at the expected raw offset;
- no partition overlaps the boot area;
- kernel architecture is AArch64;
- DTB contains the expected compatible strings;
- kernel modules match the kernel release;
- required firmware paths exist;
- extlinux/boot script references files present in the image;
- all board-support inputs appear in provenance metadata.

**Gate:** one deterministic, structurally valid image exists with no Ubuntu/ArkOS/KNULLI root filesystem.

### WP4 — Non-destructive inspection and delivery plan

Before writing media, provide:

```text
nex image inspect <artifact>
nex image write --target <explicit removable device> <artifact>
```

or an equivalent host action with these safeguards:

- materialization and writing are separate commands;
- target must be a whole removable device, not a partition;
- show path, model, serial suffix, capacity, mounted partitions, and current partition table;
- reject the host boot/system disk;
- require unmounting and an explicit confirmation tied to the displayed device;
- verify image digest before write;
- flush writes and re-read/verify written ranges or partitions;
- emit a delivery receipt with image digest and target facts;
- never choose a target automatically.

Until Nex owns this safely, the design may emit an exact manual `dd`/imager procedure, but no repository recipe may hide the destructive target argument.

**Gate:** operator can distinguish the experimental card from all host storage and recover from a wrong-image boot without touching original media.

### WP5 — First physical boot ladder

Attempt one rung at a time and preserve evidence:

1. **Power/bootloader:** LED/backlight or serial proves SPL/U-Boot execution.
2. **Kernel load:** serial, HDMI, or panel shows kernel start.
3. **Stage 1:** initrd starts and finds the root filesystem.
4. **Stage 2:** NixOS reaches `multi-user.target`.
5. **Display:** internal panel exposes a stable console or diagnostic surface.
6. **Input:** buttons enumerate and generate stable evdev events.
7. **Storage:** TF1 root is stable across reboot and controlled poweroff.
8. **Network enumeration:** Wi-Fi device/driver/firmware loads; association is not yet required.
9. **Power:** reboot and shutdown behave predictably.

After each failure, change one layer only. A black screen must not be treated as proof that SPL/kernel failed; consult serial, HDMI, partition evidence, or boot markers.

**Gate:** three consecutive cold boots reach stage 2 and three controlled shutdown/reboot cycles preserve filesystem integrity.

### WP6 — Add Styrene acceptance surfaces

Add the packaged Styrene closure and a disabled-by-default service. Validate on device:

```text
styrene --version
styrene doctor --root /var/lib/styrene
styrene ghost-check --root /run/styrene/ghost --timeout 15
```

Then enable a foreground supervised service only after these pass. Capture RSS/PSS, startup latency, file writes, failures, and cleanup.

**Gate:** persistent setup and Ghost lifecycle pass three times without affecting boot reliability or writing outside declared state roots.

### WP7 — Graduate board capabilities

Independently characterize and promote:

- Wi-Fi association and reconnect;
- mini-HDMI hotplug and console mode;
- Hall switch/lid events;
- suspend/resume;
- battery/charging telemetry;
- audio playback/capture;
- TF2 behavior;
- compact UI input/rendering.

None of these blocks the earliest NixOS console boot unless required for diagnosis.

## Verification matrix

| Evidence | Host static | QEMU/VM | Physical RG35XXSP |
|---|---:|---:|---:|
| Nix evaluation and reproducibility | required | — | — |
| Image layout and boot offsets | required | — | — |
| Minimal userspace boots | — | required | required |
| U-Boot/kernel/DTB exact-device behavior | partial | not representative | required |
| Internal display and controls | no | no | required |
| Styrene doctor/Ghost lifecycle | required in ARM container | useful | required |
| Wi-Fi, HDMI, Hall, suspend, battery | no | no | later physical gates |

Evidence always records image digest, source revisions, hardware revision, exact card, boot rung reached, and observed output.

## Failure and recovery policy

- Original cards are never modified during bring-up.
- Test only from a new TF1 card; leave TF2 absent until explicitly tested.
- Keep stock/KNULLI media available solely as recovery and behavior references.
- A failed boot is recovered by removing the experimental card, not by modifying internal storage—the device is assumed to have no required internal system installation.
- Do not repeatedly hard-power a mounted writable filesystem. Prefer serial/U-Boot reset or wait for a known timeout; re-image test media when integrity is uncertain.
- No secrets or production Styrene identity are placed on bring-up images.

## Initial implementation slice

The first implementation change should stop before physical flashing and deliver:

1. pinned upstream research in `provenance.toml`;
2. a Nix flake producing a generic minimal AArch64 NixOS image;
3. an RG35XXSP hardware module skeleton with explicit unresolved inputs;
4. deterministic image-layout and provenance tests;
5. a QEMU userspace boot test where feasible;
6. a non-destructive image inspection report;
7. no media-writing command.

The following slice fills the pinned U-Boot/kernel/DTB derivations, produces the first candidate image, and hands it to a separately reviewed delivery workflow.

## Decision gate for first write

The first SD write is authorized only when:

- original cards are imaged and hashed;
- target hardware identity is recorded as far as possible;
- image layout and raw boot offsets pass static checks;
- bootloader/kernel/DTB revisions and provenance are pinned;
- required licenses permit the intended use and redistribution;
- the exact removable target can be displayed and independently confirmed;
- the candidate image digest is recorded;
- a recovery card and removal-based recovery procedure are ready;
- no unresolved question suggests a risk to hardware rather than merely a failed boot.

## Success criterion

This design reaches its first milestone when a source-first, Nix-built SD image boots the operator's RG35XXSP to minimal NixOS stage 2 on three consecutive cold starts, emits a provenance-linked evidence report, responds to controls through evdev, and performs controlled reboot and shutdown without filesystem damage. Styrene integration is the immediately following gate, not part of claiming the board boot itself.

## Generic ARM64 VM service evidence

The generic `aarch64-linux` QEMU system consumes the repository-owned
`packages.aarch64-linux.styrene` closure and runs `styrene-qemu-smoke.service`
at boot. The service executes:

```text
styrene --version
styrene doctor --root /state/doctor
styrene ghost-check --root /state/ghost --timeout 15
```

Success writes `/state/evidence/qemu-smoke.pass` and emits the serial marker
`STYRENE_QEMU_SMOKE=pass`. Failure leaves no marker and fails the unit. This
proves system composition and the real product lifecycle on generic ARM64
`virt` hardware only; it does not change the unresolved H700 hardware status.
