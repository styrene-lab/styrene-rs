# Execution and verification record

This record distinguishes environment readiness from product acceptance.
No RNS acceptance case has passed on an integrated change revision yet.

## Source and handoff identities

| Role | Branch or record | Exact input revision |
|---|---|---|
| Product baseline | Canonical `main` | `ab46aa1f13de5663c684225dc27dfcba330d965f` |
| Planning baseline | `fix/rns-no-std-78` | `9c681e8e40718b9af8a6189d2fbee4094808bf69` |
| D2 contract amendment | `fix/rns-no-std-78` | `e9e663b5691452200ae642f233059e2f267fb5dd` |
| A: dependencies | `work/issue78-dependencies` | Planning baseline |
| B: core | `work/issue78-core` | Planning baseline |
| C: validation | `work/issue78-validation` | D2 contract amendment |
| Combined A+B validation input | Detached validation worktree | `27cf0af68d5b58a30f9f94ff5ec5e3217c880679` |

Lane results and the integrated validation revision must be recorded here before
acceptance. A completed environment probe is not a lane handoff.

## Reviewed lane A handoff

Lane A returned clean commit `3627745b3c969c18a2e1c6a79f71fb764d5c8f20` from
the planning baseline. It changes only `crates/libs/styrene-rns/Cargo.toml`.
The product lockfile remains byte-identical with SHA-256
`5dfd8a486d974d4cd3bac90d6dff9402f7d261bc907df066ca1c4d2f5661ca10`.

The independent reviewer approved the commit for combined A+B integration with
no additional blockers. The reviewer checked the committed patch, retained
manifest, version constraints, security features, and 287 evidence hashes.

Lane evidence includes successful dependency-graph assertions, Linux default,
explicit-std, transport and serial checks, and LXMF/mesh/tunnel/daemon checks.
Workspace policy and its nine tests passed. These are component-level results,
not acceptance on the integrated revision.

Minimal native compilation fails at four pre-existing `OsRng` references in core
paths owned by lane B. This is the expected A+B dependency, not an accepted final
state. Independent-consumer target compilation and runtime evidence are pending.

Evidence:

```text
~/.local/state/styrene-lab/issue-78/runs/dependencies/styrene-issue78-dependencies-XOgzG1O4/HANDOFF.md
```

## Reviewed lane B handoff and combined input

Lane B returned clean commit `131555c6ee290fd242b98b484a12e7c72f0b353a` from
the planning baseline. It changes 13 authorized non-transport RNS source/test
files and leaves the manifest and product lockfile unchanged.

Independent review approved it for combined integration. Review covered
borrowed RNG state, legacy trait and immutable-Fernet compatibility, key splits,
framing, errors, constant-time authentication, and rotation/clock behavior.
The reviewer verified all 13 changed-file hashes and 84 evidence checksums.

Lane-local focused tests passed for borrowed providers, decryption, embedded
time, applicable canonical/alternate crypto fixtures, hosted compatibility, and
persistence. The new persistence tests also passed against unchanged base
sources. These are not final integrated acceptance results.

The transport fixture link attempt failed because its environment omitted
SQLite linker inputs. Investigation confirmed that SQLite and its native build
tools already exist in the declared cached native environment. Combined host
validation uses those existing inputs without provisioning or profile changes.

Evidence:

```text
~/.local/state/styrene-lab/issue-78/runs/core/20261021T011604Z/HANDOFF.md
```

The coordinator integrated the reviewed A and B commits as `3bad1011` and
`27cf0af6`. The exact combined input is
`27cf0af68d5b58a30f9f94ff5ec5e3217c880679`. A clean detached worktree preserves
that revision for validation and as the product input to lane C's independent
consumer.

## Combined A+B validation

The focused matrix at `27cf0af68d5b58a30f9f94ff5ec5e3217c880679` completed all
40 recorded commands successfully, without retries or timeouts. Initial/final
source, index, configuration, and lockfile fingerprints match.

| Check | Observed result |
|---|---|
| AArch64 minimal, alloc, and alloc/AES-128 library checks | All passed |
| Minimal Clippy and portable RNG/decrypt/time tests | All three profiles passed; 13 tests per profile |
| Default and explicit-std package tests | 66 passed per profile |
| Full interop/transport tests | 389 passed |
| Three ordinarily ignored loopback tests | Passed when run explicitly |
| Alternate crypto tests | Selected groups passed: 4, 27, and 1 tests |
| Transport/serial compilation and transport Clippy | Passed |
| LXMF, mesh, tunnel, and daemon compilation | Passed |
| Shared fixture/evidence tests | 13 passed |
| Workspace policy and formatting | Passed; nine policy tests passed |

These counts are per invocation, not unique-test totals. Hosted dev-dependencies
can restore some dependency `std` features. Independent-consumer and final
integrated validation remain separate obligations. SQLite linking succeeded using
the existing cached native environment.

Evidence:

```text
~/.local/state/styrene-lab/issue-78/runs/integration/20261021T014134Z-53j1wj84/
```

