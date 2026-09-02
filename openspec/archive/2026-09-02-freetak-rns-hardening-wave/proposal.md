# FreeTAK RNS Hardening Wave

## Intent

Close security, failure-containment, resource-lifecycle, interface-policy, and platform-byte-transport gaps found by reviewing FreeTAKTeam/LXMF-rs from `3a2d46bbea174a1049d5d3e06f00c6ea20254085` through `0ed96f7ee33cefe7fe6eb188b8094b02cd536193`. The reference is implementation evidence only. Styrene will reproduce required behavior independently in its own architecture and retain immutable provenance without importing copyleft source or fixtures.

## Scope

Included:

- Constant-time `CachedFernet` authentication verification
- Private, atomic, symlink-resistant key and ratchet persistence with secret-safe diagnostics
- Availability-only key-manager fallback and poisoned receipt-map recovery
- Adversarial non-RTT Link control state-mutation hardening and bound-interface Link sends
- Resource retry accounting, round-based requests, admission caps, and split-transfer terminal cleanup
- Supervision of transport workers and bounded passive-node announce retention
- RNS 1.5 internal-interface announce policy as an authority-owned behavior slice
- A low-level, bearer-neutral ordered-byte attempt trait and shared RNode protocol engine inside `styrene-rns`

Excluded:

- Copying source, tests, fixtures, comments, naming, or structure from FreeTAKTeam/LXMF-rs
- General Reticulum/LXMF/NomadNet parity, live interoperability gates, and ordinary channel/resource scheduling already owned by `reticulum-lxmf-nomadnet-parity`
- Canonical RNS 1.5 token vectors, MTU discovery/negotiation, generic Link-close cancellation of all resources, and requested-window search offsets owned by `reticulum-1-5-parity-wave`; this wave consumes those results and owns only the distinct cached-verifier and round/retry/cap/split gaps
- LinkRTT wire bytes, precision, parsing, and validation owned by `beechat-rns-corrections-wave`; this wave begins Link state-mutation hardening only after that work and does not add RTT codecs or validation
- The broader RNS 1.5 ingress scheduler, batching, adaptive expiry, discovery, Backbone, carrier-state, status, and API migration
- BLE, RFCOMM, USB, serial, or other platform implementations, reconnect, permissions, application lifecycle, physical acceptance, and mobile bridge work owned by existing mobile plans; this wave defines only the low-level attempt boundary and shared RNode protocol engine
- Tracking-marker, reference, or existing OpenSpec changes

## Success criteria

- Every listed gap has a deterministic regression that fails before its minimal implementation and focused verification after it
- Invalid cryptographic, persistence, Link, receipt, and resource inputs fail without leaking secrets, reviving state, acknowledging unauthenticated data, or silently rerouting work
- Worker failure and every resource terminal path produce bounded cleanup and an attributable terminal outcome
- Passive nodes retain announce packets only in bounded persistence-capable storage, while transport nodes preserve retransmission behavior
- Internal-interface announce decisions match the pinned RNS 1.5 authority for the covered policy rows and preserve permissive defaults
- Platform-owned byte bearers can drive one shared RNode/KISS runtime with cancellation-safe close and no bearer-specific protocol duplication
- Repository validation proves the new OpenSpec is structurally valid before implementation begins
