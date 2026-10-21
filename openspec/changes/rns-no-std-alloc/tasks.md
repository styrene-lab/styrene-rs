# Implementation and verification tasks

Every checkbox includes its applicable validation. Case IDs refer to
[test-plan.md](test-plan.md) and the delta spec. The coordinator alone updates
completion after reviewing evidence. [execution-plan.md](execution-plan.md)
assigns work packages, exclusive paths, handoffs, and delivery gates.

## 1. Resolve the supported contract
<!-- specs: rns-core-portability -->

- [x] Resolve D1: record the initial no-std target, atomic requirements, and explicit Cortex-M scope decision. User approval and Rust 1.97 target configuration confirm the selected atomic-capable envelope. (`RNS-NOSTD-003`)
- [x] Resolve D2: select the executable harness, allocator/startup approach, runner, and permitted environment provisioning. Independent architecture review approved the fixture-only boundary with conditions recorded in the design. (`RNS-NOSTD-011`)
- [x] Resolve D3: review caller-RNG signatures, state ownership, hosted compatibility, and entropy failure policy. The approved borrowed-state/legacy-compatibility contract is recorded in the design. (`RNS-NOSTD-004`, `RNS-NOSTD-005`)
- [ ] Capture the failing isolated dependency/target baseline at the exact base revision without modifying existing development checkouts. (`RNS-NOSTD-001`, `RNS-NOSTD-003`)

## 2. Establish the allocation-backed dependency boundary
<!-- specs: rns-core-portability -->

- [ ] Make `rmp-serde` and `serde_bytes` optional under RNS `std`. Verify persistence and transport still select them. (`RNS-NOSTD-001`, `RNS-NOSTD-009`)
- [ ] Disable the relevant RNS dependency defaults while retaining alloc, derive, crypto, and zeroization capabilities. (`RNS-NOSTD-001`, `RNS-NOSTD-010`)
- [ ] Move OS randomness activation out of the allocation-backed profile and retain it for hosted convenience APIs. (`RNS-NOSTD-001`, `RNS-NOSTD-004`)
- [ ] Preserve the minimal, explicit-alloc, default/std, transport, and serial feature relationships. Review resolved sources and lockfile changes. (`RNS-NOSTD-002`, `RNS-NOSTD-008`)
- [ ] Run focused workspace-policy and consumer checks without widening dependency or publication scope. (`RNS-NOSTD-010`)

## 3. Complete and verify entropy plumbing
<!-- specs: rns-core-portability -->

- [ ] Add regression tests proving supplied RNG consumption and advancing state across core operations. (`RNS-NOSTD-004`)
- [ ] Add the reviewed caller-RNG group-encryption entry point and retain hosted wrappers. (`RNS-NOSTD-004`, `RNS-NOSTD-008`)
- [ ] Pass the supplied RNG into private ratchet rotation and verify announce/rotation behavior. (`RNS-NOSTD-004`, `RNS-NOSTD-006`)
- [ ] Remove artificial entropy dependencies from group and ratchet decryption using existing authenticated primitives. (`RNS-NOSTD-005`)
- [ ] Verify wrong-key, tampered, truncated, authenticated-padding, and buffer-error cases without weakening verify-before-decrypt behavior. (`RNS-NOSTD-005`, `RNS-NOSTD-007`)
- [ ] Document and test the agreed RNG/provider contract, including any required `Copy`-bound or borrowing changes. (`RNS-NOSTD-004`)

## 4. Preserve clock, protocol, and hosted behavior
<!-- specs: rns-core-portability -->

- [ ] Adapt embedded-time tests to the reviewed provider boundary. Retain missing/cleared time and shared-clock rotation checks. (`RNS-NOSTD-006`)
- [ ] Run canonical committed fixture suites and applicable alternate-Fernet tests without copying or regenerating the authority corpus. (`RNS-NOSTD-007`)
- [ ] Verify default, explicit-std, transport, and supported serial checks and tests. Record platform-specific limits. (`RNS-NOSTD-008`)
- [ ] Verify existing persistence encoding, private atomic replacement, and key-fallback failure behavior. (`RNS-NOSTD-009`)
- [ ] Check affected LXMF, mesh, tunnel, and daemon consumers and review retained crypto features. (`RNS-NOSTD-010`)

## 5. Add isolated target and execution gates
<!-- specs: rns-core-portability -->

- [ ] Add an independent downstream consumer fixture with reviewed workspace-policy treatment, resolver, toolchain, and lockfile ownership. (`RNS-NOSTD-001`, `RNS-NOSTD-002`)
- [ ] Add the approved true-target library matrix for minimal, alloc, and alternate-Fernet profiles. (`RNS-NOSTD-003`)
- [ ] Add and execute the bounded target smoke workflow with synthetic state and owned cleanup. (`RNS-NOSTD-011`)
- [ ] Add a disposable negative control that introduces target-side `std` and verify the intended gate failure. (`RNS-NOSTD-012`)
- [ ] Wire the target gate into CI and a documented repository command without replacing existing host checks. (`RNS-NOSTD-003`, `RNS-NOSTD-012`)
- [ ] Retain source, lockfile, toolchain, feature, target, command, and result evidence for each case. (`RNS-NOSTD-012`)

## 6. Review completion and compatibility
<!-- specs: rns-core-portability -->

- [ ] Update crate support documentation with the accepted allocator, entropy, time, atomic, and API contract and its exclusions. (`RNS-NOSTD-002`, `RNS-NOSTD-003`, `RNS-NOSTD-004`)
- [ ] Review every acceptance case against retained evidence. Leave failed, blocked, and unexecuted requirements open. (`RNS-NOSTD-001` through `RNS-NOSTD-012`)
- [ ] Run applicable repository validation and targeted OpenSpec validation. Prepare the compatibility and evidence summary before requesting merge or archival. (`RNS-NOSTD-007` through `RNS-NOSTD-012`)
