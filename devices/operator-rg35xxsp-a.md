# operator-rg35xxsp-a

## Identity

| Field | Value | Basis |
|---|---|---|
| Unit ID | `operator-rg35xxsp-a` | Registry-assigned |
| Manufacturer | ANBERNIC | Observed rear label |
| Model | RG35XXSP | Observed rear label and stock system-information screen |
| Form factor | Clamshell handheld | Observed |
| Intended Styrene role | First constrained-communicator hardware target | Project decision |
| Deployment tier | `constrained-linux` | Planned composition |
| Build plane | `styrene-builder-a` / `rpi4b-builder-v1` | Project decision |

## Physical interfaces

| Interface | Evidence |
|---|---|
| `TF1/INT` microSD slot | Observed; OEM-included card populated at baseline |
| `TF2/EXT.` microSD slot | Observed; empty at baseline |
| USB-C labeled `USB/OTG` | Observed |
| Mini-HDMI labeled `HD` | Observed |
| 3.5 mm headphone jack | Observed |
| Shoulder controls `L1`, `L2`, `R1`, `R2` | Observed |
| Clamshell hinge/lid | Observed |

## Stock system baseline

Captured from the stock system-information screen:

| Field | Value | Basis |
|---|---|---|
| Firmware version | `20251224` | Observed; date interpretation not yet confirmed |
| SoC | Allwinner H700 | Observed system screen |
| Kernel | Linux `4.9.170` | Observed system screen |
| CPU cores | 4 | Observed system screen |
| Maximum CPU frequency | 1.5 GHz | Operator-reported from system information |
| Reported memory | 973 MiB | Observed system screen |
| Available memory at capture | 762 MiB | Observed system screen |
| Filesystem type | ext4 | Observed system screen |
| User disk usage | 27.36 GB / 43.99 GB (63%) | Observed system screen |
| System disk usage | 3.66 GB / 6.87 GB (56%) | Observed system screen |
| Battery voltage at capture | 3.54 V | Observed system screen |
| Wi-Fi UI | Present | Observed status indicator; connectivity/chipset not established |

The rear label identifies a Li-Po battery. The small electrical text appears consistent with the manufacturer's published 3.7 V / 3300 mAh specification, but the photograph is not sharp enough to treat the complete electrical transcription as direct evidence.

## Storage baseline

```text
TF1/INT: OEM-included card present
TF2/EXT.: empty
```

The contents, capacity, partition map, boot sectors, and exact role of the TF1 card remain unknown until it is imaged and inspected. Because TF2 was empty, the first product image must not require a second card without contrary evidence.

## USB/OTG observations

A USB-C-to-USB-C connection to the developer Mac was tested while the handheld was off and again after normal stock-firmware boot.

Host-side checks found no new:

- USB vendor/product enumeration;
- network interface;
- serial device; or
- external disk.

This establishes only that the stock firmware does not export a USB data function in the tested normal-boot state. It does not establish whether the cable carries data, whether host mode works, whether a gadget controller exists, or whether recovery mode is available.

## Evidence state

Confirmed for this physical unit:

- commercial manufacturer and model;
- H700 SoC family;
- stock kernel version;
- core count and reported memory;
- stock firmware identifier;
- physical slot labels and baseline occupancy;
- USB-C OTG, mini-HDMI, headphone, and shoulder-control presence.

Unresolved:

- board revision and exact DTB;
- panel identifier and display interface;
- bootloader, firmware, kernel, and device-tree provenance;
- OEM card image, capacity, partition table, and hashes;
- USB host/device/recovery capabilities;
- Wi-Fi/Bluetooth chipset and firmware;
- input event mapping;
- Hall sensor and suspend/resume behavior;
- HDMI modes/audio/hotplug behavior;
- audio devices and routing;
- battery, charging, and power-supply interfaces.

## Next safe evidence

1. Shut down normally and photograph the OEM TF1 card front/back.
2. Identify the inserted card with `diskutil list external physical` before reading it.
3. Create and hash a full-card image, including unpartitioned boot regions.
4. Preserve the partition table and first 64 MiB separately.
5. Find the device on Wi-Fi and test whether SSH or another documented remote service is available.
6. Capture read-only device-tree, storage, input, network, display, audio, power, kernel-config, and boot-file evidence.

Do not update firmware, initialize TF2, expose live state as USB mass storage, or flash either slot before the OEM TF1 image is preserved and verified.
