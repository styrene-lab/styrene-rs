# Beechat RNS Corrections Wave

## Intent

Close the remaining protocol and transport gaps evidenced by the complete
`BeechatNetworkSystemsLtd/Reticulum-rs` range
`20bb433d9934071ff9652a5ee3b9ddf92ef51aea..151e3b6c77a8c7d33fafa3971a084ae02510ef39`
without importing Beechat's architecture. Beechat is MIT lineage and implementation evidence;
canonical Python Reticulum 1.5.1 at
`149e4151095adf098b8f53eab0c03b37169e8559` is the protocol authority.

## Scope

### Included

- MessagePack `f64` LinkRTT interoperability with canonical Python Reticulum
- Early Type-2 next-hop admission and loop-free shared-medium forwarding
- An explicit, non-panicking `no_std` wall-clock contract for announce and ratchet time
- IPv4 UDP broadcast socket enablement when a forwarding target is configured
- A bounded discard/failure policy for TCP client traffic submitted while disconnected
- Focused Rust and canonical-fixture regression evidence for each correction, consuming the
  provenance authority established by `reticulum-1-5-parity-wave`
- Reconciliation with the current consolidation branch at implementation time

### Excluded

- Directly merging, rebasing, or cherry-picking Beechat commits
- Advancing `.upstream-tracking.json`, editing sync logs, or changing reference remotes
- Creating another top-level Python Reticulum pin, fixture manifest, or provenance authority
- Replacing the raw frame admission, received-hop, bounded-ingress, or fixture decisions owned by
  `reticulum-1-5-parity-wave`
- Already-present announce timestamp/retry/update behavior, link lifecycle and identification,
  link message proofs, channels, request/resource work, and LinkRequest/Proof generic-rebroadcast
  suppression
- Beechat daemon layout, crate split, Kaonic removal, examples, CI, documentation, and optional
  flexible-routing/restart/announce-forever product policies

## Success criteria

- Rust and pinned Python exchange LinkRTT packets encoded as MessagePack `f64`
- A node that overhears a non-announce Type-2 packet for another next hop performs no routing,
  cache, link-table, delivery, or egress action
- A designated next hop forwards a shared-medium packet once without a LinkRequest ping-pong loop
- `styrene-rns` builds without default features and timestamp-dependent operations fail explicitly
  until the embedding application supplies time
- IPv4 UDP forwarding to a broadcast address succeeds with broadcast permission enabled
- A disconnected TCP client cannot stall healthy interfaces or replay a stale connection epoch
  after reconnect
