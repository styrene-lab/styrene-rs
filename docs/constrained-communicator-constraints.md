+++
id = "constrained-communicator-constraints"
kind = "design_node"

[data]
title = "Constrained Communicator Constraint Discovery"
status = "exploring"
issue_type = "research"
priority = 1
dependencies = ["5dc53189-e2e3-44fb-ba23-e95b7b053f0b"]
open_questions = [
  "[assumption] The first reference device exposes a Linux userspace and stable evdev inputs",
  "[assumption] The R36S unit is a standard/original family device with a panel 4 display and V22 board, based only on seller reviews and circumstantial evidence pending physical or software identification",
  "[assumption] The first reference image can reserve at least 64 MiB RAM and 256 MiB persistent storage for the complete Styrene composition",
  "[assumption] Text entry without a hardware keyboard can be made usable without shipping a general desktop environment",
  "[assumption] Suspend/resume is a required product behavior rather than reboot-on-use",
  "[assumption] The R36S unit in hand is a standard/original Panel 4 V22 board, based on seller reviews and circumstantial evidence; this is a temporary planning hypothesis pending physical/self-reporting confirmation",
  "[assumption] The RG35XXSP is the first reference candidate because it has integrated Wi-Fi, mini-HDMI diagnostics, and a Hall lid switch",
  "Which exact R36S board, panel, DTB, battery, and clone/genuine family does the unit in hand use?",
  "Which exact RG35XXSP board revision, stock firmware, boot chain, and kernel interfaces does the unit in hand use?",
  "What RAM remains available after the selected OS, graphics stack, audio services, and networking are resident?",
  "What display viewport, DPI, minimum readable cell size, and safe-area constraints must the UI satisfy?",
  "Which controls are exposed through evdev, and are there stable mappings across hardware revisions?",
  "What text-entry mechanism is acceptable for recipient search and message composition?",
  "What boot-to-interactive and resume-to-interactive latency budgets are required?",
  "What idle, sync, compose, and message-burst memory/CPU/power ceilings are acceptable?",
  "What retention, attachment, database-growth, and removable-storage write budgets apply?",
  "What constitutes successful recovery after power loss or storage removal during a write?",
  "Which network loss, latency, and partition scenarios are release-gating?",
  "Are audio capture/playback and push-to-talk in the first milestone or a later composition?",
  "Which measurements can run under emulation, and which require physical hardware?",
  "Can the RG35XXSP boot stack be redistributed and materialized by Nex with acceptable source/provenance guarantees?",
  "Which external networking substrate, if any, is acceptable for the R36S target?",
  "Does Bluetooth on the RG35XXSP support required non-controller profiles under the selected OS?",
]
+++

## Overview

The `constrained-communicator` composition is declared and registry-valid, but it is not ready for implementation. Its current tier ceilings are provisional, its reference hardware is not pinned, and the interaction model has not been tested against the actual display, controls, text-entry path, operating-system footprint, suspend behavior, or storage failure modes.

This node defines the discovery work required before implementing `ui.compact.gamepad`, creating a Styrene-to-Nex materialization payload, building a device image, or writing media. The output is a set of measured budgets and acceptance fixtures, not application code.

Parent architecture: `docs/product-capability-streams.md`.

## Decision Gate

Implementation may begin only when all release-gating constraints below have:

1. a named measurement method;
2. a numeric or enumerated acceptance threshold;
3. an evidence owner and storage location;
4. a classification as host-testable, image-testable, or physical-hardware-only;
5. no unresolved assumption that can invalidate the selected architecture.

The existing manifest values remain hypotheses until this gate is satisfied.

## Constraint Map

### 1. Reference hardware identity

Record, without generalizing across untested clones:

- manufacturer/model and board revision;
- SoC, architecture, cores, clocks, and thermal behavior;
- installed and usable RAM;
- panel resolution, orientation, DPI, refresh behavior, and overscan/safe area;
- control inventory, evdev identifiers/codes, chords, repeat behavior, and power controls;
- storage media, bus, expected capacity, filesystem support, and removal behavior;
- Wi-Fi/Bluetooth/radio chipsets and driver/firmware status;
- speaker, headphone, microphone, codec, and mixer paths if media is in scope;
- battery reporting, charge behavior, suspend states, wake sources, and hard-power semantics;
- serial/debug/recovery access.

