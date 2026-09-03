# Announce On Connect - Delta Spec

## ADDED Requirements

### Requirement: A mobile node announces on connect

The mobile node must announce its delivery destination once after its
transport connects at start and once after every reconnect, and must not
announce while disconnected.

#### Scenario: Peer restarts
Given a mobile node connected to a peer
When the peer restarts and the node's transport reconnects
Then the node announces again
And the peer can verify the node's next message

### Requirement: Unverified inbound is explained and repaired

When an inbound message is held because the sender's identity is unknown, the
daemon must log the message id, the sender, and the reason, and must request
the sender's path.

#### Scenario: Unknown sender
Given an inbound message from a sender with no known identity
When the message is accepted but not dispatchable
Then a diagnostic names the message and sender
And a path request for the sender is sent
