# Reticulum 1.5.2 Empty Carrier Parity

## Intent

Adopt Reticulum 1.5.2's carrier-level empty-input guard without reopening the archived 1.5.1 parity wave. The immutable authority is `markqvist/Reticulum` commit `ea98db4f53dcf0defc0e71a16e60d28b1229c4e6`.

## Scope

Included:

- Ignore zero-byte UDP datagrams and decoded zero-byte HDLC frames before packet admission.
- Preserve fail-closed handling and violation accounting for non-empty malformed frames.
- Add revision-pinned, checksummed evidence to the existing RNS fixture index.

Excluded:

- Receipt, resource, MTU, token, discovery, and path-gate behavior already integrated on `main`.
- Live network parity claims. The socket-backed UDP worker test remains explicitly network-gated.

## Success criteria

- Empty carrier units produce no packet and no protocol-violation observation.
- A valid packet following an empty carrier unit is still delivered.
- Existing 1.4.2 and 1.5.1 fixture authority records and vectors remain unchanged.
- Default offline tests, transport tests, warning-denied Clippy, fixture provenance, and OpenSpec validation pass.
