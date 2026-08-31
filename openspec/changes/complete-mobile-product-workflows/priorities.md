# Complete Mobile Product Workflows Priority Corpus

## Scoring

ROI is ordinal:

- **Critical:** prevents false delivery, readiness, lifecycle, custody, or release claims.
- **High:** unlocks a required P0 user journey or removes a blocking dead end.
- **Medium:** improves operator understanding, platform completeness, or evidence accounting without independently unlocking send and receive.

LOE is a planning estimate for the owning implementation, tests, and review:

- **S:** up to two engineer-days in one repository.
- **M:** three to five engineer-days or a coordinated additive contract change.
- **L:** one to two engineer-weeks, native platform breadth, or substantial fixture coverage.
- **XL:** multi-week cross-repository, persistence, migration, or physical-device work.

The corpus is sorted by ROI first and LOE second. Dependency order may require a
lower-ranked contract or fixture task before implementation. Estimates must be
revisited after the first failing test establishes the actual boundary.

## Sorted Corpus

| Rank | Slice | ROI | LOE | Owner | Task references | Outcome |
|---:|---|---|---|---|---|---|
| 1 | Propagated method readiness gate | Critical | S | `styrene-ui` | 3.5 | Prevents false ready-to-send state and preserves drafts when node readiness changes |
| 2 | Lossless runtime and delivery projection | Critical | M | Both | 1.1, 2.1-2.5 | Stops runtime collapse, fabricated retry, and dropped route, bearer, upload, and receipt evidence |
| 3 | Event-driven propagation scheduling | Critical | M | `styrene-rs` | 6.1, 6.6 | Removes unsupported periodic mobile polling and enforces bounded lifecycle opportunities |
| 4 | Immutable backend/UI handoff | Critical | M | Cross-repository | 1.4 | Makes projection loss detectable and establishes one reviewed revision pair |
| 5 | Packaged P0 acceptance | Critical | XL | Cross-repository | 8.1-8.3, 8.6 | Converts implemented behavior into release evidence for messaging and propagation |
| 6 | Encrypted identity recovery safety | Critical | XL | Both | 5.3-5.7, 8.4 | Adds non-destructive backup/restore without exposing private material |
| 7 | Discovered peer starts conversation | High | S | `styrene-ui` | 3.1 | Removes the current People dead end using the existing backend operation |
| 8 | Truthful propagation disclosure | High | S | `styrene-ui` | 6.4 | Removes unconditional automatic-sync claims and exposes disabled reasons and airtime limits |
| 9 | First-class New Message with direct entry | High | M | `styrene-ui` | 3.2, 3.5 | Unlocks conversation creation without waiting for discovery |
| 10 | Message chronology and evidence detail | High | M | `styrene-ui` | 2.3 | Makes direction, time, method, retry, and delivery evidence usable in conversation history |
| 11 | Public identity controls | High | M | `styrene-ui` | 5.1-5.2 | Adds correct LXMF labeling, rename, public copy, and public QR without private material |
| 12 | Propagation trigger and outcome DTOs | High | M | `styrene-rs` | 6.2 | Gives the UI authoritative readiness, trigger, progress, cooldown, and terminal state |
| 13 | Operational status summary | High | M | Both | 4.1-4.2, 4.4 | Consolidates runtime, bearer, peer, unread, route, and propagation truth without copying Skywave navigation |
| 14 | Clipboard and QR destination ingress | High | L | `styrene-ui` | 3.3-3.4, 3.6 | Adds safe paste and scan paths with bounded validation and denial recovery |
| 15 | Adaptive and semantic workflow coverage | High | L | `styrene-ui` | 7.1-7.3 | Prevents inaccessible or keyboard-obscured completion paths in changed workflows |
| 16 | Physical screen-reader evidence | High | XL | Cross-repository | 7.4 | Supports VoiceOver and TalkBack claims in shipping WebViews rather than semantic snapshots |
| 17 | Permission-state surface | Medium | M | `styrene-ui` | 6.3 | Makes camera, Bluetooth, notification, and custody limitations recoverable and independent |
| 18 | Peer observation details | Medium | S | `styrene-ui` | 4.3 | Uses existing aspect, source, age, and announce-count state without claiming reachability |
| 19 | Corpus and reference reconciliation | Medium | S | `styrene-rs` | 1.2-1.3, 8.5 | Corrects stale capability text and capture-scoped RNS metadata without promoting evidence |

## Recommended Waves

1. **Truth and safety:** ranks 1-4.
2. **Core messaging completion:** ranks 7-10 and 12.
3. **Operational clarity:** ranks 13, 18, and 19.
4. **Platform completion:** ranks 11, 14, and 17.
5. **Recovery and inclusive release:** ranks 6, 15, and 16.
6. **Acceptance:** rank 5 after its applicable implementation slices pass.

Calls, Map, location sharing, groups, iCloud-specific synchronization,
propagation hosting, and guaranteed background execution are excluded and have
no LOE or ROI rank in this P0 corpus.
