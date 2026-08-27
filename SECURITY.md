# Security Policy

Styrene handles persistent identities, authenticated network traffic, remote inputs, and service-control operations. Treat security defects as potentially consequential even when they appear to affect only availability or metadata.

## Supported versions

Until the project declares a stable support window, security fixes are made on the current `main` branch and included in the next release. Older development snapshots are not maintained as separate security branches.

| Version | Supported |
|---|---|
| Current release | Yes |
| `main` | Yes, for reproduction and upcoming fixes |
| Older snapshots | No |

## Reporting a vulnerability

Use GitHub's private vulnerability-reporting flow for this repository:

**[Report a vulnerability privately](https://github.com/styrene-lab/styrene-rs/security/advisories/new)**

Include, where possible:

- Affected commit or displayed build SHA (`styrene --version`)
- Affected platform and runtime profile
- Reproduction steps or a minimal proof of concept
- Expected and observed behavior
- Security impact and required attacker position
- Whether identity material or other secrets may have been exposed

Do not include active private keys, recovery material, production addresses, access tokens, or unredacted operator databases. If GitHub private reporting is unavailable, contact the maintainers through a private channel listed on the Styrene Labs organization profile rather than filing a public issue.

## Response expectations

The project will aim to:

1. Acknowledge a credible report within 3 business days.
2. Establish reproduction and severity before discussing disclosure timing.
3. Coordinate a fix and release appropriate to the impact.
4. Credit the reporter unless anonymity is requested.

These are operational targets, not a contractual service-level agreement.

## Scope

High-priority areas include:

- Identity generation, storage, import, derivation, and signing
- Packet, proof, announce, link, resource, IFAC, and ratchet validation
- IPC authentication and authorization boundaries
- Path traversal or arbitrary filesystem access
- Remote command, tunnel, terminal, page, and fleet-service boundaries
- Secret exposure through logs, crashes, UI output, or release artifacts
- Ghost/portable profile isolation and cleanup
- Dependency or release-pipeline compromise

## Current posture

- Rust stable is pinned exactly in `rust-toolchain.toml`.
- Workspace source forbids unsafe code by default.
- Protocol interoperability is tested against committed fixtures.
- Release workflows publish checksummed artifacts and provenance where configured.
- Background runtime diagnostics are separated from the TUI rendering surface.

Styrene has **not** yet received an independent comprehensive security audit. Do not interpret compatibility tests or internal review as a cryptographic certification.
