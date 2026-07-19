# Hardware Interface Observation and Control

## Purpose

Styrene needs a reusable way to discover, observe, and eventually control hardware through interfaces such as UART, USB serial, SPI, I²C, GPIO, SWD/JTAG, and logic-analyzer captures. The RG35XXSP boot investigation is the first concrete consumer, but it must not become a device-specific pile of shell commands.

This document defines the product boundary, safety posture, evidence format, and implementation sequence for a **hardware-interface observation subsystem**.

The subsystem is not a mesh transport and is not equivalent to the existing Reticulum serial/KISS interface. It is an engineering and lifecycle surface used to:

- identify attached adapters and target interfaces;
- capture raw, timestamped evidence without interpreting it away;
- compare a known-good control run with an experimental run;
- decode captures through explicit, versioned profiles;
- permit active transmit or bus control only after electrical and operator approval;
- bind evidence to a device, artifact, flasher lifecycle, and acceptance gate.

## Architectural Separation

Three existing concepts must remain distinct.

### Product data transport

`styrene-rns` serial/KISS and `EmbeddedLinkAdapter` carry application or network frames. Their concern is delivery semantics such as MTU, ordering, acknowledgements, and reconnection.

### Device-specific hardware clients

Examples include the nRF52840 entropy coprocessor in `styrene-entropy`. These clients own a known device protocol, framing, health semantics, and operating parameters.

### Hardware observation and control

The new subsystem operates below those clients. It owns adapter discovery, electrical constraints, capture sessions, raw evidence, replayable decoding, and guarded active operations. A product client may later consume an approved interface, but observation must work before the target protocol is understood.

The dependency direction should be:

```text
hardware adapter/backend
        ↓
observation session + raw evidence
        ↓
versioned decoder/profile
        ↓
optional device-specific client
        ↓
product capability
```

Do not make product transports depend directly on macOS `/dev/cu.*`, Linux `/dev/ttyUSB*`, a specific FTDI library, or one logic-analyzer brand.

## Common Interface Model

A hardware interface should be represented by four independent records.

### Adapter

The host-side instrument:

- stable session-local adapter identifier;
- USB vendor/product IDs and serial number when safe to retain;
- driver and host device path;
- supported electrical modes and voltages;
- supported protocols, directions, rates, channels, and sampling limits;
- whether the adapter can source power;
- whether output pins default to high impedance.

A changing host path such as `/dev/cu.usbserial-1420` is an observation, not stable identity.

### Target connection

The physical relationship to the target:

- target device identity and board revision;
- photographed pad/header location;
- signal-to-adapter wiring map;
- measured voltage and idle state;
- common-ground confirmation;
- directionality;
- series resistors, level shifters, or isolation;
- target and adapter power sources;
- operator who approved the connection.

### Session

One bounded interaction:

- immutable session ID;
- start/end timestamps and clock source;
- interface profile and parameters;
- adapter and target connection references;
- stimulus: cold boot, reset, command, transaction, or passive observation;
- media/artifact under test;
- requested safety mode;
- raw capture file and SHA-256;
- tool/backend versions;
- interruptions, overruns, dropped samples, and termination reason.

### Interpretation

Derived, reproducible analysis:

- decoder name and version;
- source raw-capture hash;
- decoded transcript or transactions;
- annotations and inferred stage boundaries;
- confidence and unresolved ambiguity;
- comparison to a control session.

Raw captures are immutable. Decoding and annotation produce new artifacts; they never replace the source.

## Safety Modes

Every interface profile declares one of these modes.

### `observe-only`

Host inputs only. No target power, bus drive, serial transmit, reset, or GPIO output is permitted.

For UART this normally means:

```text
target GND → adapter GND
target TX  → adapter RX
adapter TX disconnected
adapter VCC disconnected
```

This is the default discovery mode.

### `passive-bus`

A high-impedance logic analyzer observes an existing bus without becoming a participant. Appropriate for SPI, I²C, GPIO, and clocks only after voltage compatibility and grounding are confirmed.

