# Independent RNS no-std fixture

This fixture implements the validation lane for issue #78. Its contract is
[`rns-no-std-alloc`](../../../openspec/changes/rns-no-std-alloc/design.md).
The target is `aarch64-unknown-none` with allocation, 64-bit atomics and
pointer-width atomics. All generated keys and packets are synthetic.

## Run existing tools

Use Python 3.11+, matching Cargo/rustc **1.97.0**, installed target libraries,
and a reviewed registry cache. The runner copies registry data into each run's
own Cargo home. It never copies Cargo configuration or credentials.

```sh
python3 -B scripts/check-rns-no-std-tests.py

just check-rns-no-std "$PINNED_CARGO" "$PINNED_RUSTC" "$REVIEWED_CARGO_HOME"

just execute-rns-no-std "$PINNED_CARGO" "$PINNED_RUSTC" "$REVIEWED_CARGO_HOME" "$QEMU"
```

The path arguments must select existing tools and cache data. Execution requires
QEMU **11.1.0** and the Rust toolchain's bundled `rust-lld`. The runner validates
versions and target libraries. Missing prerequisites cause failure.

| Mode | Evidence produced |
|---|---|
| `check` | Allocator arithmetic tests on the host, intended negative diagnostic, feature trees, metadata, and target library checks |
| `execute` | All `check` results, then separate panic/OOM controls and three linked synthetic target executions |
| `bootstrap` | Allocator arithmetic tests and boot-only/panic/OOM execution, without compiling RNS |
| `prepare` | External materialization only, for explicit CI cache acquisition |

Call `scripts/check-rns-no-std.py` directly for `bootstrap` or `prepare`.
Use the same arguments as `execute` for `bootstrap`. `prepare` needs only
`--cargo` and `--rustc` and prints its scratch directory to stdout.

For a coordinator-supplied read-only product snapshot, also pass
`--product-source /absolute/checkout --product-revision <full-40-character-SHA>`
directly to the script. That checkout must be clean at the exact revision.
The runner records and monitors product and fixture-source fingerprints
separately. It writes no files into either checkout. By default, both inputs
come from the checkout containing the runner.

The positive matrix is no default features, `alloc`, and `alloc,fernet-aes128`.
The separate negative consumer explicitly imports target `std`. Its expected
result is rustc **E0463**, naming `std` and that consumer's source. A missing
`core`, package, lockfile, or unrelated compilation failure does not pass it.
CI performs explicit locked cache acquisition before offline validation. CI
does not require QEMU.

## Fixture and lock ownership

`Cargo.toml.in` files are templates, not product workspace members. The runner
materializes them below a unique `/tmp/opencode/rns-no-std-*` directory. The
fixture owns resolver 3, edition 2024, Rust 1.97 and `publish = false`. It resolves
RNS through an exact source path and the `^0.1.0` version requirement, with
defaults disabled. Product Cargo configuration is not inherited.

`smoke` is a safe `no_std` library. It checks a deliberately non-Copy/non-Clone
ChaCha provider, key generation, signatures, packet encoding, group/identity/
ratchet encryption and decryption, supplied time, and rotation. It also compiles
downstream legacy trait implementations and an immutable legacy Fernet caller.
The public-identity encryption check preserves the approved existing key slicing.
The complete compatibility and negative-crypto corpus remains owned by RNS tests.

`boot` is a separate, non-public test package. It defaults to denying unsafe
code. Only `startup`, `allocator`, and `semihosting` permit their documented
unsafe operations. `smoke` and product code continue to forbid unsafe code.

The fixture lock was intentionally refreshed offline against reviewed product
revision `27cf0af68d5b58a30f9f94ff5ec5e3217c880679`. The refresh removed nine
packages and obsolete dependency edges without changing versions or checksums.
Every registry version and checksum also occurs in the product lockfile.
ChaCha 0.3.1 and rand_core 0.6.4 have defaults disabled in the fixture.
A lockfile can include unselected platform dependencies. It is not the resolved
target feature graph.

