# NomadNet domain boundary

Read README.md and the active nomadnet-crate-extraction OpenSpec change before
continuing the move. This internal workspace crate owns NomadNet content behavior;
it must not depend on styrened, styrene-ipc, UI crates or styrene-session.

Keep parsing in styrene-micron and route/link repair in styrene-rns. Keep runtime
composition, identity selection and authorization in daemon adapters. Convert IPC
DTOs explicitly; do not duplicate their serialization into domain types.

Preserve password-default omission, submission Debug redaction, field ordering,
validation bounds, exact native MessagePack bytes and response trailing-byte
rejection. Keep unknown IPC field kinds ignored when encoding selected fields.
Do not mix cache/transport fixes into the behavior-preserving extraction.

Run crate tests, the daemon native_browse tests, warning-denied Clippy and
`just check-workspace-policy`. Existing daemon tests intentionally exercise the
adapter boundary in addition to the standalone domain tests.
