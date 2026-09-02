# Reticulum 1.5.2 Empty Carrier Parity Design

## Authority

Canonical behavior is taken from Reticulum 1.5.2 commit `ea98db4f53dcf0defc0e71a16e60d28b1229c4e6`. The generator verifies that revision and inspects each canonical carrier's leading empty-input guard before emitting bounded JSON evidence.

## Admission Boundary

Empty input is a carrier event, not a malformed RNS packet. UDP and HDLC stream workers therefore discard it before IFAC processing, packet deserialization, drop accounting, queue insertion, or transport mutation. Non-empty malformed input continues through the existing fail-closed admission path.

UDP uses a pure admission function for ordinary tests. The socket-backed worker test exercises the same function but is ignored unless network access is explicitly requested.

## Integration

The additive `rns-1.5.2` authority and empty-carrier vector use fixture index schema version 2. Existing authority records, vectors, paths, and checksums are not changed. Broader live handoffs remain represented by the existing interop-runner-owned Reticulum 1.5 handoff; this maintenance behavior requires no new live claim.