**Evidence:** immutable hardware profile plus captured probes (`/proc`, `/sys`, `evtest`, DRM/fb, ALSA, network, storage) from each claimed revision.

### 2. Operating-system and service envelope

Measure the intended Nex-built base before Styrene starts:

- kernel, libc, init, graphics/terminal stack, input stack, network manager, audio stack;
- resident memory and process count at idle;
- writable filesystem layout and read-only candidates;
- boot chain and update/recovery mechanism;
- clock/time synchronization behavior;
- entropy availability at first boot;
- service supervision and crash-loop policy;
- log retention and rotation;
- dynamic-library and external-package requirements.

**Gate:** reserve a measured application budget rather than assigning all physical RAM/storage to Styrene.

### 3. Runtime budgets

For each scenario, capture peak RSS/PSS where available, CPU time/load, task/thread count, file descriptors, wakeups, bytes written, and elapsed latency:

| Scenario | Required measurements | Threshold status |
|---|---|---|
| cold boot → interactive | elapsed time, peak memory, failures | unresolved |
| resume → interactive | elapsed time, reconnect behavior | unresolved |
| idle connected | memory, CPU, wakeups, power proxy | unresolved |
| offline idle | retry cadence, memory, wakeups | unresolved |
| open 1k-message thread | latency, memory, render stability | unresolved |
| compose/send | input latency, persistence latency | unresolved |
| receive burst | queue growth, UI latency, loss behavior | unresolved |
| propagation sync | bandwidth, memory, storage growth | unresolved |
| storage nearly full | error behavior, data integrity | unresolved |
| forced power loss | recovery time and corruption outcome | unresolved |

The provisional manifest ceilings of 64 MiB RAM and 256 MiB storage must be raised, narrowed, or rejected based on these measurements. They are not acceptance criteria yet.

### 4. Persistence and lifecycle

Resolve:

- persistent versus Ghost support in the first milestone;
- identity location, backup/export, and replacement-device recovery;
- database journaling and synchronization policy for removable flash;
- maximum retained conversations/messages and pruning behavior;
- attachment support and size/count ceilings;
- atomicity boundaries for config, identity, preferences, and updates;
- behavior on read-only filesystem, ENOSPC, I/O error, unclean shutdown, and card removal;
- whether suspend keeps the runtime alive or performs an orderly stop/restart;
- expected behavior when system time moves backward or is initially unknown.

### 5. Network and delivery behavior

Define a test matrix for:

- no network at boot;
- delayed network appearance;
- repeated link loss;
- high latency and low bandwidth;
- duplicate, delayed, and reordered delivery;
- propagation-node reachability;
- Wi-Fi credential provisioning without keyboard input;
- interface selection and status communication;
- optional USB, Bluetooth, serial, LoRa/RNode, or phone-tethered substrates.

Browsing, general I2P routing, topology visualization, and fleet control remain excluded. Their services must not consume image or runtime budget accidentally.

### 6. Interaction contract

Map physical inputs to semantic actions, never directly to view-specific mutations:

| Product intent | Semantic action | Required input property |
|---|---|---|
| move selection | `MoveUp` / `MoveDown` | repeatable directional control |
| change region/page | `MoveLeft` / `MoveRight` or focus actions | distinct, reversible control |
| open/confirm/send | `Activate` | guarded against accidental repeat |
| dismiss/return | `Back` | globally consistent |
| compose | `Compose` | reachable from list and thread |
| search/filter | `Search` | text-entry path defined |
| show actions/help | `OpenPalette` / `OpenHelp` | discoverable fallback |

Required task flows:

1. identify node/connectivity state;
2. find a contact or conversation;
3. open a thread and distinguish unread state;
4. read chronological messages and delivery state;
5. compose, edit, cancel, and send text;
6. recover from send failure without losing draft text;
7. return to a known home state;
8. handle first-run identity and network provisioning;
9. communicate offline, syncing, storage-full, and fatal states without a terminal.

The discovery must compare at least two text-entry options under the real control set. Candidate mechanisms include an on-screen keyboard, radial/grouped character selection, phone-assisted composition, or optional USB/Bluetooth keyboard. A mechanism is not selected by aesthetic preference; it must meet measured task-time and error-rate thresholds.

### 7. Display and accessibility envelope

Measure and decide:

