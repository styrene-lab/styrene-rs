# RG35XXSP Open Boot-Chain Build Evidence

This is the handoff record for the first source-built RG35XXSP boot-chain baseline. It records what was actually proven, where the immutable outputs live, how they are recovered, which failed paths were already investigated, and what remains before an SD image may be delivered.

## Snapshot

Evidence date: 2026-07-19

Repository commits establishing the path:

```text
d35e8d24 feat(nix): scaffold RG35XXSP appliance bring-up
f01edc05 feat(product): declare repeatable RPi4 builder flasher
96278d9a feat(devices): add bounded OEM media preservation
22682683 fix(evidence): make bounded OEM capture portable
b4039d37 feat(product): scaffold RG35XXSP bring-up contract
0a4c3602 feat(nix): select RG35XXSP boot-chain sources
fc446ff3 feat(nix): pin RG35XXSP open boot-chain sources
1901f251 feat(nix): archive RPi4 build outputs locally
```

Current status:

```text
RPi4 native aarch64 builder        hardware-validated
TF-A                               source-built successfully
U-Boot SPL + second stage          source-built successfully
Linux kernel                       source-built successfully
Linux modules                      source-built successfully
RG35XXSP-specific DTB              source-built and present
Mac recovery archives              complete and checksum-verified
aggregate boot-chain bundle        next build gate
bring-up SD image                  not implemented
physical delivery                  disabled
physical RG35XXSP boot             not attempted
```

A successful source build is not evidence that the physical handheld boots. `product/flashers/rg35xxsp-bringup-v1.toml` remains `planned` with delivery disabled.

## Builder used

Canonical builder contract:

```text
product/flashers/rpi4b-builder-v1.toml
```

Observed accepted machine:

```text
hostname:       styrene-builder-a
SSH user:       nix-builder
observed DHCP:  192.168.8.149
architecture:   aarch64
kernel:         6.12.75
Nix:            2.34.8
CPU:            four cores; Nix limited to one job and three build cores
memory:         approximately 7.9 GiB
root:           /dev/mmcblk0p2, ext4
boot media:     /dev/mmcblk0p1, FAT
```

The DHCP address is evidence, not identity. Prefer `nix-builder@styrene-builder-a.local`; fall back to discovery or the observed address only when multicast DNS is unavailable.

Physical acceptance proved public-key SSH, an active Nix daemon, sandboxing, derivation transfer, and native `aarch64-linux` realization. The initial proof derivation was:

```text
/nix/store/z190p4xcqkd19n9p40p0lp2pj0za0j2p-rg35xxsp-provenance-check.drv
→ /nix/store/qgvr9s605wl72ws33fd1p0x9lmhannxk-rg35xxsp-provenance-check
```

## Immutable source selections

The executable definitions are in:

```text
nix/hardware/rg35xxsp/boot-chain.nix
nix/hardware/rg35xxsp/provenance.toml
```

Pinned baseline:

| Component | Repository | Revision | Build selection |
|---|---|---|---|
| TF-A | `ARM-software/arm-trusted-firmware` | `aa1793fff49a1b5a6a877c278a0df0a188e2b1f2` | `PLAT=sun50i_h616`, `bl31` |
| U-Boot | `u-boot/u-boot` | `88dc2788777babfd6322fa655df549a019aa1e69` | `anbernic_rg35xx_h700_defconfig` |
| Linux | `gregkh/linux` | `9021cc14f7d98b4a1d2c932f52c5343d4d0f6b92` | Linux `7.0.9`, ARM64 defconfig baseline |
| Device tree | same Linux revision | same revision | `sun50i-h700-anbernic-rg35xx-sp.dtb` |

No vendor-extracted boot binary was required for this build baseline.

## Build results

### Trusted Firmware-A

TF-A built natively on the RPi4 and produced `bl31.bin` for the Allwinner H616/H700 family path.

### U-Boot

U-Boot built natively in approximately 3 minutes 18 seconds and produced:

```text
u-boot-sunxi-with-spl.bin
u-boot.bin
u-boot.dtb
.config
```

Observed immutable output:

