# Raspberry Pi 4 Builder Image

This image bootstraps a native `aarch64-linux` Nix remote builder for Styrene.
It is not the constrained communicator appliance image.

## Contract

The repeatable interface is machine-readable at:

```text
product/flashers/rpi4b-builder-v1.toml
```

The contract binds the Nix flake attribute, artifact validator, guarded delivery
backend, first-boot identity, required hardware checks, and the first accepted
physical run. Validate it with:

```bash
python3 scripts/validate_flashers.py product/flashers/rpi4b-builder-v1.toml
```

Its lifecycle is intentionally four separate operations:

```text
materialize → validate artifact → deliver → validate hardware
```

A later Nex Forge adapter should consume this contract rather than duplicate the
commands or infer success from a built image.

## Build

Provide the operator's public SSH key at evaluation time:

```bash
export STYRENE_BUILDER_SSH_KEY="$(cat ~/.ssh/id_ed25519.pub)"
./scripts/build-nix-linux.sh \
  .#nixosConfigurations.rpi4-builder.config.system.build.sdImage \
  result-rpi4-builder
```

The Linux-container builder uses a persistent Nix volume and the repository's
no-host-xattrs ext4 creator. This avoids unreadable synthetic
`security.selinux` attributes exposed through macOS virtiofs while preserving
file types, permissions, symlinks, and the Nix store closure.

## Validated artifact

The first successful artifact was:

```text
result-rpi4-builder/sd-image/nixos-image-sd-card-26.11.20260713.6cdc7fc-aarch64-linux.img.zst
compressed sha256:   62623a90bfc1744aae2bab248120770ee43e2bddcac48b9048347cc0ba66b8d8
uncompressed sha256: 2adffb2a346e45485c63ffe2c4b9092d456a303dfda2863bd643e87b91415648
uncompressed bytes:  3178119168
```

Validation completed before enabling flashing:

- valid DOS/MBR signature `0xAA55`;
- 256 MiB FAT firmware partition labeled `FIRMWARE`;
- ext4 root partition labeled `NIXOS_SD`;
- independent Linux `e2fsck -fn` completed all five passes cleanly;
- firmware partition contains `bcm2711-rpi-4-b.dtb`, Raspberry Pi firmware,
  `armstub8-gic.bin`, and `u-boot-rpi4.bin`;
- `config.txt` selects 64-bit boot, UART, and U-Boot;
- the image closure identifies the expected `styrene-builder-a` NixOS system.

The declarative SSH key is an evaluation input to the NixOS closure. Focused
evaluation confirms the exact supplied key appears at
`config.users.users.nix-builder.openssh.authorizedKeys.keys`. `/etc` is created
by NixOS activation on first boot, so the key is not expected at
`/home/nix-builder/.ssh/authorized_keys` in the inactive root image. Successful
first-boot SSH remains a physical acceptance gate.

## Guarded flash

Run the non-destructive guard tests first:

```bash
./scripts/test-flash-rpi4-image.sh
```

Inspect the target disk with the host's disk tooling. Then validate without
writing:

```bash
./scripts/flash-rpi4-image.sh \
  --image result-rpi4-builder/sd-image/nixos-image-sd-card-*.img.zst \
  --device /dev/diskN \
  --confirm ERASE \
  --dry-run
```

Actual flashing is destructive and requires an explicit whole removable disk,
the exact `ERASE` confirmation, and root privileges:

```bash
sudo ./scripts/flash-rpi4-image.sh \
  --image result-rpi4-builder/sd-image/nixos-image-sd-card-*.img.zst \
  --device /dev/diskN \
  --confirm ERASE
```

The script refuses image-file targets, partitions, mounted Linux disks,
non-removable Linux disks, internal/virtual macOS disks, and the Linux root
disk. It accepts only `.img` and `.img.zst` artifacts.

## Physical acceptance — first declarative flasher

The first physical run completed on 2026-07-17:

```text
hostname:       styrene-builder-a
observed DHCP:  192.168.8.149
architecture:   aarch64
kernel:         6.12.75
Nix:            2.34.8
CPU cores:      4
memory:         approximately 7.9 GB
root:           /dev/mmcblk0p2 (ext4)
boot:           /dev/mmcblk0p1 (FAT)
```

Public-key SSH as `nix-builder` succeeded. The Nix daemon was active with
sandboxing and distributed builds enabled. A transferred native
`aarch64-linux` derivation built successfully:

```text
/nix/store/z190p4xcqkd19n9p40p0lp2pj0za0j2p-rg35xxsp-provenance-check.drv
→ /nix/store/qgvr9s605wl72ws33fd1p0x9lmhannxk-rg35xxsp-provenance-check
```

This advances `rpi4b-builder-v1` to `hardware-validated`. The observed DHCP
address is evidence only; hostname plus the injected SSH public key are the
stable access contract.

## Lessons fixed in the workflow

- macOS virtiofs can expose unreadable synthetic `security.selinux` attributes;
  image construction therefore uses the repository-owned no-host-xattrs ext4
  path while preserving inode types, modes, symlinks, and Nix store contents;
- literal backslashes in systemd store paths must survive debugfs population;
- the inactive image needs a valid Nix registration database and writable
  daemon state prepared on boot;
- offline alternate-store verification must use `local?root=/verify/root`;
- decompressor pipe reads may be short, so the `dd` path must never use
  `conv=sync`;
- a built-in SDXC reader may report `Internal: Yes` while its media is correctly
  `Removable`; delivery checks both properties and still rejects internal,
  non-removable disks;
- build, artifact verification, destructive delivery, boot, SSH, and native
  build are independent evidence gates.

## Reproduction checklist

1. Set `STYRENE_BUILDER_SSH_KEY` to the intended operator public key.
2. Build with `just nix-rpi4-builder-build`.
3. Verify the resulting image with `scripts/verify-rpi4-image.sh`.
4. Run `just nix-rpi4-flash-test`.
5. Identify the inserted whole removable disk and run the flash dry-run.
6. Flash only with the explicit `ERASE` confirmation.
7. Boot with Ethernet connected and verify hostname/public-key SSH.
8. Run `scripts/verify-rpi4-builder-host.sh --host nix-builder@HOST` to verify
   architecture, mounts, Nix daemon, and sandbox state.
9. Re-run it with `--derivation /nix/store/NAME.drv` to copy and realize a
   non-substituted `aarch64-linux` derivation on the Pi.
10. Record build, artifact, delivery, and hardware evidence independently.

## First-boot acceptance

1. Connect Ethernet and, preferably, a 3.3 V USB serial adapter at 115200 baud.
2. Boot the Pi and confirm hostname `styrene-builder-a`.
3. Find its DHCP address and connect:

   ```bash
   ssh nix-builder@styrene-builder-a.local
   ```

4. Confirm the machine and Nix daemon:

   ```bash
   uname -m
   nix --version
   nix-store --verify --check-contents
   ```

5. The first physical run completed all builder-role gates. Repeat runs must
   still execute them; historical acceptance does not prove a newly built or
   newly flashed artifact.

The builder is operationally validated when SSH succeeds with the injected key
and a native, non-substituted `aarch64-linux` derivation completes.
