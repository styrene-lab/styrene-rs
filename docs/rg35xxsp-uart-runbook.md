# RG35XXSP UART Evidence Runbook

## Scope

This is a manual bring-up procedure for obtaining the serial-console evidence required by `docs/hardware-evidence-boundary.md`. It does not define a Styrene adapter framework or assign general hardware probing to Styrene.

External hardware tooling—ultimately Nex—owns adapter discovery, electrical profiles, capture provenance, and active control. Styrene consumes the resulting evidence reference and evaluates product-relevant markers.

## Safety

Use a USB-to-TTL adapter whose logic voltage is measured and compatible with the target. Do not use RS-232 voltage levels or assume a nominal adapter setting is correct.

Begin receive-only:

```text
RG35XXSP GND → adapter GND
RG35XXSP TX  → adapter RX
adapter TX disconnected
adapter VCC disconnected
```

Rules:

- power the handheld from its own battery;
- identify ground with the target unpowered using continuity to known ground;
- measure candidate signal idle voltage before connecting adapter RX;
- prefer pogo pins, micro-grabbers, or a high-impedance logic analyzer;
- disconnect the battery before soldering;
- do not move loose conductive probes over a powered board;
- do not connect adapter VCC during observation.

## Equipment

- 3.3 V-capable USB-to-TTL adapter, subject to measured confirmation;
- multimeter;
- fine probes or pad connector;
- optional logic analyzer or oscilloscope;
- magnification and nonconductive work surface.

Published RG35XX-family pinouts are leads, not proof for this physical SP board revision.

## Identify the interface

1. Power off and remove both cards and external cables.
2. Open the shell without stressing battery or display ribbons.
3. Photograph the full PCB and candidate test-pad groups.
4. With battery disconnected, identify ground by continuity to USB/HDMI shield or battery negative.
5. Reconnect battery and measure candidate signal idle voltages.
6. Use the OEM card and watch for power-on bursts on a candidate TX pad.
7. Validate readable output before drawing conclusions from the Styrene card.

The OEM card is the control that distinguishes a silent target from wrong wiring, wrong pad, wrong voltage, or wrong serial parameters.

## Discover the USB adapter on macOS

Before and after connecting the adapter:

```bash
ls /dev/cu.*
system_profiler SPUSBDataType
```

Common paths include:

```text
/dev/cu.usbserial-*
/dev/cu.SLAB_USBtoUART
/dev/cu.wchusbserial*
```

The path is an observation, not stable adapter identity.

## Capture

Start with:

```text
115200 baud
8 data bits
no parity
1 stop bit
no hardware/software flow control
```

The candidate kernel requests `console=ttyS0,115200`; this does not prove BootROM or U-Boot uses the same parameters. If the OEM control remains unreadable after wiring is electrically verified, test alternate explicit profiles such as 1500000, 921600, 57600, and 38400 baud.

Start capture before applying target power. Preserve raw bytes rather than relying on terminal scrollback. Do not edit the raw capture; derive readable transcripts separately.

Tools such as `picocom`, `tio`, Sigrok, or a small termios capture utility are acceptable if the evidence result records the tool/version, parameters, timestamps, termination, and raw SHA-256.

## Required sessions

Capture separately:

```text
rg35xxsp-oem-cold-boot-01
rg35xxsp-styrene-cold-boot-01
rg35xxsp-styrene-reset-01
rg35xxsp-styrene-cold-boot-02
```

For every session record:

- target and PCB revision;
- adapter identity and measured logic voltage;
- photographed pad/wiring map;
- serial parameters;
- TF1 media and TF2 state;
- battery/external power state;
- cold boot versus reset;
- LED and display behavior;
- raw capture digest;
- capture start/end and termination reason.

Hash raw files:

```bash
shasum -a 256 rg35xxsp-*.raw > rg35xxsp-uart-sha256.txt
```

A readable copy may be derived without modifying the source:

```bash
tr -d '\r' < session.raw > session.txt
```

## Interpretation

- OEM silent: setup is unproven; do not infer a Styrene boot failure.
- OEM readable and Styrene silent: investigate BootROM acceptance, SPL header/layout, DRAM initialization, and bootloader placement.
- SPL output followed by failure: inspect DRAM, PMIC, clocks, and board-specific support.
- U-Boot output but no boot entry: inspect MMC index, partition scanning, filesystem support, and extlinux lookup.
- Kernel output with dark display: boot chain works; inspect panel, backlight, regulators, GPIO, and console routing.
- Kernel panic: preserve the exact panic and preceding device/root-mount messages.

Styrene acceptance consumes the external evidence result and its markers. The capture mechanism itself is outside Styrene's ownership boundary.
