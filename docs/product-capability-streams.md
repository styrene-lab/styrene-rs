+++
id = "5dc53189-e2e3-44fb-ba23-e95b7b053f0b"
kind = "design_node"

[data]
title = "Styrene Product Capability Streams and Deployment Tiers"
status = "exploring"
issue_type = "architecture"
priority = 1
dependencies = []
open_questions = [
  "[assumption] Capability streams are stable cross-surface contracts while named compositions remain replaceable catalog entries",
  "[assumption] Nex consumes a versioned Styrene materialization payload rather than importing Styrene repository internals",
  "[assumption] The first R36S-class target runs Linux and can host the allocator/async stack but requires a purpose-built UI and bounded services",
  "[assumption] NomadNet, I2P, and semantic HTTP can share a page model without hiding substrate-specific identity, trust, and action semantics",
  "Ownership of shared file/resource transfer across messaging, browsing, and media",
  "Whether Device Communicator remains a stream after multiple constrained device families exist",
  "Objective thresholds separating linux-appliance, constrained-linux, and headless-edge",
  "Which EmbeddedAlloc claims hold under current advertised memory budgets",
  "First embedded milestone: embedded Linux, allocator-capable RTOS, or separate milestones",
  "Ownership and versioning of the shared Styrene-Nex payload schema",
]
+++

+++
id = "5dc53189-e2e3-44fb-ba23-e95b7b053f0b"
kind = "design_node"

[data]
title = "Styrene Product Capability Streams and Deployment Tiers"
status = "exploring"
issue_type = "architecture"
priority = 1
dependencies = []
open_questions = []
+++

## Overview

+++
id = "5dc53189-e2e3-44fb-ba23-e95b7b053f0b"
kind = "design_node"

[data]
title = "Styrene Product Capability Streams and Deployment Tiers"
status = "exploring"
issue_type = "architecture"
priority = 1
dependencies = []
open_questions = []
+++

## Overview

# Styrene Product Capability Streams and Deployment Tiers

# Styrene Product Capability Streams and Deployment Tiers

## Overview

Styrene needs a product map that is more durable than a list of applications or a ladder from “small” to “large.” The useful organizing unit is a **capability stream**: a coherent user outcome whose components evolve together. Deployment tiers are a separate axis describing where those streams can run under explicit resource, input, display, storage, and networking constraints.

This distinction prevents two bad groupings:

- NomadNet page browsing, I2P browsing, and a semantic public-web reader belong to a shared **information-access stream**, even though they use different protocols and engines.
- A compact conversation surface for an R36S-class handheld belongs to a **constrained communicator stream**, not to browsing merely because both may run on Linux.

Nex is a third axis: it does not define Styrene product behavior. Nex Forge turns selected capability streams plus a hardware profile into reproducible media and, separately, delivers that media to a device.

The resulting model is:

```text
product stream(s) × deployment tier × hardware profile
                    ↓
          validated capability manifest
                    ↓
       Nex materialization target (build)
                    ↓
         Nex delivery target (flash/write)
```

## Goals

1. Define stable, outcome-oriented product capability streams.
2. Keep deployment/resource tiers orthogonal to product streams.
3. Make capability inclusion explicit and machine-readable rather than inferred from binary names or target triples.
4. Give constrained and embedded systems first-class product shapes rather than treating them as degraded desktop builds.
5. Define the Styrene–Nex boundary for assembling, validating, and flashing device media.
6. Allow one device image to compose multiple compatible streams without creating a combinatorial set of hand-maintained editions.

## Non-Goals

- Naming or branding final commercial editions.
- Selecting every supported board or handheld.
- Defining Nex's internal Forge schema on Styrene's behalf.
- Claiming that all current capabilities work on all tiers.
- Treating compile success as hardware validation.
- Combining deterministic image construction with destructive device writing.
- Replacing runtime profiles (`Standard`, `Portable`, `Ghost`, `PortableGhost`), which describe persistence and lifecycle rather than product capability.

## Axes of Composition

### Axis A — Product capability stream

