# Protocol Parity Claims

`product/capabilities-v1.toml` is the machine-readable authority for protocol parity claims. Product capability status describes whether an implementation exists. It does not prove upstream compatibility.

## Current claims

| Claim | Level | Current evidence boundary |
|---|---|---|
| `rns.primitives` | Experimental | Python-derived fixtures exist, but their original generation revision was not recorded. |
| `rns.operations` | Experimental | Rust end-to-end tests and a manual Python script exist. There is no automated routed Python/Rust gate. |
| `lxmf.codec` | Experimental | Codec coverage is Rust-only. |
| `lxmf.direct` | Degraded | Direct and Opportunistic Python-to-Rust scenarios are ignored and manual. Rust-to-Python is not covered. |
| `lxmf.resources` | Experimental | Rust resource tests exist. Bidirectional Python/Rust resource delivery is not covered. |
| `lxmf.propagation` | Unsupported | Standard destination, offer, retrieval, policy, and persistence behavior exists, but the required enabled Python/Rust interoperability gate is absent. |
| `micron.rendering` | Experimental | Parser coverage exists. Fixture provenance and renderer assertions are incomplete. |
| `nomadnet.transport` | Unsupported | Native static and bounded dynamic host/client paths exist, but canonical Python fixtures and bidirectional pinned NomadNet gates are absent. |

## Evidence rules

- A Rust-only test proves internal behavior.
- A fixture proves only the covered bytes against its recorded source.
- A manual or ignored test cannot produce a verified claim.
- A Styrene-specific protocol cannot prove native LXMF propagation or NomadNet transport.
- A verified claim requires every required gate to be automated, enabled, non-ignored, native, and tied to a pinned upstream revision.

The pinned upstream revisions define the current assessment target. Existing legacy fixtures remain marked `legacy-unrecorded` until they are regenerated from those revisions.