### `interactive`

The host may transmit, but only according to an approved connection profile. UART TX, I²C controller operation, SPI controller/chip-select control, reset, and boot-mode pins belong here.

### `power-capable`

The adapter may source target power or alter rails. This is a separate destructive/high-risk authority and must never be implied by `interactive` mode.

The implementation must fail closed when requested behavior exceeds the profile. Adapter discovery is not permission to drive pins.

## Electrical Rules

- Never connect an RS-232 ±12 V interface to TTL/CMOS pads.
- Never assume USB adapter logic voltage from its product name or jumper position; measure it.
- Never connect adapter VCC during initial observation.
- Establish common ground before attaching a signal.
- Identify ground with the target unpowered using continuity to a known ground point.
- Measure target idle voltage before connecting adapter RX.
- Prefer receive-only or high-impedance probing first.
- Use level shifting when voltage domains differ or remain uncertain.
- A small series resistor can limit fault current during initial UART receive probing, but it does not make an incompatible voltage safe.
- Do not attach or move probes while loose conductors can short adjacent pads.
- Disconnect target power before soldering.
- Record adapter power source and target power source independently.

These constraints belong in machine-readable profiles eventually; documentation alone is not an enforcement boundary.

## Interface Families

### UART and USB-to-TTL serial

Parameters:

- target voltage domain;
- baud rate;
- data bits;
- parity;
- stop bits;
- flow control;
- signal inversion;
- RX/TX direction;
- optional break behavior.

Capture raw bytes before text conversion. Preserve framing, malformed bytes, NULs, and binary output. A readable transcript is derived evidence.

USB CDC ACM and native USB serial devices use the same byte-stream capture model but do not imply access to target-level UART pads.

### SPI

A passive capture needs at least clock, controller-out/peripheral-in, controller-in/peripheral-out, chip select, and ground. The profile records:

- voltage;
- clock polarity and phase;
- bit order;
- word width;
- chip-select polarity;
- sample rate;
- channel map.

SPI has no universal framing. Decoding into registers or messages is always a profile-specific interpretation.

Active SPI operation is high risk because the host becomes bus controller and may contend with the target SoC. It requires explicit `interactive` approval and a wiring/profile review.

### I²C/SMBus

Passive observation records SDA, SCL, voltage, pull-up arrangement, and sampling rate. Active access must account for open-drain signaling, existing controllers, clock stretching, and address conflicts. Do not add pull-ups blindly.

### GPIO, reset, boot-select, PWM, and clocks

Treat each line as a typed signal with voltage, direction, active level, bias, and safe-state metadata. Driving reset or boot-mode pins is an active control operation even if no protocol decoder is involved.

### SWD and JTAG

Debug ports can halt CPUs, alter memory, bypass normal boot policy, and expose secrets. Discovery may be passive, but attach/halt/read/write operations require a separate debug authority and target ownership confirmation. Captures and dumps need secret review before retention.

### Logic-analyzer and oscilloscope evidence

A logic analyzer produces sampled digital transitions; an oscilloscope produces analog measurements. Both should fit the same session/evidence envelope while preserving their native files. Exported CSV, VCD, Sigrok, screenshots, and decoder text are derived artifacts.

## Evidence Bundle v1

A future implementation should emit a self-contained directory:

```text
hardware-evidence/<target>/<session-id>/
├── session.toml
├── connection.toml
├── adapter.toml
├── raw/
│   ├── capture.bin
│   └── capture.bin.sha256
├── derived/
│   ├── transcript.txt
│   ├── decode.jsonl
│   └── annotations.md
├── photos/
│   └── README.md
└── review.md
```

Minimum `session.toml` shape:

```toml
schema_version = 1
session_id = "rg35xxsp-oem-cold-boot-01"
target = "operator-rg35xxsp-a"
interface = "uart"
mode = "observe-only"
stimulus = "cold-boot"
control_role = "known-good"
started_at = "<RFC3339>"
ended_at = "<RFC3339>"
clock = "host-monotonic+wall"
raw_capture = "raw/capture.bin"
raw_sha256 = "<sha256>"
termination = "operator-stop"

[parameters]
baud = 115200
data_bits = 8
parity = "none"
stop_bits = 1
flow_control = "none"
```

