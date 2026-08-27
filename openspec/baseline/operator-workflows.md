# operator-workflows - Baseline

### Requirement: Operator workflows expose authoritative lifecycle state

Messages, fleet jobs, propagation work, and content requests must display daemon-reported stages, terminal outcomes, and correlated errors.

#### Scenario: Message delivery changes state
Given an outbound message exists
When receipt, retry, resource, propagation, or terminal events arrive
Then the Messages page updates one message lifecycle
And exposes available method and correlation details

#### Scenario: Remote page identity cannot be resolved
Given path discovery completes for a remote page host
And identity resolution fails
When the Content page receives the outcome
Then it shows identity resolution as the failed stage
And offers retry and diagnostic inspection without remaining in loading state

#### Scenario: Privileged fleet action is denied
Given the caller lacks a required capability
When a privileged fleet action is considered
Then the console disables or rejects the action with the required capability
And records no misleading success state

#### Scenario: Propagation is disabled
Given propagation support is disabled or unavailable
When the Propagation page opens
Then it explains the missing capability or configuration
And does not fabricate queue, peer, or synchronization state

### Requirement: Unsupported daemon operations remain explicit

The console must derive workflow availability from negotiated daemon capabilities and must not emulate unsupported daemon behavior in the view layer.

#### Scenario: Daemon operation is not implemented
Given the active daemon reports an operation as unsupported
When the related page or action renders
Then the console marks the operation unavailable with the daemon reason
And does not issue a substitute protocol operation

#### Scenario: Capability changes after reconnect
Given the console reconnects to a daemon with different capabilities
When workflow controls are rebuilt
Then availability reflects the new connection generation
And stale capabilities cannot authorize an action