A stream owns a coherent user outcome, its domain contracts, and its acceptance evidence. Streams may share substrate libraries but should not be grouped merely because they share a UI or executable.

### Axis B — Deployment tier

A tier defines the execution envelope: operating system, allocator, memory, storage, display, input, concurrency, and background-service assumptions. A tier is not a feature ranking.

### Axis C — Hardware profile

A hardware profile records board/device facts and affordances: architecture, display geometry, controls, radios, storage medium, power behavior, and writable-device identity. Styrene consumes those facts as constraints; Nex owns their reproducible system/image expression.

### Axis D — Runtime profile

Runtime profiles remain independent: standard versus portable persistence, persistent versus Ghost identity, and embedded versus external runtime host. For example, a constrained communicator can be persistent or Ghost; an information-access workstation can be portable.

## Initial Capability Streams

### 1. Conversations and Asynchronous Messaging

**Outcome:** exchange dependable identity-addressed messages under intermittent connectivity.

**Includes:**

- identity and contacts;
- LXMF send/receive and delivery state;
- conversation history and receipts;
- compact text composition;
- store-and-forward/propagation behavior;
- paper/offline message affordances where supported;
- bounded attachments as a separately advertised capability.

**Surfaces:** full TUI/DX conversations, compact handheld chat, headless service integration.

**Tier posture:** universal baseline from desktop through constrained Linux; a reduced protocol profile may reach allocator-capable embedded systems. The R36S-class product begins here, with device-native navigation and a deliberately small interaction footprint.

### 2. Information Access and Page Browsing

**Outcome:** discover, retrieve, navigate, and read information across heterogeneous page systems through one semantic interaction model.

**Includes:**

- NomadNet page browsing over RNS/LXMF;
- I2P site/service browsing when an I2P substrate is available;
- semantic public-web reading through a bounded engine such as Lightpanda;
- page history, links, forms, bookmarks, content metadata, and trust/substrate indicators;
- protocol-specific fetch adapters projecting into a shared semantic page model.

**Why these belong together:** the user task is browsing and reading. NomadNet, I2P, and HTTP are acquisition substrates, not separate product families. Their trust, latency, addressing, and interaction affordances remain visible rather than flattened away.

**Tier posture:** desktop/full Linux first; possible kiosk or SBC reader tier. Not assumed for constrained handheld chat or allocator-only embedded targets because browser engines, I2P routers, page models, and caches have materially different resource demands.

### 3. Live Media and Session Communications

**Outcome:** negotiate and carry live or near-live human communication over an available high-bandwidth substrate.

**Includes:**

- voice signaling over Styrene/RNS control paths;
- media carriage over established Styrene tunnels or suitable overlay substrates;
- push-to-talk, voice notes, and live voice as distinct service levels;
- codec/device adaptation and session telemetry;
- future video only as a separately proven capability.

**Tier posture:** desktop, capable SBC, and purpose-built media appliances. Constrained devices may support PTT or voice-note subsets without inheriting full live-media requirements.

### 4. Mesh Node, Relay, and Gateway Operations

**Outcome:** operate and observe the network rather than merely consume it.

**Includes:**

- embedded or service-hosted Styrene runtime;
- routing, propagation, relay, and interface lifecycle;
- TCP, local, UDP/AutoInterface, I2P, serial/KISS/RNode, Meshtastic, BLE, or future substrate adapters as independently evidenced capabilities;
- node health, topology, diagnostics, and bounded remote operations;
- gateway roles between explicitly permitted substrates.

**Tier posture:** headless server, edge box, SBC, and embedded node. A gateway image can compose this stream with conversations or information access, but those are not intrinsic to routing.

### 5. Fleet, Provisioning, and Field Operations

**Outcome:** inspect, configure, update, recover, and attest deployed Styrene nodes.

**Includes:**

- hardware and runtime inventory;
- remote status and approved command execution;
- configuration deployment and rollback;
- release/artifact verification;
- field diagnostics and air-gapped handoff;
- identity-safe replacement and recovery procedures.

**Tier posture:** operator workstations and dedicated field-diagnostics media. Managed nodes expose only a constrained agent capability, not the whole operator surface.