- logical viewport and orientation;
- minimum legible font/cell dimensions on the physical panel;
- maximum simultaneous regions;
- line length and thread density;
- color contrast under panel and outdoor limitations;
- focus, unread, delivery, error, and disabled indicators that do not depend solely on color;
- animation and refresh limits;
- long-name, long-message, Unicode, RTL, and oversized-content behavior.

A desktop layout rendered smaller is explicitly not an acceptable prototype.

### 8. Evidence ladder

Every claim advances independently:

1. **Constraint declared** — question and proposed method recorded.
2. **Bench measured** — raw device/OS observations captured.
3. **Budget decided** — threshold and rationale recorded in this node and registry.
4. **Host conformance** — semantic state/action model passes deterministic tests.
5. **Image conformance** — exact materialized image passes static and boot checks.
6. **Hardware conformance** — physical input/display/network/storage/power checks pass.
7. **Field conformance** — intermittent network, suspend, battery, and repeated-use scenarios pass.

Evidence must name the artifact digest, hardware revision, OS/image revision, Styrene revision, measurement command/tool, and result. “Works on R36S” is not sufficient provenance.

## Device Research — 2026-07-15

This section records online research for the two physical devices available to the operator. It distinguishes manufacturer claims, community documentation, and facts that must be probed on the actual units. URLs were accessed on 2026-07-15.

### Source quality

1. **Manufacturer:** ANBERNIC RG35XXSP product specification — <https://anbernic.com/products/rg35xxsp>
2. **Community hardware catalog:** Handhelds Wiki R36S Overview — <https://handhelds.wiki/R36S_Overview>
3. **Community OS/port documentation:** KNULLI RG35XX SP page — <https://knulli.org/devices/anbernic/rg35xx-sp/>
4. **Community firmware source:** AeolusUX ArkOS-R3XS — <https://github.com/AeolusUX/ArkOS-R3XS>

The R36S lacks a reliable single manufacturer specification and is sold under one name with materially different boards, displays, batteries, and clones. Community facts are useful for choosing probes, but the unit in hand is authoritative. ANBERNIC provides a model-level specification for the RG35XXSP, but its software page still does not establish the exact board revision or Linux interfaces of the operator's unit.

### R36S working assumption

Pending conclusive self-reporting or physical inspection, planning may use **standard/original R36S family, panel 4, V22 board** as a provisional hypothesis. This is based on seller-review evidence associated with the purchased item and other circumstantial markers, not a verified unit identity. It may guide which DTB and probe documentation to examine, but it must not authorize flashing, become a Nex hardware-profile fact, or support compatibility claims. Any conflicting runtime, card-image, label, or board evidence supersedes it immediately.

### R36S with two TF slots

**Temporary unit hypothesis:** until self-reporting or physical inspection provides conclusive evidence, planning may treat the operator's unit as a standard/original R36S with a Panel 4 display and V22 board. This is based on seller reviews and circumstantial evidence, not verified hardware identity. It may guide selection of a read-only test image/DTB, but it must not be promoted to a Nex hardware profile, used to overwrite the original cards, or generalized to other R36S units. A failed display boot should invalidate the hypothesis rather than trigger blind DTB replacement.

**Model-family facts reported by Handhelds Wiki:**

- Rockchip RK3326 SoC; Cortex-A35 CPU; Mali-G31 MP2 GPU;
- 1 GiB DDR3L RAM;
- 3.5-inch 640×480 4:3 IPS display;
- Linux OS;
- no internal storage and two microSD slots;
- no integrated Wi-Fi, Bluetooth, or video output on the standard model;
- mono speaker and 3.5 mm audio jack;
- nominal dimensions 130×83×35 mm and weight 187 g.

These align with the operator's observation of two TF slots but do **not** identify the board. The same source documents at least six display variants, V12/V21/V22 and later board variants, clone boards, and changing battery/regulator/GPIO mappings. It explains that the correct device-tree blob is panel/board dependent and that a mismatch commonly produces a black screen. Reported batteries vary by manufacturer; common community reports are removable 3.7 V 3000 or 3200 mAh packs, despite some seller claims of 3500 mAh. Battery telemetry is reported as inaccurate.

**Product implications:**

- Treat this physical R36S as a revision-specific bring-up target, not as evidence for all “R36S” devices.
- Do not build or flash an image before preserving both existing cards and identifying the board/panel/DTB.
- The two-slot topology is valuable: one slot can hold the boot/system medium and the other product data, fixtures, or recovery material, subject to actual mount and boot behavior.
- Standard-model networking requires an external supported USB Wi-Fi adapter or another external substrate. Wi-Fi cannot remain an assumed baseline for this target.
- Lack of video output removes external-display debugging as a normal path.
- Removable storage, uncertain battery telemetry, and hard-power behavior make power-loss and card-integrity tests release-gating.

