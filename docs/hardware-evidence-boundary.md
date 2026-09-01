# External Hardware Evidence Boundary

## Purpose

Styrene defines **what product and runtime behavior must be proven**. It does not own general hardware adapter discovery, electrical profiles, probe-session orchestration, image construction, block-device delivery, or low-level bus control.

This document defines the narrow contract by which Styrene acceptance can consume externally produced UART, USB-serial, SPI, I²C, GPIO, logic-analyzer, oscilloscope, or debug-port evidence without absorbing those hardware-tooling responsibilities.

The canonical ownership split remains `docs/product-capability-streams.md` under **Styrene–Nex Forge Boundary**.

## Ownership

### Styrene owns

- capability composition and deployment-tier requirements;
- product/runtime configuration and persistence semantics;
- application-level acceptance commands;
- expected runtime markers and failure predicates;
- the requirement for a known-good control where silence is ambiguous;
- sanitized fixtures needed to test a Styrene runtime protocol client;
- references from a product acceptance record to external evidence results.

For RNode firmware operations, Styrene also owns the product-specific contract:

- exact RNode target and operation admission;
- signed firmware manifests and immutable plans;
- declared writable and protected regions;
- operator confirmation bound to the plan and target generation;
- RNode provisioning semantics and authoritative post-write observations;
- product UI and recovery guidance.

### External hardware tooling owns

- adapter inventory and stable hardware identity;
- host device-path discovery;
- electrical compatibility and voltage profiles;
- photographed wiring and target connection records;
- capture-session lifecycle and timestamps;
- raw capture storage, hashing, and chain of custody;
- UART/SPI/I²C/GPIO/SWD/JTAG backends;
- logic-analyzer and oscilloscope integration;
- reset, boot-select, programmer, and power control;
- generic destructive-delivery safety and low-level operator approval;
- hardware profiles, boot chains, system images, generic flashing, and low-level post-write checks.

Nex is the intended owner of these machine/hardware concerns. This repository may retain transitional reference implementations until equivalent Nex contracts exist, but that is incubation evidence, not permanent Styrene product ownership.

### RNode Provisioning Exception

An RNode firmware operation is product behavior, not general image delivery. A
Styrene host can expose this operation only through an exact bounded executor.
The executor must accept an admitted immutable plan. It must not accept an
arbitrary command, executable, device path, image, address, or erase range.

Until a versioned Nex provisioning API exists, this repository can retain an
exact RNode executor adapter. The adapter is transitional and must remain inside
the RNode product boundary. It must not become a generic serial, USB, DFU, reset,
or programmer framework.

When a suitable Nex API exists, Nex owns device discovery, bootloader entry,
reset control, programmer invocation, bounded byte delivery, and low-level
post-write reads. Styrene supplies the admitted plan and validates the returned
typed evidence. Nex must not select firmware, broaden writable regions, infer a
Styrene operation, or report product success.

This exception does not change the ownership of system images, removable-media
delivery, generic embedded firmware, or non-RNode hardware provisioning.

## Runtime Client Exception

Styrene continues to own clients for hardware interfaces that directly implement a Styrene runtime protocol. Existing examples include:

- Reticulum serial/KISS in `styrene-rns`;
- `EmbeddedLinkAdapter` protocol semantics;
- the nRF52840 entropy-coprocessor protocol in `styrene-entropy`.

The boundary is:

```text
external hardware tooling
  discovers, validates, provisions, and assigns an interface
                    ↓
Styrene runtime client
  uses that assigned interface for a known product protocol
```

Styrene runtime clients may define framing, retries, health semantics, and protocol conformance. They must not grow generic adapter inventory, electrical probing, or programmer/flasher responsibilities.

An exact RNode provisioning adapter is not a runtime client. It is allowed only
under the constrained exception above and must remain separate from normal
Reticulum serial/KISS and BLE NUS sessions.

## Evidence Result Contract

Styrene acceptance consumes a reference to evidence, not the capture backend itself. A result must provide at least:

