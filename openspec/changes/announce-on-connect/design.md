# Design

## Why the phone got no echo

The echo peer verifies a sender's identity before it dispatches or answers a
message. It learns identities from announces. The phone announced once, at
first start, to whatever it was connected to then. When the peer restarted
it forgot the phone, the phone reconnected without announcing, and every
later message from the phone arrived unverified. The peer stored it, marked
it untrusted, and said nothing.

## Connect-announce worker

`workers::announce::spawn_connect_announce_worker` subscribes to the transport
lifecycle. If the transport is already connected it announces once after a
short settle; then it announces after every `Connected` or `Reconnected`
event. Disconnects and lagged events do nothing. The mobile node spawns it
beside its other workers and aborts it with them. The daemon binary is
unchanged: its announce schedule is a command-line interval.

## Unverified inbound

When `inbound_is_dispatchable` is false on either the packet or the resource
path, the worker now resolves the sender's identity; if it is unknown it logs
`held: sender identity unknown, requesting path` and requests the path, so
the sender's next message can verify. If the identity is known and the
message is still untrusted, it logs `held: authentication or stamp untrusted`.
The drop event and the storage outcome are unchanged.