```text
/nix/store/9pjh4mcwxjmagaglxg1hlpxrnlliln0v-rg35xxsp-u-boot-2026.04-88dc2788777b
```

The build compiled the upstream H700 ANBERNIC device tree and integrated TF-A BL31. This is structural build evidence, not serial boot evidence.

### Linux

The cold, broad ARM64 kernel derivation completed after several hours on three Pi cores. It intentionally used a broad baseline first, resulting in unrelated filesystems and drivers being compiled, including JFS, NFS, Tegra support, generic PCI, and AMDGPU. Do not treat this duration as the expected steady-state handheld kernel workflow.

Derivation:

```text
/nix/store/ff9262qq60q7kj5xl1rplaai3yzr7hgz-rg35xxsp-linux-7.0.9-9021cc14f7d9.drv
```

Outputs:

```text
kernel:
/nix/store/1yyzpm6a3g4q7va6f78r4y19m8y8pzcr-rg35xxsp-linux-7.0.9-9021cc14f7d9

modules:
/nix/store/bc50j3jaa0d5l4axg1vm7g7mqnvw5csv-rg35xxsp-linux-7.0.9-9021cc14f7d9-modules

development output:
/nix/store/sfl35vgrm2a77yssak35yckn3x2qh4k1-rg35xxsp-linux-7.0.9-9021cc14f7d9-dev
```

Approximate uncompressed output sizes:

```text
kernel:   204 MiB
modules:  149 MiB
dev:      894 MiB
```

Required files were confirmed in the kernel output:

```text
Image
dtbs/allwinner/sun50i-h700-anbernic-rg35xx-sp.dtb
```

Additional upstream DTBs present:

```text
sun50i-h700-anbernic-rg35xx-h.dtb
sun50i-h700-anbernic-rg35xx-2024.dtb
sun50i-h700-anbernic-rg35xx-plus.dtb
```

The exact SP DTB exists upstream at the pinned Linux revision. Do not begin an out-of-tree SP DTS patch unless later runtime evidence identifies an actual hardware mismatch.

## Durable local recovery archives

Completed kernel outputs were exported from the Pi with `nix-store --export`, compressed, copied to the Mac, and independently checksum-verified. They are intentionally gitignored under:

```text
.builder-artifacts/rpi4b/
```

| Output | Archive size | SHA-256 |
|---|---:|---|
| kernel | 29,370,802 bytes | `61b0b1d736e9a2a626982fb041892c79e3871b8513e95a9d6c292c3c79906e84` |
| modules | 125,699,716 bytes | `244ce15ea34645b0214ec0bd4ef342a2cd9714c8e32588752a7bb63f71ec0b87` |
| dev | 289,962,669 bytes | `542e988b4ccfc633c360edb72005a6a4be4fd3e67740569891ede1abe80b6170` |

Exact files:

```text
.builder-artifacts/rpi4b/1yyzpm6a3g4q7va6f78r4y19m8y8pzcr-rg35xxsp-linux-7.0.9-9021cc14f7d9.nar.zst
.builder-artifacts/rpi4b/bc50j3jaa0d5l4axg1vm7g7mqnvw5csv-rg35xxsp-linux-7.0.9-9021cc14f7d9-modules.nar.zst
.builder-artifacts/rpi4b/sfl35vgrm2a77yssak35yckn3x2qh4k1-rg35xxsp-linux-7.0.9-9021cc14f7d9-dev.nar.zst
```

Each has a sibling `.manifest` recording source builder, store path, compressed SHA-256, Nix hash, and direct references.

Persistent indirect GC roots also exist on the Pi:

```text
/home/nix-builder/.local/state/styrene/gcroots/<store-basename>
```

Verify local archives:

```bash
for f in .builder-artifacts/rpi4b/*rg35xxsp-linux*.nar.zst; do
  expected=$(sed -n 's/^sha256=//p' "$f.manifest")
  actual=$(shasum -a 256 "$f" | awk '{print $1}')
  test "$expected" = "$actual"
done
```

Restore an output after builder loss or reflash:

```bash
just nix-rpi4-restore .builder-artifacts/rpi4b/OUTPUT.nar.zst
```

Archive another completed output:

