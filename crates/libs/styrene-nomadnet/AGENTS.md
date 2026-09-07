# NomadNet domain boundary

Read README.md and the nomadnet-crate-extraction OpenSpec change before work.
This internal workspace crate owns NomadNet browsing sessions, cache, downloads,
content projection and lifecycle policy. It must not depend on styrened,
styrene-ipc, UI crates or styrene-session. Workspace layer checks enforce this.

Keep parsing in styrene-micron and route/link repair in styrene-rns. Keep runtime
composition, local private identity selection and authorization in daemon adapters.
Domain models have no serde implementations; convert stable IPC DTOs explicitly
in styrened. Preserve observation/correlation and owner identifiers during conversion.
The retained public IPC address API must stay behaviorally compatible with the
canonical domain parser; exercise both in integration tests when changing addresses.

Preserve password-default omission, submission Debug redaction, field ordering,
validation bounds, exact native MessagePack bytes and response trailing-byte
rejection. Preserve absolute deadlines, cancellation/drop cleanup, borrowed-link
protection, reference counts, capacity limits, owner isolation and atomic saves.
Do not change cache/route policy as an incidental refactor.

Run crate tests, daemon native_browse adapter tests, nomadnet_split integration
tests, warning-denied Clippy and `just check-workspace-policy`. Do not infer real
mesh or mobile hardware readiness from scripted or Unix IPC fixture evidence.