The schema must support unknown values. Discovery evidence should say `unknown`, not invent certainty.

## Control and Differential Evidence

Hardware bring-up should pair experimental captures with a known-good control whenever possible.

For the RG35XXSP:

1. Capture a cold boot with the preserved OEM card.
2. Confirm the selected pad, voltage, baud rate, and readable UART stream.
3. Capture the Styrene card with identical wiring and parameters.
4. Capture reset separately from cold boot.
5. Compare the earliest divergent line or missing stage.

This prevents “no output” from being misclassified when the real problem is wiring, baud rate, or adapter selection.

## RG35XXSP UART Runbook

### Equipment

- USB-to-TTL adapter explicitly capable of the measured target logic voltage;
- multimeter;
- fine probes, pogo pins, or micro-grabbers;
- optional logic analyzer or oscilloscope;
- magnification and nonconductive work surface.

### Board preparation

1. Power off, remove cards, and disconnect external cables.
2. Open without stressing battery or display ribbons.
3. Photograph the full PCB and candidate test-pad groups.
4. Disconnect the battery before soldering.
5. With power removed, identify ground by continuity to USB/HDMI shield or battery negative.
6. Reconnect battery and measure candidate signal idle voltages.
7. Prefer a logic analyzer to identify a pad with power-on bursts.

Published RG35XX-family pinouts are leads, not proof for this physical SP board. Record this board revision and validate pads electrically.

### Initial wiring

Use receive-only wiring:

```text
RG35XXSP GND → adapter GND
RG35XXSP TX  → adapter RX
```

Leave adapter TX and all adapter power pins disconnected.

### Host discovery on macOS

Before and after inserting the adapter:

```bash
ls /dev/cu.*
system_profiler SPUSBDataType
```

Probable paths include `/dev/cu.usbserial-*`, `/dev/cu.SLAB_USBtoUART`, and `/dev/cu.wchusbserial*`.

### Capture parameters

Start with `115200 8N1`, no flow control, because the candidate kernel command line uses `console=ttyS0,115200`. This does not prove that BootROM or U-Boot uses that rate. If the OEM control produces no readable stream after wiring is validated, investigate `1500000`, `921600`, `57600`, and `38400` as explicit alternate profiles.

Start logging before applying target power. Capture raw bytes to a file; do not rely only on a terminal scrollback.

Preferred tool behavior:

- open device read-only for observe-only sessions;
- configure termios explicitly;
- disable echo and software/hardware flow control;
- flush every captured block;
- report overruns and disconnections;
- retain raw bytes unchanged;
- timestamp session boundaries outside the byte stream unless an instrument provides per-sample timing.

### Required sessions

```text
rg35xxsp-oem-cold-boot-01
rg35xxsp-styrene-cold-boot-01
rg35xxsp-styrene-reset-01
rg35xxsp-styrene-cold-boot-02
```

Record LED, display, card slot, TF2 state, power source, adapter, wiring, measured voltage, and termination reason for each.

### Diagnostic interpretation

- OEM silent: capture setup is unproven; do not infer a Styrene failure.
- OEM readable, Styrene silent: investigate BootROM acceptance, SPL header, DRAM initialization, and bootloader placement.
- SPL output then failure: inspect DRAM/PMIC/clock board support.
- U-Boot output but no boot entry: inspect MMC index, partition scanning, filesystem support, and extlinux search.
- Kernel output with dark display: boot chain works; inspect panel, backlight, regulator, GPIO, and console routing.
- Kernel panic: use the exact panic and preceding mount/device messages as the next acceptance boundary.

## Security and Privacy

Raw hardware captures can contain:

- Wi-Fi credentials;
- device serial numbers;
- MAC addresses;
- cryptographic keys;
- boot arguments;
- filesystem paths;
- identities and message content;
- firmware dumps or proprietary material.

