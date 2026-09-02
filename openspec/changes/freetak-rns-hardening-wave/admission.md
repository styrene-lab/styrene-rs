# FreeTAK RNS Hardening Wave: Immutable Admission Record

Recorded 2026-09-02 before any test or implementation work in this wave.

## Evidence repository

- URL: `https://github.com/FreeTAKTeam/LXMF-rs.git`
- Immutable review range: `3a2d46bbea174a1049d5d3e06f00c6ea20254085..0ed96f7ee33cefe7fe6eb188b8094b02cd536193`
- Role: implementation evidence only. The endpoint is licensed EPL-2.0 with
  GPL-2.0-or-later elected as its secondary license. It is not an
  MIT-compatible patch source for Styrene.
- Applicable evidence commits per gap are listed in `design.md`; this record
  does not add to or move that range.

## Protocol authority

- Reticulum 1.5.1 at immutable revision
  `149e4151095adf098b8f53eab0c03b37169e8559`, pinned by the archived
  `reticulum-1-5-parity-wave`.
- The internal-interface announce policy was introduced at Reticulum 1.5.0
  revision `e32d4df754a7b87b1bf1bb0d08675d12ff505ae6` and must be confirmed
  unchanged at the 1.5.1 authority before group 14 claims it.

## Admission

No source, test, fixture, test vector, comment, symbol name, module layout,
or line-for-line control flow from the evidence repository has been copied or
will be copied into Styrene. Every test and implementation in this wave is
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
