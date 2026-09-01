# Complete Mobile Product Workflows Design

## Ownership

The canonical OpenSpec remains in `styrene-rs`, but implementation is split by
authority rather than by screen:

| Concern | Owning repository | Boundary |
|---|---|---|
| Runtime phases, message attempts, receipts, route and bearer evidence, retry eligibility | `styrene-rs` | Additive typed mobile/session DTOs and operations |
| Propagation selection, readiness, synchronization, cooldown, and trigger policy | `styrene-rs` | No wall-clock polling owned by a frontend |
| Identity custody and encrypted backup/restore | `styrene-rs` | Secret material never enters presentation state |
| Dioxus routes, reducers, status summary, discovery, compose, history, and settings | `styrene-ui` | Render backend facts and dispatch typed actions |
| Clipboard, QR, permissions, lifecycle opportunities, notifications, and file sharing | `styrene-ui` | Rust-owned platform services return typed outcomes |
| Fixture handoff, revision pinning, packaged runs, and physical evidence | Cross-repository | Record both immutable revisions and evidence class |

`complete-mobile-p0-backend-contracts` remains authoritative for the backend
contracts it completed. `shared-frontend-session` owns the reusable session
transport. `shared-dioxus-mobile-ui` owns general Dioxus and accessibility
practice. This change owns product integration and acceptance of those outputs;
it does not duplicate their lower-level implementations.

## Product Decisions

Styrene retains Messages, People, Network, and More. The reference application's
Overview demonstrates the value of a concise health summary, not a requirement
to add another top-level route. The summary must use available backend facts and
must mark unavailable facts as unavailable. It must not infer node, relay, path,
mail, or connectivity state from counts, animation, or current bearer labels.

Calls, maps, location sharing, and multi-peer groups remain outside P0. They are
recorded as scope decisions so they cannot be mistaken for overlooked defects.
New Message covers one canonical LXMF delivery destination selected from People,
entered directly, pasted, or scanned.

## Projection Contract

`styrene-rs` remains the sole authority for lifecycle and evidence. The public
mobile projection must preserve:

- stopped, starting, offline-ready, connecting, connected, reconnecting,
  degraded, and failed runtime states;
- canonical message and attempt identifiers, direction, persisted timestamp,
  requested and actual method, retry eligibility, typed failure, propagation
  upload, route and bearer observation, and correlated receipt evidence; and
- propagation selection, metadata freshness, readiness, last synchronization,
  in-flight progress, trigger source, cooldown, and terminal outcome.

`styrene-ui` may derive layout and human-readable formatting. It may not collapse
distinct backend states, infer retryability from terminal status, or reconstruct
delivery evidence from display strings.

### Operational summary authority

The existing public boundaries already own the bounded facts needed by the
operational summary:

- `MobileSessionSnapshot` owns runtime, connection phase, and bearer state;
- `MobilePeerSnapshot` owns the canonical peer projection;
- `ConversationInfo` owns unread counts;
- `MessageAttemptInfo` owns route observations for loaded message attempts; and
- `MobilePropagationSnapshot` owns selection, readiness, synchronization state,
  progress, cooldown, and terminal failure.

No additional backend aggregate is required. `styrene-ui` may count these
bounded projections, but route counts must be labeled as loaded evidence. An
empty route projection remains unknown and must not become a direct path, relay,
reachability, or mail claim.

## Compose And Discovery

People and Messages share one destination-validation path. Starting from a
discovered peer invokes the backend's idempotent conversation-shell operation.
Manual entry, clipboard, and QR decoding all produce the same bounded candidate
value and receive the same backend validation result. A scan or paste result is
never treated as a contact, route, or reachable peer until the backend says so.

The composer evaluates each method independently. Direct availability comes
from backend capability. Propagated availability additionally requires a ready
selected node. An unavailable method remains visible only when its typed reason
helps recovery; submission cannot begin while the selected method is disabled.

## Identity And Platform Services

Public identity metadata and private custody remain separate. Rename persists
public metadata without changing the identity hash. Copy and QR expose only the
public LXMF delivery destination. Encrypted backup exports an opaque,
authenticated artifact through a platform file/share service. Restore validates
and decrypts inside `styrene-rs`, reports typed conflicts and failures, and never
places private key bytes in Dioxus state, logs, fixtures, or accessibility text.

