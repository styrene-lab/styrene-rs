# RNS Wire Corrections - Delta Spec

## ADDED Requirements

### Requirement: LinkRTT uses canonical floating-point precision

LinkRTT payloads must encode finite non-negative RTT seconds as a MessagePack 64-bit float and
must interoperate with canonical Python Reticulum 1.5.1. Fixture evidence must come from the shared
`reticulum-1-5-parity-wave` provenance authority rather than an independent pin or manifest.

#### Scenario: Python RTT activates the inbound link
Given canonical Python Reticulum emits an encrypted LinkRTT payload containing a MessagePack 64-bit float
When Rust receives the payload on the matching pending inbound link
Then Rust decodes the complete value without precision-width failure
And the link records the RTT and completes its canonical activation transition

#### Scenario: Rust RTT is consumed by Python
Given Rust has established an outbound link with a finite measured RTT
When Rust emits the encrypted LinkRTT payload
Then the decrypted MessagePack value is encoded as a 64-bit float
And pinned Python Reticulum accepts the value as the link RTT

#### Scenario: Invalid RTT is rejected
Given a LinkRTT payload is malformed, negative, non-finite, or has trailing data
When Rust processes the payload
Then the payload does not activate or refresh the link
And no invalid duration is stored

### Requirement: Only the designated next hop admits transported packets

Before queue insertion, duplicate caching, routing mutation, cryptographic work, delivery, or
egress, transport must reject every non-announce packet whose transport ID does not equal the local
transport identity.

#### Scenario: Shared-medium packet is overheard
Given a Type-2 non-announce packet identifies another transport instance as its next hop
When the local node receives the packet on a shared medium
Then the local node drops it before mutable transport state changes
And the local node emits no forwarded or rebroadcast packet

#### Scenario: Designated next hop receives the packet
Given a Type-2 non-announce packet identifies the local transport instance as its next hop
When the local node receives the packet
Then the packet enters the canonical Reticulum 1.5.1 processing path once
And any forwarding uses only the selected route and interface policy

#### Scenario: Transported announce retains announce semantics
Given a Type-2 announce carries a transport ID different from the local transport identity
When the local node receives the announce
Then the non-announce next-hop gate does not reject it solely for that transport ID
And announce validation and path policy decide its outcome

### Requirement: Shared-medium forwarding is loop free

Ingress processing must not generically rebroadcast LinkRequest, Proof, or other routed packets in
parallel with their dedicated routing paths.

#### Scenario: Link request crosses a shared relay
Given an origin, designated relay, overhearing peer, and destination share a broadcast medium
When the origin sends a routed LinkRequest through the relay
Then the designated relay forwards the request once toward the destination
And neither the origin nor overhearing peer starts a forwarding ping-pong loop
