# Execution and verification record

This record distinguishes local validation from operator acceptance.
The integrated candidate has passing local evidence. Required macOS/remote CI
checks and final acceptance remain open.

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
| Combined A+B+C local candidate | Detached validation worktree | `02643d7c9c4bd306bd2ff181b7c230137bbbdc06` |

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

## Lane C review and corrections

Lane C returned commits `efcdb2aff936ae1a546b0978b331c0f57474a387` and
`0cf5f83c33c8c8143a59cc21e001e3a67fa28005` against the D2 amendment. Its recorded
positive runs compile, link, and execute all three profiles with product input
`27cf0af68d5b58a30f9f94ff5ec5e3217c880679`. Review verified the source binding,
47 indexed evidence hashes, and the positive target results.

The initial handoff was withheld from integration. Independent review required:

| Finding | Required correction | Status |
|---|---|---|
| C-1: OOM false acceptance | Distinguish unexpected allocation success from actual OOM; require the intended fault point and test rejection | Corrected in `1da7b648`; independent re-review closed |
| C-2: cancellation cleanup | Route SIGTERM through owned-child cleanup and test cancellation without a successful result | Corrected in `c716bb24`; independent re-review closed |
| C-3: symlink provenance | Reject input-root and ancestor symlinks before resolution/materialization; test containment | Corrected in `7dd36ba1`; independent re-review closed |

Other reviewed boot properties include confined unsafe modules, checked bump
allocation, EL1/interrupt/FP initialization, Normal-memory translation, and bounded
ELF regions. Both reviewers approved corrected result
`7dd36ba16b932ac6f494f37d0b5514833475737c` for integration after checking the new
controls, source binding, and 55 evidence-file hashes.

Initial evidence:

```text
~/.local/state/styrene-lab/issue-78/runs/validation/rns-no-std-7a3_kdcx/handoff.json
```

Corrected handoff:

```text
~/.local/state/styrene-lab/issue-78/runs/validation/rns-no-std-wbrd16g7/handoff.json
```

Its SHA-256 is
`27d4bc2fc1947d05c3f30a6730204e09230b6f9d26bb08c8546044a9ebf732b1`.

## Final integrated local validation

Product and fixture both came from clean integrated candidate
`02643d7c9c4bd306bd2ff181b7c230137bbbdc06`. Initial/final source, index,
configuration, and lockfile fingerprints match. All 51 outer commands passed.
The 38 runner child records retain their expected negative statuses rather than
rewriting negative-control failures as successful compilation.

- The 40-command matrix passed: 608 Rust test executions and nine policy tests.
- Seventeen runner tests passed. Each independent runner invocation passed three
  allocator tests.
- All three independent profiles compiled, linked, and executed in QEMU with
  exactly the completion marker and exit zero.
- The negative consumer failed with the intended missing-target-`std` E0463.
- Armed panic/OOM probes exited one with their expected handshakes.
- Unexpected allocation success exited three and was rejected. Early unarmed
  panic exited one and was rejected.
- Formatting, OpenSpec structure, and the cached CI command-shape rehearsal passed.

Seven executed ELFs and linker maps are retained. Counts represent test
executions, not distinct tests or physical-device coverage.

Evidence:

```text
~/.local/state/styrene-lab/issue-78/runs/integration/20261021T021357Z-final-wmywplmr/
```

`HANDOFF.md`, `RESULTS.md`, and `CASES.md` identify the environment, all commands,
results, durations, log hashes, and per-case limits. The evidence inventory digest
is `bd2093faf42ebc143f9799f45989746afc2fd87a2fa07456d8464edd15b1109f`.

Independent final integration review verified all 34 reviewed lane files and Git
modes, all 361 indexed evidence files, and same-revision product/fixture binding.
It found no new implementation blockers. Additional integration changes were
documentation/OpenSpec only, and the product lockfile remained unchanged.

This ledger records evidence for the exact candidate above. Later ledger-only
commits must not be described as that tested SHA. PR-head CI and any changed
implementation require their own attributable validation.

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

Local PASS means the stated check completed at `02643d7c`, not operator acceptance.

| Cases | Local result | Remaining scope |
|---|---|---|
| 001–007 | PASS | Required delivery review and CI gates remain separate |
| 008 | PARTIAL | Required macOS checks remain unexecuted |
| 009–011 | PASS | Synthetic/fixture evidence does not establish physical hardware acceptance |
| 012 | PARTIAL | Remote GitHub Actions, fresh-runner acquisition, and artifact upload remain unexecuted |

## Broader workspace assessment

The broader assessment also used clean candidate
`02643d7c9c4bd306bd2ff181b7c230137bbbdc06`, without provisioning or source edits.
Workspace/all-target compilation and Clippy passed. Additional successful
selections included 1,027 core tests, 21 RNode tests, 58 IPC-server tests,
17 offline E2E tests, two CLI tests, and 189 Identity repository-signing tests.
Minimal-library, five daemon compile profiles, and offline-boundary/corpus/
attribution/fixture-preservation checks also passed.

The daemon library gate failed: **573 passed, six failed, three ignored**.
Cargo did not execute its 23 selected integration-test targets after the library
failure. The Perl-dependent escaped-session test returned early because
`/usr/bin/perl` was absent, so its cleanup behavior was not exercised.

Production dynamic-page cleanup launches absolute `/bin/ps`, which is absent on
this NixOS host. Fixtures also require absent `/bin/sleep` and `/usr/bin/touch`.
The utilities exist at Nix paths, but changing PATH cannot satisfy absolute
lookups. Both affected source files are unchanged from canonical `ab46aa1`.
Independent review verified the causal diagnostics, unchanged files, and all
180 indexed evidence files. It classified this as a pre-existing daemon host-layout
assumption affecting production cleanup as well as fixtures.