Portable backup export runs against the active mobile node. Portable restore is
a preboot operation because normal mobile boot creates an identity when custody
is absent. On an installation without identity custody, the host must inspect
identity presence and wait for an explicit Create or Restore choice. A failed or
cancelled restore remains before boot and cannot create replacement custody.
The running-node encrypted-file IPC methods do not establish portable Keychain
or Android Keystore recovery.

iCloud-specific synchronization is excluded because no cross-platform recovery
policy has been selected. The UI must not imply cloud backup from Keychain or
Keystore custody alone.

Permission state is queryable independently for every protected capability.
Denial or restriction leaves unaffected workflows operational and provides a
typed route to settings only when the platform supports one. QR scanning is the
only P0 camera consumer. Location sharing remains absent.

## QR Ingress Decision

The P0 scanner uses an operating-system camera or image-picker capture followed
by bounded decoding in Rust. The Dioxus file event supplies one encoded image to
the platform-service boundary. The decoder accepts only JPEG or PNG, enforces
compressed-byte and decoded-pixel limits before QR detection, and returns one
generation-tagged candidate. The backend remains the only destination validator.

The selected decoder is `quircs` with a narrowly configured `image` decoder.
`quircs` is pure Rust and accepts an 8-bit grayscale buffer. The `image` crate
must disable default formats and enable only JPEG and PNG. Tests generate QR
images in memory. Scanned frames are not retained in fixtures, diagnostics, or
failure values.

The following alternatives remain available:

| Option | Fit | Decision |
|---|---|---|
| System capture plus Rust `quircs` decoding | Uses maintained Dioxus file events and no generated native product source; provides capture-then-decode rather than continuous preview | **Selected for P0** |
| Native iOS AVFoundation and Android CameraX with ML Kit or ZXing | Best continuous-scanning UX; iOS dependencies already exist, but the pinned Dioxus Android package has no reviewed Gradle dependency or generated-source ownership seam | Defer until both native integrations are maintainable |
| Web `getUserMedia` plus a Rust or WebAssembly decoder | Cross-platform in principle, but moves camera lifecycle and frame transfer into WebView script and requires more cancellation, privacy, and performance evidence | Defer |
| Web `BarcodeDetector` | Small Android implementation, but WebKit does not provide a dependable enabled implementation for the supported iOS baseline | Reject as a cross-platform requirement |
| External scanner application or deep link | Avoids an embedded decoder, but no standard iOS and Android result contract preserves generation, cancellation, and payload bounds | Reject |
| Paste or manual entry only | Existing safe fallback | Retain, but it does not satisfy QR ingress |

The P0 capture is single-shot and user initiated. It permits camera capture or
selection of an existing image. Cancellation, permission denial, unsupported
capture, no QR code, multiple QR codes, malformed text, oversized input, stale
generation, and decode exhaustion are distinct typed outcomes. A failed scan
does not clear manual or pasted input.

## Propagation Scheduling

The standard propagation coordinator owns synchronization. Automatic triggers
are limited to initial connection, reconnection, and a typed foreground or
platform-granted background opportunity. Each trigger is single-flight, bounded,
and subject to cooldown. A free-running periodic poll is not an eligible mobile
trigger.

The UI states whether automatic synchronization is enabled, which opportunities
the platform can provide, the last trigger source and result, and that manual
synchronization may consume airtime. It never promises background delivery.

## Work Order

`priorities.md` is the ranked LOE/ROI corpus. Execution follows its recommended
waves while preserving these hard dependencies:

1. Freeze authoritative projection and trigger contracts before UI integration.
2. Pin the reviewed backend revision before accepting cross-repository tests.
3. Complete core messaging before optional platform ingress methods.
4. Complete encrypted recovery safety tests before exposing recovery actions.
5. Complete semantic and adaptive checks before physical accessibility claims.
6. Run packaged iOS and Android scenarios only against one declared revision pair.

Backend and frontend can use fixture branches in parallel, but packaged evidence
must use one declared immutable revision pair. A checked implementation task does
not satisfy a device or application-parity task.

## Compatibility And Safety

Serialized backend additions are additive where external consumers require
rolling compatibility. Exhaustive enum changes require a coordinated UI update.
Persisted identity or message changes require migration and forced-restart tests.

Diagnostics and evidence artifacts remain payload-free and privacy-reviewed.
Public identity destinations may be present only where the evidence policy
explicitly permits them. Private keys, message content, endpoint credentials,
clipboard contents, and scanned raw frames are forbidden from logs and fixtures.