### 6. Device Communicator

**Outcome:** provide a focused, appliance-like communication experience on small displays and gamepad/keypad-driven devices.

**Includes:**

- compact conversation list and thread view;
- contacts and identity status;
- deterministic focus/navigation vocabulary;
- low-background-work operation;
- suspend/resume and removable-storage resilience;
- optional PTT where hardware and media capabilities are proven.

**Explicit exclusions by default:** semantic web engine, I2P router, full topology visualization, fleet console, general page browsing, and unrestricted terminal functionality.

**Why this is a stream rather than merely a tier:** the interaction model and product outcome are distinct. Shrinking the desktop UI would preserve the wrong hierarchy and background assumptions. R36S is an initial hardware candidate, not the stream's name or universal baseline.

### 7. Embedded Application Substrate

**Outcome:** let firmware or tightly bounded native applications use Styrene communication primitives without carrying a desktop product stack.

**Includes:**

- allocator-capable SDK profile with manual tick;
- explicit memory/event/attachment budgets;
- identity, messaging, topics, telemetry, and selected remote-command primitives;
- storage adapters including NOR flash where implemented;
- transport adapters selected per board;
- no implicit filesystem, process, terminal, browser, or daemon assumptions.

**Tier posture:** embedded Linux at the upper boundary, then allocator-capable RTOS/bare-metal targets where the crate graph proves compatible. A future no-alloc tier is separate and must not be implied by `EmbeddedAlloc`.

## Initial Deployment Tiers

| Tier | Execution envelope | Typical surfaces | Candidate streams |
|---|---|---|---|
| `desktop-full` | Desktop OS, ample memory/storage, full async runtime | DX, full TUI | all operator-facing streams |
| `linux-appliance` | Linux userland, bounded services, framebuffer/terminal optional | kiosk, gateway, field console | browsing, communicator, media, node/gateway |
| `constrained-linux` | Small ARM/x86 device, tens to low hundreds of MiB, small display/gamepad | compact communicator | conversations, selected PTT, diagnostics-lite |
| `headless-edge` | Linux SBC/edge box, no local interactive UI required | service/agent | node/gateway, fleet agent, telemetry |
| `embedded-alloc` | allocator available, manual tick, strict static budgets, no process model assumed | firmware API | messaging/telemetry subsets |
| `embedded-core` | future no-alloc/static-memory envelope | firmware API | not yet claimed |

Tiers state ceilings and required evidence. They do not automatically enable a stream. For example, `constrained-linux` can technically run an I2P binary on some devices, but that does not make I2P browsing part of the communicator product without resource and UX evidence.

## Capability Manifests

Each buildable product composition should resolve to a capability manifest with stable identifiers rather than a marketing edition name. A conceptual shape is:

```toml
schema_version = 1
product = "styrene"
composition = "field-communicator"
deployment_tier = "constrained-linux"

streams = ["conversations", "device-communicator"]
capabilities = [
  "identity.persistent",
  "messaging.lxmf.direct",
  "messaging.lxmf.propagated",
  "ui.compact.gamepad",
]

[requirements]
min_memory_bytes = 67108864
persistent_storage_bytes = 268435456
requires_allocator = true
requires_process_model = true

[exclusions]
capabilities = ["browse.semantic-web", "browse.i2p", "fleet.operator"]
```

The concrete schema should reuse existing SDK capability identifiers where they accurately describe protocol/runtime behavior. Product-level capabilities such as page browsing or compact gamepad UI need their own namespace. A resolved manifest must distinguish:

- **required** — absence makes the composition invalid;
- **supported** — included and tested but optional to the primary outcome;
- **excluded** — deliberately absent, not accidentally missing;
- **experimental** — present without release-grade support evidence.

## Styrene–Nex Forge Boundary

### Styrene owns

- capability-stream definitions and composition rules;
- tier resource requirements and runtime constraints;
- product binaries/libraries and default configuration;
- application-level persistence and identity semantics;
- release archive and runtime acceptance checks;
- stream/tier conformance tests;
- payload metadata describing what must be installed and enabled.

