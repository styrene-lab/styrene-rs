# Contributing to Styrene

Styrene is systems and security software. Small, reviewable changes with explicit tests are preferred over broad speculative rewrites.

## Development setup

Install Git, [Rustup](https://rustup.rs/), and [`just`](https://github.com/casey/just), then:

```bash
git clone https://github.com/styrene-lab/styrene-rs.git
cd styrene-rs
just check
just test
```

The committed `rust-toolchain.toml` pins the supported stable Rust release. Do not silently downgrade the workspace MSRV or introduce nightly-only features.

## Before opening a change

```bash
just format
just validate
```

For protocol work, also run:

```bash
just test-interop
```

Relevant focused commands:

```bash
cargo test -p <crate>
cargo clippy -p <crate> --all-targets -- -D warnings
just install                 # local release-build upgrade test
```

## Engineering expectations

- Add tests for non-trivial behavior and regressions.
- Preserve `unsafe_code = "forbid"` unless a separately reviewed architectural decision changes the policy.
- Keep external input bounded and validated: paths, IPC payloads, network frames, and imported identities are trust boundaries.
- Never emit background diagnostics directly into the TUI framebuffer. Runtime events belong in structured application channels.
- Do not commit secrets, private identities, databases, generated build output, or local Flynt/Omegon state.
- Keep wire-format changes explicit and test them with fixtures.
- Prefer narrow crate dependencies; protocol-core crates must not absorb application dependency trees.

## Commit and pull-request guidance

Use focused conventional commits where practical:

```text
feat(tui): add runtime profile indicator
fix(rns): validate transported link proof
build: refresh release target matrix
```

A pull request should state:

1. The problem and affected interface.
2. The chosen mechanism and relevant tradeoffs.
3. Tests and validation performed.
4. Compatibility, migration, or security implications.

## Upstream protocol changes

Do not merge or rebase the historical upstream repositories into this tree. The workspace has diverged structurally. Follow the review-and-apply process in [UPSTREAM.md](UPSTREAM.md):

```bash
just upstream-status
just upstream-review
```

Cite the source commit when porting an upstream correction.

## Security reports

Do not open a public issue for a suspected vulnerability involving identity keys, authentication, packet validation, privilege boundaries, or remote code execution. Follow [SECURITY.md](SECURITY.md).

## License

By contributing, you agree that your contribution is licensed under the repository's [MIT License](LICENSE).