### ANBERNIC RG35XXSP with mini HDMI

**Manufacturer-confirmed model facts:**

- Allwinner H700, quad-core ARM Cortex-A53 at up to 1.5 GHz;
- dual-core Mali-G31 MP2 GPU;
- 1 GiB LPDDR4 RAM;
- 3.5-inch 640×480 full-view IPS, OCA laminated display;
- 64-bit Linux;
- dual TF/microSD slots, up to 512 GB expansion claimed;
- integrated dual-band 2.4/5 GHz 802.11a/b/g/n/ac Wi-Fi;
- Bluetooth 4.2, with ANBERNIC specifically describing controller use;
- 3300 mAh lithium-polymer battery, manufacturer claim of about eight hours;
- 5 V/1.5 A charging with C-to-C support;
- Hall-effect lid switch/magnetic closure;
- TV output, consistent with the observed mini-HDMI port;
- speaker, vibration, USB controller support, and network multiplayer/streaming claims.

**Community OS facts from KNULLI:**

- identifies the device as Allwinner H700 ARM with Mali-G31;
- its current device page reports an Allwinner BSP 4.9.170 kernel;
- reports wireless, Bluetooth, suspend by brief power-button press, and HDMI support;
- states ANBERNIC had not published the RG35XXSP U-Boot and kernel source used by the stock firmware;
- KNULLI releases therefore include bootloader/U-Boot/kernel binaries extracted from stock firmware, while source-built alternatives may lack elements.

**Product implications:**

- This is the stronger first constrained-communicator candidate: integrated Wi-Fi, a clamshell/Hall lifecycle signal, mini-HDMI for development/diagnostics, 64-bit Cortex-A53, and dual-card storage reduce bring-up uncertainty.
- The Hall switch introduces a first-class close/open policy: close-to-suspend, close-to-lock, or close-to-orderly-stop must be measured rather than assumed.
- HDMI is a useful diagnostic surface but must not become a product requirement; the handheld UI must remain complete on 640×480.
- A vendor BSP 4.9 kernel and redistributed binary boot chain create maintenance, provenance, and reproducibility constraints for Nex. “Image builds” cannot imply a fully source-reproducible boot stack.
- Manufacturer Bluetooth wording does not prove arbitrary BLE peripheral support. Probe kernel config, controller, BlueZ behavior, and profiles before assigning a communication function.
- The manufacturer battery-life figure is a marketing claim, not a Styrene power budget.

### Comparative disposition

| Constraint | R36S unit | RG35XXSP unit |
|---|---|---|
| CPU/RAM class | RK3326 / reported 1 GiB DDR3L | H700 / manufacturer-confirmed 1 GiB LPDDR4 |
| Internal networking | standard model reports none | dual-band Wi-Fi + Bluetooth 4.2 |
| Display | reported 3.5-inch 640×480; panel/DTB varies | manufacturer-confirmed 3.5-inch 640×480 |
| Storage | two microSD; boot topology revision/OS dependent | dual microSD, up to 512 GB claimed |
| External display | reported none | mini-HDMI/TV output |
| Lifecycle sensor | no model-wide lid/suspend signal | Hall lid switch + suspend support reported |
| Reproducibility risk | clones, board/panel/DTB variance | vendor BSP and binary boot-chain dependency |
| Recommended role | secondary compatibility and offline/removable-media target | first discovery/reference candidate |

This is a recommendation, not a support claim. The RG35XXSP should be probed first because it removes networking and diagnostic blockers. The R36S remains valuable precisely because it exercises a harsher offline/external-network and revision-fragmented boundary.

### Required local probes before deciding budgets

Preserve the original cards first. Perform read-only capture from each running stock/community OS where possible:

```text
uname -a
cat /proc/cpuinfo
cat /proc/meminfo
cat /proc/device-tree/model
tr '\0' '\n' </proc/device-tree/compatible
cat /proc/cmdline
lsblk -o NAME,SIZE,TYPE,FSTYPE,LABEL,PARTLABEL,MOUNTPOINTS,RO,MODEL
findmnt
cat /proc/partitions
ip -brief link
rfkill list
lsusb
cat /proc/bus/input/devices
dmesg
```

