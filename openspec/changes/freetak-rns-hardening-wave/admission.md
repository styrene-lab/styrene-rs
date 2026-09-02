# FreeTAK RNS Hardening Wave: Immutable Admission Record

Recorded 2026-09-02 before any test or implementation work in this wave.

## Evidence repository

- URL: `https://github.com/FreeTAKTeam/LXMF-rs.git`
- Immutable review range: `3a2d46bbea174a1049d5d3e06f00c6ea20254085..0ed96f7ee33cefe7fe6eb188b8094b02cd536193`
- Role: implementation evidence only. The endpoint is licensed EPL-2.0 with
  GPL-2.0-or-later elected as its secondary license. It is not an
  MIT-compatible patch source for Styrene.
- Applicable evidence commits per gap are listed in `design.md`. This record
  does not add to or move that range.

## Protocol authority

- Reticulum 1.5.1 at immutable revision
  `149e4151095adf098b8f53eab0c03b37169e8559`, pinned by the archived
  `reticulum-1-5-parity-wave`.
- The internal-interface announce policy was introduced at Reticulum 1.5.0
  revision `e32d4df754a7b87b1bf1bb0d08675d12ff505ae6` and must be confirmed
  unchanged at the 1.5.1 authority before group 14 claims it.

## Admission

Nothing from the evidence repository has been copied or will be copied into
Styrene. That covers source, tests, fixtures, test vectors, comments, symbol
names, module layout, and line-for-line control flow. Every test and implementation in this wave is
written independently from this record, the design's behavioral statements,
the authoritative protocol, public APIs, and black-box observation. This
record changes no refs, tracking markers, or other OpenSpec changes.

## Predecessor gates

| Group | Gate | Evidence | Startable |
| --- | --- | --- | --- |
| 2 Cached Fernet | `reticulum-1-5-parity-wave` group 10 | Archived 2026-08-30 at `openspec/archive/2026-08-30-reticulum-1-5-parity-wave/tasks.md`, 36/36 complete | Recorded complete in task 2.1 |
| 3, 4, 5, 7, 8, 9, 12 | Provenance admission only | This record | Yes |
| 6 Link mutation | `beechat-rns-corrections-wave` group 2 (LinkRTT wire precision) | Open, 0/3 | No, blocked |
| 10 Resource caps | `reticulum-1-5-parity-wave` group 7 plus this wave's group 9 | Archived complete | After group 9 |
| 11 Split resources | `reticulum-1-5-parity-wave` group 8 plus this wave's group 10 | Archived complete | After group 10 |
| 13, 14 Announce policy | `reticulum-1-5-parity-wave` groups 2-6 | Archived complete; the reconciled admission and path architecture is in `crates/libs/styrene-rns/src/transport/core_transport` | Yes |
| 15 Ordered-byte attempt | Provenance admission; mobile ownership retained by `shared-dioxus-mobile-ui` group 5 and `deliver-mobile-messaging-minimum` groups 6-7 | Complete (tasks 15.1-15.3) | Done |

## Behavioral clarifications

These points were derived independently while implementing the admitted
groups. They clarify Styrene behavior without changing the admitted evidence
range or copying from the evidence repository.

- Advertisement `segment_index` and `total_segments` describe a resource's
  place in a split transfer. An advertisement always carries the first
  hashmap segment of its own resource. Later hashmap segments arrive as
  hashmap updates. The receiver used to read the two fields as hashmap
  segment numbers, which rejected any resource spanning more than one hashmap
  segment.
- A hashmap continuation is anchored at the last mapped hash before the gap.
  That hash precedes the consecutive height once a whole segment has arrived.
- Arriving fragments reset the receiver's retry budget, so the budget bounds
  consecutive stalls rather than the whole transfer. The request window is
  bounded to `1..=10` fragments.
- The local exception that lets a passive node queue announces is expressed
  as the `shared_instance` interface descriptor flag. Styrene has no shared
  instance transport of its own. The flag is the hook an embedding host sets
  for interfaces that serve local client instances.
- The two internal announce flags live on the interface descriptor, are
  inherited by child interfaces when unset, and can be hot-applied per
  interface. The transport has no configuration file keys for them yet.
- Supervised workers drain within five seconds of cancellation or a sibling
  failure before any straggler is aborted.
- A segment that cannot be built or dispatched fails its split as
  link-closed when the Link is gone and as an integrity failure otherwise.
  No new failure reason was added because the daemon matches the existing
  reasons exhaustively.
- Split segments are only produced by outbound data transfers. Request and
  response resources remain single resources.
- The internal announce policy predates Reticulum 1.5.0. In a clone of the
  Reticulum repository, `announces_from_internal` is present at tag 1.4.0
  and `announces_to_internal` at tag 1.4.1. The 1.5.0 tag commit
  `e32d4df754a7b87b1bf1bb0d08675d12ff505ae6` and the 1.5.1 tag commit
  `149e4151095adf098b8f53eab0c03b37169e8559` carry identical announce
  decision rows, so the 1.5.1 authority stands unchanged.
