# Stabilize Mobile Platform Hosts

## Intent

Preserve and verify the current iOS and Android host work before frontend
contracts or repositories move. The native hosts are transition references and
hardware integration evidence. They are not the target cross-platform UI.

## Scope

This change includes the current embedded-node lifecycle and direct TCP profiles.
It also includes RNode bearers, mobile integration fixtures, deployment support,
and host validation. It separates the work into reviewable behavioral commits.

This change excludes a new frontend API, extraction of `styrene-dx`, and a
Dioxus mobile target. It does not remove native hosts or claim unsupported
physical Android evidence.

This change is the prerequisite for `shared-frontend-session`.

## Success criteria

- The mobile changes are represented by reviewable commits based on refreshed
  `origin/main`.
- Rust, Android, iOS, corpus, deployment, and documentation checks pass at the
  declared evidence level.
- A cold iOS launch restores the embedded node and approved Bluetooth RNode
  without a manual start action.
- Android keeps Bluetooth as the default RNode bearer and USB as an explicit
  fallback.
- Generated bindings, native libraries, application packages, and runtime
  evidence remain untracked.