No failed assertion was weakened, skipped, retried, or converted to a pass.
No command timed out. Standard local workspace readiness remains blocked.
An authorized compatible environment must run the complete daemon selection and
actually exercise the Perl-dependent test before merge and acceptance.

Evidence:

```text
~/.local/state/styrene-lab/issue-78/runs/integration/20261021T023409Z-workspace-readiness-nxl0xcpx/
```

`HANDOFF.md`, `RESULTS.md`, and `DAEMON-BLOCKER.md` retain all 39 commands and
causal details. The inventory SHA-256 is
`f6871826868b37a9fb45a89fd02eb7defb7c7ca01f2d2cf921fc968be14b79a4`.

## Draft publication and remaining gates

On 2026-10-21, Wilson authorized pushing the feature branch and opening an
explicitly blocked draft PR, with the failed and unexecuted daemon checks disclosed.
Canonical `main` was rechecked at `ab46aa1f13de5663c684225dc27dfcba330d965f` before
publication preparation. No competing open PR was found.

Draft PR [#79](https://github.com/styrene-lab/styrene-rs/pull/79) was published at
`1a953ba9c342c1308e9bf58e6d2047e972ccd43d`. The publication receipt confirms draft
state, exact head/body, and no automatic issue-closing reference.
The tested implementation candidate remained `02643d7c`; the publication delta
contained only evidence/acceptance-documentation updates.

## First PR-head CI result

At head `1a953ba9`, CI completed with 11 passed checks, one failure, and one
cancellation. No checks remained pending. The PR stayed open and draft.

- Linux passed the complete daemon library selection: 579 passed and three ignored.
  All 23 selected integration targets ran and passed, with 202 top-level passes.
  Logs confirm that the Perl-dependent cleanup case entered its body and passed.
  This provides the compatible-host coverage missing from the NixOS run. It does
  not repair or erase the local daemon host-layout limitation.
- The new no-std job failed after incomplete fresh acquisition. Target-only
  `cargo fetch` omitted `curve25519-dalek-derive 0.1.1`, which Cargo's graph
  inspection also loads through host-applicable edges. Offline tree inspection
  failed before the positive target checks. The independent negative control and
  artifact upload succeeded. This is a new CI preparation defect, not a passing gate.
- macOS was cancelled because GitHub could not allocate an arm64 hosted runner.
  No macOS tests executed. This is an infrastructure cancellation, not compatibility
  evidence or a product-test failure.

The failed no-std job is
[119235940040](https://github.com/styrene-lab/styrene-rs/actions/runs/39604150488/job/119235940040).
The Linux job is
[119235940355](https://github.com/styrene-lab/styrene-rs/actions/runs/39604150488/job/119235940355).
The cancelled macOS job is
[119235940060](https://github.com/styrene-lab/styrene-rs/actions/runs/39604150488/job/119235940060).

Sanitized remote receipts, job results, artifact evidence, and diagnostics:

```text
~/.local/state/styrene-lab/issue-78/publication/20261021T130139Z-ci-watch-ZMFjaH/
```

The user authorized a host-plus-target acquisition correction, an actually empty
isolated-cache regression, and a normal update push to this draft branch. No
dependency version, lockfile, validation-offline policy, or platform gate may be
weakened to make CI pass. A warm-cache rehearsal is not sufficient proof.

## Reviewed fresh-cache CI correction

Correction `417137ba8905e3b653edc9321ddc895f8c39b607`, based on published head
`1a953ba9`, adds `--target host-tuple` to the locked AArch64 fetch and updates
the fixture acquisition documentation. No Rust source, runner behavior,
dependency version, or lockfile changed.

The regression proved that Cargo home, home, temporary, and target directories
were empty before one authorized acquisition. Cargo 1.97 fetched 44 locked
packages, including `curve25519-dalek-derive 0.1.1`, without retries or warm-cache
supplementation. The archives totaled 2,425,197 bytes and matched fixture-lock
checksums.

Using only that acquired cache, validation remained offline and locked. All nine
tree/metadata/library checks across the three profiles passed. The intended
negative E0463, three allocator tests, all 17 runner tests, fixture formatting,
and diff checks also passed. Tested dirty-source hashes match the committed result.

Independent review approved the correction with no findings. It verified all
63 evidence checksums and equality of all 1,503 registry-file hashes between
the acquisition inventory, seed cache, and offline-check cache.

Evidence:

```text
~/.local/state/styrene-lab/issue-78/publication/20261021T133051Z-ci-preparation-fix-54chiz1v/
```

`receipt.json`, `cache-prestate-empty.json`, `acquired-packages.json`,
`check-evidence/`, and `SHA256SUMS` retain the command, source, toolchain, cache,
and lockfile proof. These local results do not replace the next PR-head CI run.

Case 008 remains partial pending macOS execution. Case 012 remains open after
the failed acquisition. Passing no-std and Linux/macOS CI on the corrected PR head
and final acceptance remain outstanding. Publication is not a readiness waiver.
Merge, release, issue closure, and archival remain separately approval-gated.

The [test plan](test-plan.md) defines case criteria. The
[execution plan](execution-plan.md) defines authority and required evidence.
Only the explicitly authorized branch push, draft PR, and CI monitoring may
proceed. This record does not authorize merge, issue closure, archive, or release.
