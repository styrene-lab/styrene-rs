# Contributing to Styrene

Styrene is systems and security software. Small, reviewable changes with explicit tests are preferred over broad speculative rewrites.

## Development setup

Install Git, [Rustup](https://rustup.rs/), [`just`](https://github.com/casey/just),
and Python 3.11 or newer for repository policy and product tooling, then:

```bash
git clone https://github.com/styrene-lab/styrene-rs.git
cd styrene-rs
just check
just test
just install-hooks
```

The committed `rust-toolchain.toml` pins the supported stable Rust release. Do not silently downgrade the workspace MSRV or introduce nightly-only features.
Workspace crates inherit Rust edition 2024 and the pinned `rust-version` from
`workspace.package`; new crates must use those shared settings rather than
declaring an older local edition or MSRV.

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

## Workspace boundaries

`Cargo.toml` is the source of truth for crate layers and public package intent under
`workspace.metadata.styrene`. Every workspace crate belongs to exactly one layer.
Production dependencies must point to the same or a lower layer, while dev
dependencies may cross layers for tests and harnesses.

Run the policy check after adding a crate, changing an internal dependency, or
changing publication metadata:

```bash
just check-workspace-policy
```

Internal crates must set `publish = false`. Public crates may depend only on other
public workspace crates. Each such path dependency must declare the dependency's
current package version; Cargo represents this repository convention as an exact
caret requirement such as `^0.1.0`.

Selected reusable crates must keep their host library target valid with default
features disabled:

```bash
just check-library-minimal
```

This check proves feature isolation on the host; it does not prove a complete
`no_std` dependency graph. Feature flags should be additive capabilities. Keep
platform, hardware, and live service requirements out of ordinary validation
unless a crate's documented default contract explicitly requires them.

## Commit and pull-request guidance

Use focused conventional commits where practical:

```text
feat(tui): add runtime profile indicator
fix(rns): validate transported link proof
build: refresh release target matrix
```

Repository hooks reject automated attribution, bot authorship trailers, and
model-generated boilerplate. Committers remain responsible for every change.

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
