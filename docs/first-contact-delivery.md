# First-contact delivery and deferred dispatch

Unnamed nodes publish canonical LXMF delivery metadata with a null display name.
Standalone, embedded, mobile and explicit identity announces use the same encoder.
A name remains optional; an empty application payload is no longer the default.

A newly inserted local message with unknown sender identity can enter a
session-scoped deferred-dispatch queue. Packet and resource receive paths share
the worker. Entries contain only a durable message ID, source destination and a
30-second monotonic deadline. The input channel and pending set each cap at 64
entries. The worker checks identity availability every 250 ms, with a 100 ms cap
on each identity lookup. Queue overflow and expiry leave the stored message
undispatched and report diagnostics.

When identity is available, the messaging service re-verifies exactly that
canonical stored wire message. Existing immutable-field, signature, stamp and
ticket checks remain authoritative. Only dispatchable records proceed to protocol
handlers and the existing auto-reply queue. Invalid signatures or stamps never
become auto-replies. The entry is removed before dispatch; duplicate wire packets
cannot re-enter because only the initial successful canonical insert can enqueue.
This is at-most-once dispatch within the session, not crash-recoverable exactly-once
execution. Old held messages are not replayed at startup.

Shutdown owns the deferred worker alongside packet, resource and response tasks.
No schema migration, identity rotation, change to signed bytes, or weakened
verification policy is introduced. A transport receipt still precedes application
acceptance and must not be presented as proof of bot execution.

Tests in `worker_inbound` exercise delayed identity for packet and resource,
correlated single echo, duplicate suppression, invalid signatures and timeout.
`lxmf_fidelity_storage` covers existing durable re-verification invariants.
