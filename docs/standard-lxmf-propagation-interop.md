# Standard LXMF Propagation Interoperability

## Scope

This document records live interoperability evidence for the public Brutus hub.
The evidence applies to image `ghcr.io/styrene-lab/styrened-hub:9f65f66` on 2026-08-28.

The public endpoint was `rns.styrene.io:4242`. Its propagation destination was
`780e7aa7b2f175c88f28c7ba8ab1b714`. The deployment preserved delivery identity
`1d423b7a0a0ec4f6111480aa1910d58d` across rollout and restart.

## Python LXMF Evidence

The client used these pinned upstream revisions:

- RNS `1.4.2` at `b48b96e61676504e0a4e527b33b9a0b4495c6872`
- LXMF `1.1.0` at `795fdaa2b0777c13033787d933d1afc94a2377cb`

The client connected through the public endpoint and decoded an active
`lxmf.propagation` announce. The announce contained these values:

- Name: `Styrene Community Hub`
- Costs: `[16, 3, 18]`
- Transfer limit: `256 KB`
- Synchronization limit: `4000 KB`

The client then uploaded one propagated LXMF message and replayed the same
encrypted payload. Brutus acknowledged both transmissions. The identified
recipient fetched one message, and the plaintext matched the sent content. The
first synchronization returned one message and zero client-side duplicates.
After cleanup acknowledgement, the second synchronization returned zero
messages.

A second run restarted the Brutus deployment after upload and before retrieval.
The client TCP connection closed and reconnected. The recipient then fetched the
one persisted message. The post-acknowledgement synchronization returned zero
messages. The replacement pod used image `9f65f66`, became ready, and had zero
container restarts.

These runs provide live evidence for discovery, upload, transient-ID duplicate
suppression, identified retrieval, cleanup acknowledgement, and queue persistence
across process replacement. They do not provide production capacity or expiry
evidence. OpenSpec task `8.8` remains incomplete for those gates.

## Skywave Evidence

Skywave `1.0` build `5`, with Reticulum `0.9.4`, connected to the public IPv4
endpoint. Brutus repeatedly decoded its canonical `lxmf.delivery` announces as
`FPIG_SKYWAVE` at destination `e01b09b22ccc4e2755d29eead962677b`.

The observed Skywave UI supports TCP connectivity and local propagation hosting.
It did not expose third-party propagation-node selection or synchronization.
Therefore, this evidence proves Skywave TCP and announce interoperability only.
It does not prove Skywave upload, retrieval, acknowledgement, or duplicate
suppression against Brutus.
