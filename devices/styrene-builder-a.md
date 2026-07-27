# styrene-builder-a

## Identity

| Field | Value | Basis |
|---|---|---|
| Unit ID / hostname | `styrene-builder-a` | Declarative NixOS configuration and physical verification |
| Manufacturer | Raspberry Pi | Hardware identity |
| Model | Raspberry Pi 4 Model B | Hardware profile |
| Role | Native `aarch64-linux` Nix builder | Hardware validated |
| Declarative target | `rpi4b-builder-v1` | `product/flashers/rpi4b-builder-v1.toml` |

## Accepted physical baseline

Observed on 2026-07-17:

| Field | Value |
|---|---|
| Architecture | `aarch64` |
| Kernel | `6.12.75` |
| Nix | `2.34.8` |
| CPU cores | 4 |
| Memory | approximately 7.9 GB |
| Root | `/dev/mmcblk0p2`, ext4 |
| Boot media | `/dev/mmcblk0p1`, FAT image partition |
| Access | `nix-builder`, public-key SSH only |
| Observed DHCP address | `192.168.8.149` |

The Nix daemon was active with sandboxing and distributed builds enabled. A transferred native `aarch64-linux` derivation built successfully:

```text
/nix/store/z190p4xcqkd19n9p40p0lp2pj0za0j2p-rg35xxsp-provenance-check.drv
→ /nix/store/qgvr9s605wl72ws33fd1p0x9lmhannxk-rg35xxsp-provenance-check
```

The DHCP address is historical evidence, not stable identity. Use hostname and the injected SSH public key as the access contract.

## Revalidation

```bash
just nix-rpi4-builder-accept nix-builder@styrene-builder-a.local
```

For native realization evidence, provide an `aarch64-linux` derivation:

```bash
just nix-rpi4-builder-accept nix-builder@styrene-builder-a.local /nix/store/NAME.drv
```

See [../docs/rpi4-builder-image.md](../docs/rpi4-builder-image.md) for build, artifact-validation, delivery, and hardware-validation procedures.
