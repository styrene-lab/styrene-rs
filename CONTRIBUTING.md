# Contributing to LXMF-rs

## Goals
- Keep protocol libraries small, explicit, and testable.
- Enforce one-way dependencies between `libs` and `apps`.
- Keep release behavior deterministic through repeatable gates.

## Development Setup
- Rust stable (`rust-toolchain.toml` tracks the pinned MSRV toolchain).
- Install optional tooling:
  - `cargo install cargo-deny cargo-audit cargo-public-api`
  - `cargo install cargo-nextest`
  - `cargo install cargo-udeps --locked`

## Local Quality Gates

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --no-deps -- -D warnings
cargo test --workspace
cargo doc --workspace --no-deps
cargo deny check
cargo audit
cargo +nightly udeps --workspace --all-targets
./tools/scripts/check-boundaries.sh
```

Equivalent umbrella commands:

```bash
cargo xtask ci
cargo xtask release-check
```

## Pull Requests
- One logical change per PR.
- Include tests for behavior changes.
- Mark breaking changes clearly.
- Update `docs/contracts/*` and `docs/migrations/*` for API/wire contract changes.
