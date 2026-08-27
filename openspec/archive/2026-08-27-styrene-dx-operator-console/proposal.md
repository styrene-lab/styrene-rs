# Styrene DX Operator Console

## Intent

Turn the experimental `styrene-dx` Dioxus spike into a dependable operator console and protocol-testing scaffold without duplicating daemon or interoperability logic in the UI.

## Product Planes

- **Operate** monitors and controls a real Styrene deployment.
- **Lab** launches deterministic scenarios, exposes protocol milestones, and retains evidence.
- **Admin** inspects identities, interfaces, policy, storage, and runtime behavior, and changes them only through negotiated typed daemon contracts.

All planes use the same typed domain stores and backend-session contract. Lab-only mutation and fault-injection controls are never exposed implicitly in Operate mode.

## Scope

### Included

- Replace implicit IPC-to-embedded fallback with explicit Live, Embedded, and Fixture runtime profiles
- Split the monolithic application into routed pages, domain stores, a request broker, and backend sessions
- Preserve and correct the network graph while separating discovery, route, link, and interface truth
- Add Command, Network, Messages, Fleet, Propagation, Content, Lab, and System pages
- Provide a global activity timeline and contextual inspectors
- Reuse daemon IPC contracts and the pinned interoperability harness for Lab scenarios
- Expose role-aware confirmations and audit outcomes for privileged or destructive actions
- Add deterministic fixtures, component tests, scenario tests, and desktop smoke coverage
- Remove warning-denied Clippy failures in `styrene-dx`

### Excluded

- Moving protocol behavior from `styrened` into the UI
- Creating a second scenario orchestrator or shell-backed protocol harness
- Claiming a discovered peer is a direct route or active link without corresponding daemon evidence
- Browser deployment, mobile packaging, or replacing `styrene-tui` in this change
- Implementing daemon capabilities that currently return `NotImplemented`
- Pixel-perfect visual redesign before page and state boundaries are stable

## Dependencies

- Typed `styrene-ipc` request, response, and event contracts
- The pinned upstream interoperability harness for executable Lab scenarios
- Daemon support for incremental events or cursored snapshots where full-list polling is too expensive
- Existing `styrene-micron` parsing for NomadNet-compatible content

## Success Criteria

- Operators explicitly select or configure a runtime profile; a failed Live connection never silently starts a full node
- Independent requests and event subscriptions do not wait behind one long-lived bridge mutex
- Discovery, routes, links, and interfaces are visibly and structurally distinct
- Every primary page has loading, empty, ready, degraded, and error states
- Lab scenarios use the same topology definitions, deadlines, revision pins, cleanup, and evidence format as CLI/CI interop gates
- Privileged actions show capability requirements, confirmation, outcome, and correlation evidence
- Fixture mode can exercise every primary page without network access or a running daemon
- `cargo test -p styrene-dx`, warning-denied Clippy, desktop launch smoke, and applicable interop scenarios pass

## Delivery Slices

1. **Foundation:** runtime profiles, backend sessions, request broker, routed shell, stores, and fixture mode.
2. **Network truth:** Command and Network pages, typed observations, event timeline, and corrected graph semantics.
3. **Operator workflows:** Messages, Content, Fleet, Propagation, and System pages.
4. **Protocol Lab:** scenario catalog, controls, milestone timeline, assertions, evidence, and replay.
5. **Hardening:** permissions, accessibility, performance budgets, recovery, tests, and release gates.