```bash
just nix-rpi4-archive-outputs /nix/store/OUTPUT
```

The NAR is the recovery artifact. A plain tar copy does not preserve Nix registration metadata.

## OEM evidence already preserved

Bounded, read-only OEM TF1 evidence is under the gitignored/local evidence tree described by the device-preservation tooling. Confirmed facts include:

```text
62.5 GB GPT media
seven partitions
Android boot image magic on partition 4
Allwinner sun50i ARM64 command line
console=ttyS0,115200
mmc_root=/dev/mmcblk0p5
```

The bounded capture is sufficient for research, not recovery. A full raw image of the OEM card is still required before destructive experiments on that card. Prefer a separate sacrificial card and leave the OEM card untouched.

## Resolved failures and lessons

Do not repeat these investigations:

1. **Incorrect Linux source hash.** The verified Linux fetch hash is the value committed in `boot-chain.nix`; the initial guessed hash failed as expected and was corrected from Nix's fixed-output report.
2. **TF-A tool selection.** Explicit ARM64 compiler/binutils make flags were required for the native derivation.
3. **U-Boot BL31 propagation.** BL31 must be supplied to the U-Boot build; the committed derivation does so.
4. **Kernel duration.** The initial ARM64 defconfig is far too broad for routine handheld work. It compiled desktop/server drivers for many hours. Preserve this baseline; derive a constrained H700 config for subsequent clean builds.
5. **Network outage versus build state.** SSH timeouts did not imply build failure. The Nix build remained active on the Pi and completed. Monitoring must distinguish unreachable from completed.
6. **Unsigned remote Nix outputs.** The Mac daemon rejects unsigned Pi outputs. Do not disable global signature enforcement. Export completed outputs as NARs over authenticated SSH; production should later add a dedicated builder signing key.
7. **Archive watcher resilience.** The first watcher exited on network loss. Direct post-build copy-back succeeded once both systems shared the network. Repository archive/restore commands are the durable interface.
8. **Garbage collection.** Completed long builds need indirect GC roots on the Pi plus Mac NAR backups. Both are now present.
9. **Flashing decompressor short reads.** The RPi image flasher must not use `dd conv=sync`; that previously padded short pipe reads. The corrected guarded flasher is committed separately.
10. **macOS image construction xattrs.** Repository-owned no-host-xattrs image construction is required when macOS virtiofs presents synthetic security xattrs.

## Next exact actions

Proceed in this order:

1. Build `packages.aarch64-linux.rg35xxsp-boot-chain` on the Pi. Kernel, TF-A, and U-Boot should now be reused from the store rather than rebuilt.
2. Archive and GC-root the completed aggregate bundle on the Pi and Mac.
3. Validate bundle contents and SHA-256 manifest:
   - `bl31.bin`;
   - `u-boot-sunxi-with-spl.bin`;
   - `Image`;
   - exact SP DTB;
   - pinned revision record.
4. Replace the broad kernel configuration with a constrained H700 bring-up configuration, but retain the broad output as the known-good source-build baseline.
5. Implement `packages.aarch64-linux.rg35xxsp-bringup-image` with proven Allwinner boot offset and non-overlapping partitions.
6. Implement the real image validator; do not promote the flasher contract merely because the image builds.
7. Obtain complete OEM-card recovery evidence and a separate experimental TF1 card.
8. Promote to `delivery-approved` only after artifact validation and recovery gates pass.
9. Attempt the physical boot ladder one rung at a time: SPL/U-Boot, kernel, stage 1, stage 2, display, input, storage, network enumeration, controlled power.

## Explicitly unresolved

- Physical-unit board revision and whether it differs materially from the upstream compatible.
- Serial access method/pads if panel and HDMI provide no evidence.
- Exact first-image raw boot offset and partition layout validation.
- Internal panel behavior under the pinned mainline kernel.
- Buttons, Hall switch, PMIC, battery, charging, audio, Wi-Fi/BT firmware, HDMI, and suspend.
- Full OEM-media recovery image.
- Generic ARM64 QEMU acceptance for the final first-boot service composition.
- Styrene compact communicator UI and runtime integration.

None of these should be inferred from successful compilation.