After integrating dependency changes, the coordinator must review lock
compatibility. If Cargo requires a lock refresh, generate it deliberately in an
external materialization. Review its package/source/checksum changes before
replacing this fixture's lock. Validation always uses `--offline --locked` and
never updates a product lockfile. The negative consumer has its own empty-graph
lockfile. The fixture's independent workspace is outside the product workspace
policy membership check; the runner tests verify its equivalent package policy.

## Boot contract for independent review

The only supported machine is QEMU `virt-11.1`, Cortex-A53, TCG single-thread,
one CPU and 128 MiB RAM. There are no network, display, serial or monitor
connections. Native semihosting uses fixed strings and process exit only.

`boot/src/boot.S` requires EL1 with the MMU and caches initially disabled. It
masks DAIF, selects SP_EL1, sets exception vectors and enables FP/NEON before
Rust. It zeroes BSS and two translation-table pages. A 39-bit, 4 KiB-granule
translation maps exactly `0x40000000..0x48000000` using 64 L2 blocks. These blocks
are Normal, write-back, inner-shareable RAM with access flags set. Atomics
therefore run with the MMU enabled and valid Normal-memory attributes.

`boot/linker.ld` starts the ELF at `0x40080000`. Ordered, aligned sections contain
text/data, BSS, 8 KiB of page tables, a 16 MiB heap and a 1 MiB stack. Linker
assertions enforce the upper memory bound and non-overlap. Review each retained
ELF and linker map before accepting execution evidence.

The allocator checks alignment, addition overflow and the exclusive heap end.
Successful reservations advance an atomic cursor. Exhaustion returns null.
Deallocation is a no-op for this finite workload. Storage is never reset or
reused. Allocator operations neither allocate recursively nor unwind.

The panic handler emits one fixed failure string and exits 1. The pinned
no-std allocation-error runtime reaches that handler on OOM. Each fault control
emits its own armed marker immediately before the intended operation. Acceptance
requires that marker, the failure marker, and exit 1 in that order. An allocation
that returns emits `OOM-RETURNED` and exits 3. Target regression controls force
that return path and an earlier unarmed panic; neither can satisfy a fault check.
All these diagnostics and exits are allocation-free.
Exception vectors emit a fixed trap string and
exit 2 without using the stack. Neither path emits a completion marker.

Every QEMU execution has a 60-second deadline, configurable from 1 to 120 seconds.
The runner kills and reaps its own process group on timeout or cancellation.
Its scoped SIGTERM handler stops the active group, restores the previous signal
handler, and records cancellation before returning exit 143. Command logs and
cancellation status are retained when evidence finalization can complete.
Build commands have a 600-second deadline, two Cargo jobs and 64 MiB log limits.
Success requires the exact scenario marker and exit 0. Boot-only success uses
a different marker and cannot establish product success.

## Evidence and limits

The runner preserves logs, materialized manifests, source fingerprints, lockfile
hashes, exact commands/environment overrides, tool paths/versions/hashes, target
libraries, artifact hashes and linker maps below:

```text
~/.local/state/styrene-lab/issue-78/runs/validation/<run-id>/
```

Each summary is labeled unreviewed until the independent reviewer accepts it.
Source mutation invalidates a run. Symlinked or source-overlapping output roots
are refused. Input checks reject symlinks in the product crate and fixture roots,
their ancestors and descendants before resolving aliases or reading content.
Resolved inputs must remain within their source root. Regular Git worktree
`.git` pointer files retain their normal Git semantics.
The runner does not retain ambient environment variables, private
identities, credentials, or traffic. Review retained summaries before sharing.
Scratch and retained evidence require an explicit cleanup decision.

Bootstrap success proves only the fixture substrate. Target compilation,
linking, synthetic QEMU execution and physical-device acceptance are separate
evidence levels. Full product results require the integrated A+B+C revision.
This fixture does not establish physical-board, radio, transport or Apple-host
acceptance.
