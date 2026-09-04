# Mobile Peer Identity And Link

## Intent

Give the mobile node's peer projection enough to group announces by the
identity behind them and to say what the link to a peer looks like.

## Scope

The peer projection and one new query. A `MobilePeer` now carries the hex
address hash of the announcing identity plus the hops and interface kind of
the announce that produced it, and `MobileNode::peer_link` reports whether a
destination is reachable right now and over what. The announcing identity and
route are observed by the announce worker, persisted with the peer record, and
projected through the discovery device contract, so a restart keeps them.

It excludes any change to announce acceptance, to how peers are keyed
(the announced destination hash stays the key), and to the mobile shell that
renders peers.

## Success criteria

- A received announce yields a peer whose identity hash is the announcing identity's, distinct from the destination hash, with the announce's hop count and interface kind.
- A peer persisted before this change still projects, with an empty identity hash and no route.
- `peer_link` reports an unknown or malformed destination as unreachable rather than as an error, and reports hops and interface kind for a destination with a path.