```toml
schema_version = 1
evidence_id = "<stable external identifier>"
kind = "serial-console"
target = "<hardware identity or profile>"
stimulus = "cold-boot"
control_role = "experiment"
raw_digest = "sha256:<digest>"
produced_by = "<tool and version>"
started_at = "<RFC3339>"
ended_at = "<RFC3339>"
termination = "<reason>"

[interpretation]
decoder = "<name and version>"
source_digest = "sha256:<same raw source>"
markers = ["<observed marker>"]
first_failure = "<marker or unknown>"
confidence = "<measured|inferred|unknown>"
```

When a control is required, the result also references it:

```toml
[control]
evidence_id = "<known-good session>"
raw_digest = "sha256:<digest>"
relationship = "same-target-same-wiring-same-parameters"
```

The external tool may use a richer schema. Styrene needs only a stable projection carrying integrity, provenance, control relationship, and product-relevant observations.

## Acceptance Semantics

A product acceptance declaration may require external predicates such as:

```toml
[[acceptance.external_evidence]]
kind = "serial-console"
control = "known-good-required"
required_markers = ["U-Boot", "Linux version"]
required_product_markers = ["styrene-ready"]
forbidden_markers = ["Kernel panic"]
```

Styrene must not infer success from:

- a non-empty transcript;
- a capture process exiting successfully;
- an image having been built or flashed;
- LED behavior alone;
- the absence of an error marker in an incomplete capture.

A result is acceptable only when its provenance and control requirements are satisfied and the declared predicates are observed.

## RG35XXSP Current Evidence Requirement

The current bring-up needs serial-console evidence to distinguish early boot failure from a working boot chain with a dark display.

Required sessions:

```text
rg35xxsp-oem-cold-boot-01
rg35xxsp-styrene-cold-boot-01
rg35xxsp-styrene-reset-01
rg35xxsp-styrene-cold-boot-02
```

The OEM session is the known-good control and must establish that the same physical target, adapter, wiring, voltage domain, and serial parameters produce a readable stream. Until that control succeeds, silence from the Styrene card is not boot evidence.

Styrene-relevant observations are the earliest markers among:

```text
BootROM/SPL output
DRAM initialization
U-Boot banner
MMC/partition discovery
extlinux selection
Linux version
kernel command line
root filesystem mount
NixOS stage 1
NixOS stage 2
styrene runtime readiness
panic, reset, or power-off
```

The manual procedure for collecting the first evidence remains summarized in `docs/rg35xxsp-uart-runbook.md`. It is a bring-up aid, not a Styrene hardware-tooling API.

## Transitional Repository Assets

The following current assets cross into the eventual Nex ownership boundary:

```text
product/flashers/*.toml
scripts/flash-rpi4-image.sh
scripts/validate_flashers.py
scripts/verify-rg35xxsp-bringup-image.sh
nix/make-rg35xxsp-image.nix
nix/hardware/rg35xxsp/
```

They are retained because they provide working reference contracts and physical evidence. Future work should:

1. avoid expanding them into a general Styrene hardware framework;
2. describe their role as reference/incubation implementations;
3. extract versioned handoff requirements rather than shell command coupling;
4. migrate hardware profiles, image construction, delivery, and probe orchestration to Nex;
5. leave Styrene with capability payloads, runtime artifacts, and acceptance predicates.

## Explicit Non-Goals

This repository will not introduce:

- a `styrene-hardware-interface` general-purpose crate;
- a `styrene-hw` adapter/probe CLI;
- generic SPI, I²C, GPIO, SWD, or JTAG host backends;
- generic UART capture-session orchestration;
- voltage-profile or wiring-authority management;
- target power or programmer control;
- unattended remote hardware writes.

It will also not expose arbitrary device paths, commands, images, offsets, erase
ranges, reset sequences, or programmer options through an RNode executor.

A Styrene runtime protocol client remains valid where it directly serves product behavior. General physical hardware lifecycle tooling belongs outside Styrene.