For an RNode product operation, Styrene also owns exact target admission, signed
firmware manifests, immutable plans, RNode provisioning semantics, product
confirmation, and authoritative product-level verification.

### Nex owns

- hardware inventory and hardware-purpose profiles;
- Nix/system configuration and reproducible image construction;
- composition of Styrene payloads with OS, boot, drivers, codecs, I2P/router packages, and device services;
- materialization targets such as `toplevel`, `sd-image`, `raw-image`, `iso-image`, or future firmware artifacts;
- artifact checks and build evidence;
- delivery targets such as file output, USB, SD card, removable block device, or supported programmer;
- destructive-write safety, target-device validation, confirmation, and post-write verification.

For RNode provisioning, the last item means low-level device safety and typed
delivery evidence. Styrene remains responsible for product authorization,
plan-bound operator confirmation, and the decision that an RNode is verified.

### Shared contract

The handoff should be a versioned **materialization payload**, not a shell script hidden in either repository. It should identify:

- Styrene release artifact and digest;
- resolved stream/capability manifest;
- deployment tier;
- hardware-profile requirements;
- service/runtime profile defaults;
- persistent and ephemeral partitions/paths;
- required external substrate packages (for example I2P or codecs);
- product acceptance commands (`styrene doctor`, future Ghost/runtime checks);
- post-boot health and hardware validation expectations.

Forge must preserve the distinction already identified in Nex:

```text
materialize artifact → validate artifact → deliver artifact → validate hardware
```

Building an SD image is deterministic materialization. Writing it to `/dev/...` is a separate, destructive delivery action. Booting an R36S and proving controls, display, suspend, networking, and audio is hardware validation and cannot be inferred from a successful image build.

### Exact RNode Provisioning Contract

RNode firmware provisioning is a narrow product exception to the general Forge
delivery split. Styrene can retain an exact bounded executor adapter until Nex
provides a versioned provisioning API. The adapter must accept only a
Styrene-admitted immutable plan for one declared target and executor class. It
must not expose arbitrary commands, device paths, images, offsets, erase ranges,
reset sequences, or programmer options.

The long-term handoff is typed and bidirectional:

```text
Styrene admitted plan -> Nex bounded provisioning API -> typed execution evidence
```

Nex owns generic discovery, bootloader entry, reset and programmer control, byte
delivery, and low-level reads. Styrene owns artifact selection, writable-region
policy, product confirmation, RNode metadata semantics, recovery presentation,
and verified product success. Normal RNode serial/KISS and BLE NUS runtime
clients remain separate from provisioning sessions.

## Candidate Compositions

| Composition | Streams | Tier | Likely Nex materialization |
|---|---|---|---|
| Full workstation | conversations, information access, media, fleet/operator | `desktop-full` | `toplevel` / release archive |
| Semantic field reader | information access, conversations | `linux-appliance` | `sd-image` or `raw-image` |
| R36S-class communicator | conversations, device communicator, optional PTT | `constrained-linux` | device-specific `sd-image` |
| Mesh gateway | node/gateway, fleet agent, optional information cache | `headless-edge` | SBC `sd-image` / `raw-image` |
| Field diagnostics kit | fleet/operator, conversations, air-gap handoff | `linux-appliance` | bootable image |
| Embedded telemetry endpoint | embedded substrate subset | `embedded-alloc` | firmware/library artifact; flashing backend depends on board |
| Media appliance | media, conversations, node/gateway as needed | `linux-appliance` or `headless-edge` | device image |

These are compositions, not permanent editions. Their names may change without changing stable stream and capability identifiers.

## Dependency and Compatibility Rules

1. Streams depend on capabilities, never directly on UI applications.
2. Acquisition substrates remain visible in page/media models; a common UX does not erase protocol trust boundaries.
3. A stream may require another stream's low-level capability without inheriting its presentation. For example, media uses conversation signaling but need not ship the full conversations UI.
4. Hardware profiles can constrain or forbid capabilities but cannot silently add product behavior.
5. Tier limits are test inputs and runtime enforcement inputs, not documentation-only estimates.
6. Every materialized composition must expose its resolved manifest on-device.
7. Compile/build evidence, virtual execution evidence, and physical hardware evidence are reported separately.
8. Destructive delivery is never an implicit side effect of building.