Evidence is private by default. Review before committing or publishing. Keep raw captures in ignored evidence storage unless a sanitized fixture is deliberately promoted. Derived fixtures must retain a pointer to the private source hash without embedding secrets.

Active debug interfaces may bypass operating-system authorization. Styrene must not expose arbitrary UART consoles, SPI transactions, memory reads, or GPIO writes as unattended remote actions.

## Proposed Styrene Components

This is a design direction, not an implemented capability.

### `styrene-hardware-interface` library

Owns transport-neutral types:

- adapter identity and capabilities;
- electrical profile;
- target connection;
- session lifecycle;
- evidence manifest;
- safety mode and authorization decision;
- raw sink/source interfaces;
- decoder provenance.

It should not depend on Reticulum or a UI.

### Host backends

Potential feature-gated backends:

- `serialport`/termios for UART and USB serial;
- libusb for device discovery and class/vendor protocols;
- Sigrok-compatible capture import for logic analyzers;
- Linux `spidev`, `i2c-dev`, and GPIO character-device APIs;
- mock/replay backend for tests.

Direct SPI/I²C/GPIO access is primarily a Linux-appliance capability; macOS workflows will usually use USB instruments.

### `styrene-hw` CLI

Candidate commands:

```text
styrene-hw adapters list
styrene-hw profile validate PROFILE
styrene-hw capture start --profile PROFILE --session SESSION
styrene-hw capture inspect SESSION
styrene-hw decode SESSION --decoder DECODER
styrene-hw compare CONTROL EXPERIMENT
styrene-hw replay SESSION
```

There should be no generic `write` command in the first slice.

### Forge and flasher integration

Flasher contracts should reference required evidence sessions and checks, not shell-specific capture commands. Example:

```toml
[[acceptance.evidence]]
interface = "uart"
profile = "rg35xxsp-console-v1"
control_session = "oem-cold-boot"
required_markers = ["U-Boot", "Linux version", "stage-2"]
```

Forge advances lifecycle only after the referenced evidence validates. It must not infer hardware success from a process exit code or a non-empty transcript.

## Implementation Sequence

### Phase 1 — Observation-only UART

- Define v1 adapter, connection, session, and evidence schemas.
- Discover serial adapters safely.
- Capture raw UART bytes receive-only.
- Hash evidence and emit manifests.
- Add replay and text-derivation tests.
- Use OEM-versus-Styrene RG35XXSP captures as the first acceptance fixture.

### Phase 2 — Passive digital buses

- Import Sigrok/VCD captures.
- Represent SPI, I²C, GPIO, and clock channel maps.
- Add profile-driven decoding without active bus control.

### Phase 3 — Guarded interaction

- Add explicit authorization records.
- Permit bounded UART transmit and selected reset/boot controls.
- Add voltage/profile checks and adapter capability enforcement.
- Keep target power control separately authorized.

### Phase 4 — Product clients

- Migrate known hardware clients, where useful, onto approved backends.
- Connect accepted evidence to Forge/flasher lifecycle gates.
- Add remote operation only with RBAC, operator presence rules, audit logs, and bounded recipes.

## Decisions and Non-Decisions

Decided:

- Raw evidence and interpretation are separate artifacts.
- Observe-only is the default.
- Electrical profile and physical wiring are first-class data.
- Control sessions are required for uncertain bring-up paths.
- Hardware observation is not the same abstraction as product/network transport.
- Delivery and hardware lifecycle gates consume evidence references rather than terminal prose.

Not yet decided:

- Canonical Rust crate/API shape;
- exact on-disk schema serialization beyond the v1 proposal;
- adapter allowlist and voltage-verification mechanism;
- Sigrok/libusb dependency policy;
- whether per-byte timestamps are required for UART;
- remote-operation policy and RBAC capabilities;
- which SPI/I²C/GPIO active operations, if any, belong in the maintained product.

These unknowns should be resolved before implementing active control. They do not block the receive-only UART evidence slice.
