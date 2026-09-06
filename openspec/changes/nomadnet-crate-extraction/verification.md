# Gate 1 verification — 2026-09-06

Backend base: cdeda5c80d7cacf1811d325239e97d4c980ba773.

Implemented: internal domain workspace crate, pure Micron projection, form encoding,
native binary-response decoding, and explicit daemon IPC conversions. Public IPC
sources and serialized DTO definitions are unchanged. Native coordinator production
calls use the extracted functions; old implementations were removed.

- `cargo test --locked -p styrene-nomadnet`: 5 passed. Includes exact native form
  bytes, redaction, projection/links/warnings and invalid response/submission cases.
- `cargo test --locked -p styrened --lib native_browse`: 37 passed through the new
  production adapter. Existing lifecycle, cache, owner cleanup and form tests retained.
- `cargo clippy --locked -p styrene-nomadnet -p styrened --lib --tests -- -D warnings`:
  passed after removing imports made unused by extraction.
- `cargo fmt --all -- --check`: passed.
- Workspace policy checker and its 9 tests: passed. New crate is in the domain layer,
  has no upward dependency, and remains unpublished. Lockfile only adds this workspace
  package and the daemon dependency.

Coordinator contracts and ownership have not moved yet. Address compatibility,
cache policy fixes and failed-route recovery remain outstanding as specified. Keep
this change unarchived. No desktop pin update, running-daemon replacement, crates.io
publication, remote push, Apple device, Android or Nucleus validation performed.
