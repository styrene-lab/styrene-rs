# Reticulum 1.5 Parity Wave

## Intent

Close observable Reticulum core gaps introduced between canonical Python RNS 1.4.2 and 1.5.1. The authority is `markqvist/Reticulum` over the immutable range [`b48b96e61676504e0a4e527b33b9a0b4495c6872`](https://github.com/markqvist/Reticulum/commit/b48b96e61676504e0a4e527b33b9a0b4495c6872) through [`149e4151095adf098b8f53eab0c03b37169e8559`](https://github.com/markqvist/Reticulum/commit/149e4151095adf098b8f53eab0c03b37169e8559), inclusive of the latter and exclusive of the former for changed behavior.

## Scope

### Included

- Fail-closed packet and frame admission, canonical received-hop accounting, and protocol-violation observations
- Same-destination path-request batching, bounded tag and discovery state, egress limits, and slow-medium deadlines
- Configurable bounded priority ingress for data, announces, path requests, and ingress-limited traffic
- Link MTU discovery policy and the 1.5 link, resource, and receipt regressions that remain observable in Styrene
- Canonical interface-discovery implementation/version and optional operator LXMF metadata
- Constant-time token authentication and security-sensitive distinction between invalid and policy-blocked traffic
- Revision-pinned fixtures, differential tests, adversarial tests, and retained evidence
- The shared versioned RNS fixture authority and schema for additive 1.5.1 vectors, preserving all 1.4.2 entries for Beechat, FreeTAK, and Leviculum consumer waves

### Excluded

- LXMF messaging, propagation, NomadNet transport, and broad live routed gates already owned by `reticulum-lxmf-nomadnet-parity`; this change depends on its tasks 4.7, 5.7, 8.8, and 12.6 where live interoperability is required
- Python threading, profiler, Cython, logging, CLI formatting, packaging, utility-only, and generated-documentation changes
- New Python-compatible interface families, automatic interface connection, Backbone zero-copy transmit buffering, and throughput benchmark parity
- Changing upstream tracking markers, upstream refs, the pinned LXMF or NomadNet revisions, or existing OpenSpec artifacts
- Network access, Python process launch, serial hardware, or mutable upstream checkout use in ordinary tests

## Success Criteria

- Canonical and adversarial frames are accepted or rejected without truncation, panic, state mutation, or hop-count ambiguity
- Same-destination requests produce one bounded discovery operation and answer every eligible requester without bypassing ingress or egress controls
- Saturated ingress remains memory bounded, follows canonical strict-priority starvation semantics, and reports exact per-class capacity, depth, and cumulative drops
- MTU negotiation, link teardown, resource part handling, and receipt callbacks remain correct under mixed interfaces, concurrency, and malformed input
- Discovery metadata round-trips canonical 1.5.1 fields while invalid optional metadata fails closed
- All ordinary validation is deterministic and offline; pinned live evidence remains in the existing dedicated interoperability gate
- Beechat, FreeTAK, and Leviculum waves consume this wave's shared RNS fixture index instead of defining competing authority records or schemas
