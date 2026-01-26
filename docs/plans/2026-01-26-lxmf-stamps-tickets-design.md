# LXMF Stamps/Tickets Verification Parity Design

**Date:** 2026-01-26

## Goals & Scope

This phase delivers strict verification parity for LXMF stamps and tickets while deferring stamp creation/cost logic. The Rust implementation should parse Python-generated stamp/ticket bytes and return the same accept/reject verdicts. Scope includes parsing, validation rules, error classification, and golden fixtures for valid/invalid cases. Non-goals: minting stamps, dynamic cost policies, or router policy integration beyond a verify-only gate.

## Architecture & Components

- **`stamp` module**: Defines `Stamp` with `from_bytes` and `verify` methods. Parses the Python wire format (version/tag, signing key bytes, payload bytes, signature) and exposes deterministic validation.
- **`ticket` module**: Defines `Ticket` with `from_bytes` and `validate`. Parses ticket fields (type, recipient, expiry/epoch, cost/amount) and validates against a context (e.g., current time, recipient).
- **Verifier entrypoints**: Either a `StampVerifier` trait or free functions that take raw bytes and return `Result<Stamp, StampError>` plus a verification verdict, mirroring Python error modes.
- **Fixtures**: Golden byte fixtures under `tests/fixtures/python/lxmf/` for valid and invalid stamps/tickets.
- **Tests**: `tests/stamp_parity.rs` and `tests/ticket_parity.rs` cover parse/verify and error variants.

The verification layer remains pure and side-effect-free so router logic can call it later without coupling to cost/creation policy.

## Data Flow

1. Message fields provide stamp/ticket bytes.
2. `Stamp::from_bytes` / `Ticket::from_bytes` parse bytes into typed structures.
3. Validation runs locally and returns `Ok` or a typed error.
4. Router/handlers consume the verdict (accept/reject) but do not perform creation or accounting in this phase.

## Error Handling

Use small, stable enums:

- `StampError`: `InvalidFormat`, `InvalidSignature`, `UnsupportedVersion`, `InvalidRecipient`, `Expired` (if applicable)
- `TicketError`: `InvalidFormat`, `InvalidRecipient`, `Expired`, `UnsupportedVersion`

Tests assert on error variants rather than string messages.

## Testing Strategy

- Valid fixtures: parse + verify succeeds.
- Invalid fixtures: signature mismatch, invalid recipient, expired ticket.
- Optional round-trip parse/serialize if format supports it.

## Out of Scope

- Stamp creation/minting
- Cost/difficulty policy enforcement
- Router admission policy beyond verification verdict