## Evidence Model

Each `(stream, tier, hardware profile)` claim progresses independently:

1. **Declared** — manifest resolves without contradiction.
2. **Built** — binaries/image materialize reproducibly.
3. **Artifact-validated** — contents, signatures/digests, configuration, and static limits pass.
4. **Virtually exercised** — runtime behavior passes on a representative VM/emulator where meaningful.
5. **Hardware-validated** — boots and passes device-specific input/display/radio/audio/power checks.
6. **Field-validated** — survives realistic intermittent connectivity, suspend, storage, and recovery scenarios.

Product documentation must not collapse these into a single “supported” flag.

## Initial Sequencing

### Phase 1 — Vocabulary and manifests

1. Stabilize stream IDs and product-capability namespaces.
2. Map current crates, CLI methods, TUI/DX surfaces, and SDK capabilities into streams.
3. Define the tier constraint schema and evidence states.
4. Create two reference compositions:
   - full workstation;
   - constrained communicator.

### Phase 2 — Information-access vertical slice

1. Define a shared semantic page model.
2. Project current NomadNet page behavior into it.
3. Add substrate/trust indicators and navigation history.
4. Prototype read-only semantic HTTP through Lightpanda.
5. Add I2P acquisition only through an explicit router/proxy capability.

### Phase 3 — Constrained communicator vertical slice

1. Choose an emulator/test viewport and input map before binding to R36S hardware.
2. Implement compact conversations and contact navigation.
3. Enforce a constrained background/memory profile.
4. Materialize a device image through Nex.
5. Validate boot, controls, display, suspend, storage, and networking on hardware.

### Phase 4 — Forge payload integration

1. Publish versioned Styrene materialization payload metadata.
2. Add Nex hardware-purpose profile(s) for the first communicator and gateway devices.
3. Build artifact-only targets first.
4. Add explicit `write-image` delivery after target-device safety is implemented.
5. Feed post-build and post-boot evidence back into the composition manifest/report.

### Phase 5 — Embedded substrate

1. Audit the current `EmbeddedAlloc` capability claims against the actual crate graph.
2. Select one board/RTOS or embedded-Linux boundary as the first target.
3. Add static budget and manual-tick conformance tests.
4. Define firmware materialization separately from Linux disk-image materialization.
5. Add generic board flashing only through a safe Nex delivery backend. Keep the exact RNode product exception constrained by its contract above.

## Open Questions

- [assumption] Capability streams should be stable cross-surface product contracts, while named compositions remain replaceable catalog entries.
- [assumption] Nex will consume a versioned Styrene materialization payload rather than importing Styrene repository internals.
- [assumption] The R36S-class first target runs Linux and can host the existing allocator/async stack, but requires a purpose-built UI and bounded services.
- [assumption] NomadNet, I2P, and semantic HTTP can project into one page model without hiding substrate-specific identity, trust, and action semantics.
- Which stream owns file/resource transfer when it serves both messaging attachments and page/media content: conversations, information access, or a shared content substrate?
- Should `Device Communicator` remain a product stream or become a presentation profile once two constrained device families prove the same interaction contract?
- What objective thresholds separate `linux-appliance`, `constrained-linux`, and `headless-edge`?
- Which existing SDK `EmbeddedAlloc` capabilities are genuinely usable under the advertised 8 MiB heap and 2 MiB event-queue budgets?
- Does the first embedded target mean embedded Linux, RTOS with allocation, or both as separate milestones?
- Which repository owns the shared payload schema and compatibility tests: Styrene, Nex, or a small versioned schema crate?
- How should Nex report hardware validation evidence back to Styrene release metadata without coupling release lifecycles?

## Immediate Design Target

The next bounded design slice is to create the **capability registry and two resolved reference manifests**—`full-workstation` and `constrained-communicator`—then map existing implementation surfaces to them. This will test whether the stream/tier separation is useful before committing to device images or broad Forge integration.

## Open Questions

## Open Questions
