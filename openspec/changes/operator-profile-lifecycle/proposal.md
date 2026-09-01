# Operator Profile Lifecycle

## Intent

Replace frontend-specific runtime modes with a backend-owned profile lifecycle
that has coherent storage, explicit daemon ownership, truthful persistence, and
identity-preserving transitions.

## Scope

This change defines Quick, Local, Portable, and Connected profiles. It covers
profile roots, ownership, promotion, snapshots, restore, custody continuity,
portable-media safety, typed IPC operations, and frontend migration.

Protocol behavior, LXMF delivery policy, automatic removable-media execution,
and unverified hardware-custody claims remain outside this change.

## Success criteria

- Every managed daemon path derives from one validated profile root or host-private runtime root.
- Promotion and restore preserve identity and committed state without publishing partial output.
- Live snapshots use a coherent database backup mechanism instead of copying WAL files.
- Portable profiles fail closed on ownership conflict or media loss.
- Desktop and TUI clients consume one typed backend lifecycle contract.