`HANDOFF.md` and `RESULTS.md` identify exact commands, counts, durations, and log
hashes. The `evidence.sha256` file has digest
`40dac8c0c3bb7d41b1fde0bd79cc5f06e47d543d3e6a95f5268a436198e4d891`.

## Exact-base regression proof

An immutable archive of canonical base
`ab46aa1f13de5663c684225dc27dfcba330d965f` was checked with the same pinned
Rust 1.97 toolchain and actual AArch64 target libraries. The minimal library
check failed with exit 101 at `getrandom`'s unsupported-target diagnostic.
A diagnostic repetition with `--keep-going` also exposed `E0463` for target
`std` in `serde_core`, `num-traits`, `signature`, `byteorder`, and `hex`.

This is the expected portability failure, not a missing-core, cache, or manifest
setup failure. It contrasts with the three target-profile passes on reviewed
combined source `27cf0af68d5b58a30f9f94ff5ec5e3217c880679`.

Evidence:

```text
~/.local/state/styrene-lab/issue-78/runs/integration/20261021T015641Z-baseline-tcdrq2sl/HANDOFF.md
```

The baseline archive SHA-256 is
`174af84814c01da84e84c4c181da44f5f9ad0e836387fce53e99a3ba7ea3a245`.
The evidence inventory SHA-256 is
`3f7063037a22a21756391fd83ffa9647f4fbdc2cf62fb227e8a944aa554ad98d`.

## Lane C review in progress

Lane C returned commits `efcdb2aff936ae1a546b0978b331c0f57474a387` and
`0cf5f83c33c8c8143a59cc21e001e3a67fa28005` against the D2 amendment. Its recorded
positive runs compile, link, and execute all three profiles with product input
`27cf0af68d5b58a30f9f94ff5ec5e3217c880679`. Review verified the source binding,
47 indexed evidence hashes, and the positive target results.

The initial handoff is not accepted for integration. Independent review requires:

| Finding | Required correction | Status |
|---|---|---|
| C-1: OOM false acceptance | Distinguish unexpected allocation success from actual OOM; require the intended fault point and test rejection | Returned to lane C |
| C-2: cancellation cleanup | Route SIGTERM through owned-child cleanup and test cancellation without a successful result | Returned to lane C |
| C-3: symlink provenance | Reject input-root and ancestor symlinks before resolution/materialization; test containment | Returned to lane C |

Other reviewed boot properties include confined unsafe modules, checked bump
allocation, EL1/interrupt/FP initialization, Normal-memory translation, and bounded
ELF regions. This review does not waive the three findings or final revalidation.

Initial evidence:

```text
~/.local/state/styrene-lab/issue-78/runs/validation/rns-no-std-7a3_kdcx/handoff.json
```

## Environment readiness

On Nucleus, environment run `20261021T011622Z` provisioned the separately
authorized retained tool profile:

```text
~/.local/state/styrene-lab/profiles/rns-no-std
```

The configuration owner is `/home/wilson/nix-config/tools/rns-no-std/`.
Its new `flake.nix`, `flake.lock`, and `README.md` remain untracked and unstaged
in that owning repository. They are not product-source changes.

Verified tool identities:

- Rust and Cargo 1.97.0, compiler commit
  `2d8144b7880597b6e6d3dfd63a9a9efae3f533d3`.
- Rust sysroot: `/nix/store/bbbljcccczvsg1hxpc3w8sa8lpffbmi5-rust-default-1.97.0`.
- AArch64 target libraries:
  `/nix/store/7dv7fkx9g0xviksmwh8am0150wksg1vm-rust-std-1.97.0-aarch64-unknown-none`.
- Profile output: `/nix/store/k5rsxm0vbwf48xmzhh6r6119ab18rm5a-profile`.
- Reused QEMU 11.1.0 and bundled LLD 22.1.6.

The target contains `core`, `alloc`, and compiler builtins, but no target `std`.
Rust reports the required 64-bit and pointer-width atomics. An isolated no-std
library using `Vec`, `Arc`, and `AtomicU64` built offline for
`aarch64-unknown-none`. Object inspection identified AArch64 code.

The coordinator independently rechecked the retained profile, compiler/Cargo/QEMU
versions, and target `core`/`alloc` presence. No default toolchain, service,
NixOS/Home Manager activation, or existing owner configuration was changed.
Nix's GC-root query performed automatic stale-root housekeeping, recorded in the
environment evidence. No garbage collection was requested.

Evidence root:

```text
~/.local/state/styrene-lab/issue-78/runs/environment/20261021T011622Z/
```

`summary.json` links the commands, provenance, before/after preservation checks,
and compile result. This establishes tool availability only. It does not establish
RNS target compilation, executable linking, guest execution, or hardware support.

## Acceptance state

Cases `RNS-NOSTD-001` through `RNS-NOSTD-012` remain unaccepted on a final A+B+C
revision. Environment readiness and reviewed A+B implementation/validation are
complete. Lane C corrections, final integration, required remaining validation,
and final acceptance remain outstanding.

The [test plan](test-plan.md) defines case criteria. The
[execution plan](execution-plan.md) defines authority and required evidence.
No push, PR, merge, issue closure, archive, or release is authorized by this record.
