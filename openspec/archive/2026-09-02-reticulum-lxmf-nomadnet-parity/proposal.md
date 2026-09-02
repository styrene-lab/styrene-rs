# Reticulum, LXMF, and NomadNet Parity

## Intent

Establish and meet evidence-backed baseline parity for ordinary Reticulum operations, LXMF messaging and propagation, and NomadNet page browsing and serving. Protocol claims must describe behavior proven against the upstream Python implementations, while the operator console must expose daemon-authoritative state without inferring unsupported behavior.

## Scope

### Included

- Native Reticulum request paths, client requests, path discovery, links, resources, receipts, and truthful runtime observations
- Authoritative LXMF delivery-method selection, message lifecycle, conversations, resources, stamps, and standard propagation-node behavior
- Native NomadNet `nomadnetwork.node`, `/page/...`, and `/file/...` interoperability plus Micron browsing workflows
- Production composition equal to tested composition for page, propagation, receipt, and retry handlers
- Capability negotiation, observation provenance, correlated operations, and parity-oriented operator workflows
- Bidirectional, revision-pinned Python/Rust interoperability gates with retained evidence
- Explicit claim levels that distinguish primitives, direct messaging, propagation, Micron rendering, and native NomadNet transport

### Excluded

- Every Python Reticulum interface family, utility, or callback surface
- NomadNet publishing and site administration before browsing parity is stable
- Fault injection in ordinary Operate workflows
- Treating Rust-only, Fixture, or Styrene-specific transport tests as upstream interoperability evidence
- Maintaining duplicate protocol implementations in `styrene-ui`

## Success criteria

- A deterministic three-node topology proves routed path discovery, delivery, receipts, route loss, and recovery
- Python and Rust exchange direct, opportunistic, and resource-backed LXMF messages in both directions with authoritative lifecycle outcomes
- Rust interoperates with Python LXMF propagation for advertise, offer, fetch, persistence, expiry, and offline delivery
- Python NomadNet fetches Rust pages and files, and Rust fetches and renders Python NomadNet pages through native RNS requests
- Production startup registers the same required protocol handlers exercised by interoperability tests
- Operator controls derive from negotiated capabilities and report source, generation, freshness, correlation, and terminal outcomes
- Unsupported claim levels remain explicit in product metadata, diagnostics, and UI
- Ordinary validation remains offline; pinned live interoperability runs only in its dedicated gate
