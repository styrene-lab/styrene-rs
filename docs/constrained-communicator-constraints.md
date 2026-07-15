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
  "[assumption] The reference image can reserve at least 64 MiB RAM and 256 MiB persistent storage for the complete Styrene composition",
  "[assumption] Text entry without a hardware keyboard can be made usable without shipping a general desktop environment",
  "[assumption] Suspend/resume is a required product behavior rather than reboot-on-use",
  "[assumption] Wi-Fi is the baseline network substrate; device-integrated LoRa is optional",
  "Which exact R36S hardware revision or equivalent reference device is authoritative?",
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

## Discovery Work Packages

### WP1 — Pin and probe one reference device

**Inputs:** one physical reference unit and its currently bootable recovery media.

**Outputs:** hardware revision record, raw probe bundle, photographed/recorded control labels, evdev map, display measurements, recovery procedure.

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
