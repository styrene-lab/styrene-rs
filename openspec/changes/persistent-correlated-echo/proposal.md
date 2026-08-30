# Persistent correlated echo service

## Intent

Provide an operator-controlled echo response mode that survives restart and safely responds to accepted LXMF packet and resource messages, while allowing failed direct sends to retain their identity and fall back to opportunistic delivery.

## Scope

This change covers daemon configuration, service and IPC projections, canonical inbound handling, and persisted router fallback. It preserves the canonical inbound persistence owner and legacy event adapter; it does not restore the legacy inbound writer or change generated/platform artifacts.

## Success criteria

- Echo configuration is backward compatible, persistent, queryable, and mutable through IPC.
- Only accepted, trusted, non-duplicate application messages are echoed to their exact 16-byte LXMF source destination, with a correlated loop-prevention marker.
- A direct send may use the same persisted message, correlation, attempt, and deadline for opportunistic fallback when the stripped wire fits one packet.