Capture supporting interfaces when present:

```text
for event in /dev/input/event*; do evtest --query "$event" EV_KEY 2>/dev/null; done
modetest -c -p 2>/dev/null
fbset -i 2>/dev/null
cat /sys/class/graphics/fb0/virtual_size 2>/dev/null
cat /sys/class/power_supply/*/uevent 2>/dev/null
cat /sys/class/drm/*/status 2>/dev/null
aplay -l 2>/dev/null
arecord -l 2>/dev/null
zcat /proc/config.gz 2>/dev/null
```

Also record:

- photographs of labels, ports, card-slot labels, and board/revision markings available without destructive disassembly;
- SHA-256 hashes plus partition tables and full images of both original cards;
- exact current firmware name/version and download source;
- a hardware-interface evidence bundle following `docs/hardware-interface-observation.md`, including adapter identity, measured electrical profile, photographed wiring, raw captures, and control-versus-experiment sessions;
- boot behavior with each card removed independently;
- input events for every button, lid close/open, short power press, and long power press;
- RG35XXSP HDMI hotplug, mode, audio, and lid behavior while HDMI is connected;
- R36S USB-OTG role and tested Wi-Fi dongle chipset, if any.

Do not publish serial numbers, Wi-Fi credentials, private keys, identities, or full unredacted `dmesg`/environment captures. Probe bundles must be reviewed for secrets before entering the repository.

## Discovery Work Packages

### WP1 — Pin and probe one reference device

**Inputs:** one physical reference unit and its currently bootable recovery media.

**Outputs:** hardware revision record, raw probe bundle, photographed/recorded control labels, evdev map, display measurements, recovery procedure. UART/USB-serial and other electrical probe evidence must use the adapter/connection/session/interpretation separation and observe-only default defined in `docs/hardware-interface-observation.md`; a known-good control capture is required before treating silence from an experimental image as boot evidence.

**Stop condition:** if hardware identity or recovery is unreliable, do not establish a product baseline from that unit.

### WP2 — Establish the minimal OS baseline

**Inputs:** reference hardware profile and candidate Nex/system configuration.

**Outputs:** bootable non-Styrene image, idle resource report, service/package inventory, filesystem map, input/display/network/audio availability report.

**No destructive automation:** image construction and artifact checks only until manual target-device selection and recovery are proven.

### WP3 — Interaction experiments

**Inputs:** semantic actions, measured viewport/control map, synthetic conversation fixtures.

**Outputs:** low-fidelity compact state model, virtual-input traces for required flows, two text-entry prototypes, task-time/error observations.

**Exclusion:** no production compact UI and no device-specific input code in view components.

### WP4 — Runtime characterization

**Inputs:** exact Styrene release artifact, isolated state fixtures, baseline image.

**Outputs:** scenario measurements from the runtime-budget table, persistence/failure observations, candidate thresholds.

The existing release-artifact verifier and future bounded Ghost check should supply reusable fixtures; this work must not invent a separate installation path.

### WP5 — Decide budgets and update product manifests

**Inputs:** WP1–WP4 evidence.

**Outputs:** decided numeric budgets, tier corrections, capability status changes, acceptance matrix, and a go/no-go decision for `ui.compact.gamepad` implementation.

Only after WP5 should a Styrene-to-Nex materialization payload be specified.

## Explicit Deferrals

Until the decision gate closes, do not:

- implement an R36S-specific UI;
- add raw gamepad key codes to conversation views;
- claim the provisional 64 MiB/256 MiB limits as supported;
- add `ui.compact.gamepad` to required capabilities;
- define a broad cross-repository Forge schema;
- automate writes to SD cards or block devices;
- include browsing, I2P routing, topology, fleet console, unrestricted terminal, or live media in the communicator image;
- claim embedded/RTOS support from constrained-Linux measurements.

## Relationship to Independent Work

Release archive construction, clean-machine installation, bounded Ghost lifecycle validation, and artifact verification continue independently. They are prerequisites for reliable runtime characterization and later Nex payloads, but they do not depend on choosing communicator hardware or interaction budgets.

## Open Questions

The canonical unresolved questions are tracked in this node's frontmatter. Research should resolve them with evidence, then convert accepted thresholds into decisions and implementation constraints rather than silently deleting them.
