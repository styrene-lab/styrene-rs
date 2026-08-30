# Bounded Ingress - Delta Spec

## ADDED Requirements

### Requirement: Inbound traffic is prioritized and bounded

The transport must classify admitted packets into independently bounded FIFO queues for data, announces, path requests, and ingress-limited traffic, drained with strict priority and no fairness or aging.

#### Scenario: All classes contain traffic
Given every inbound traffic class contains an accepted packet
When the transport drains the next packet
Then data drains before announces, announces before path requests, and path requests before ingress-limited traffic
And FIFO order is preserved within each class

#### Scenario: Higher-priority traffic is sustained
Given a lower-priority queue is nonempty
And a higher-priority queue remains nonempty after every dequeue
When the transport performs repeated dequeue selections
Then every selected packet comes from the higher-priority queue
And the lower-priority queue may remain starved until all higher-priority queues are empty

#### Scenario: Sustained higher priority ends
Given a lower-priority queue was starved by sustained higher-priority traffic
When all higher-priority queues become empty
Then the next dequeue selects the oldest packet from the lower-priority queue

#### Scenario: A class queue is full
Given one traffic class has reached its configured capacity
When another packet of that class arrives
Then the new packet is dropped without blocking the interface worker
And accepted packets in all queues remain unchanged

### Requirement: Queue occupancy is observable

Runtime observations must expose each class's unsigned integer configured capacity, current depth, and monotonic cumulative drops from one consistent snapshot.

#### Scenario: Saturated traffic is inspected
Given packets have filled and overflowed one inbound class
When an operator or test requests ingress statistics
Then the snapshot reports that class's exact depth, capacity, and cumulative drops
And another class's counters are not incremented

### Requirement: Control-plane state remains bounded

Path-request state must preserve canonical limits: a 16,000-entry replay-tag rotation threshold whose retained crossing entry produces a 16,001-entry previous generation, one in-flight gate per destination eligible for pruning after 45 seconds, at most one waiter per registered interface for a destination, and at most 32 pending discovery transmissions.

#### Scenario: Unique path requests exceed limits
Given unique tagged path requests cross the 16,000-entry generation threshold and the pending discovery queue reaches 32 entries
When additional requests arrive
Then replay tags rotate current to previous and a new pending discovery item is not enqueued
And in-flight pruning after the 45-second threshold or count-based tag-generation rotation releases the corresponding state
