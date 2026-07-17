# Raspberry Pi 4 Builder Image

This image bootstraps a native `aarch64-linux` Nix remote builder for Styrene.
It is not the constrained communicator appliance image.

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

5. Register it from the developer machine as an SSH Nix builder, then build the
   canonical Styrene ARM64 package and QEMU system through it.

Until SSH succeeds with the injected key and a remote Nix build completes, the
image is structurally flash-ready but the builder role is not operationally
validated.
