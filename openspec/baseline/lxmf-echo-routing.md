# lxmf-echo-routing - Baseline

### Requirement: Persistent automatic response configuration

The daemon persists automatic response mode as `disabled`, `all`, `first_only`, or `echo`, defaults missing configuration to `disabled`, applies it in every production composition root, and exposes the active value through IPC query and update operations.

#### Scenario: Existing configuration has no automatic response section
Given a configuration written before automatic response persistence existed
When the daemon loads the configuration
Then automatic responses are disabled

#### Scenario: IPC enables echo mode
Given a daemon with a writable configuration path
When an authorized client sets automatic response mode to echo
Then the active service reports echo mode
And the configuration on disk reports echo mode after restart

### Requirement: Correlated safe echo responses

The canonical inbound owner may echo the unchanged content of an accepted packet or completed-resource message only after authentication and stamp policy accept it. Protocol messages, marked echo responses, duplicates, rejected messages, and malformed source destinations never produce an echo. Each echo contains a structured `styrene_echo` response marker and inbound message request ID.

#### Scenario: Trusted packet is echoed
Given a trusted accepted application packet with a 16-byte LXMF delivery source
When echo mode handles the packet
Then one response is addressed to the unchanged source hash
And its content is unchanged
And its `styrene_echo` marker identifies the inbound message

#### Scenario: Trusted resource is echoed
Given a trusted accepted completed-resource application message
When echo mode handles the resource
Then one correlated response is sent through the normal messaging lifecycle

#### Scenario: Unsafe inbound does not echo
Given an inbound message that is a protocol message, marked response, duplicate, trust reject, stamp reject, or has a malformed source
When the canonical inbound owner handles it
Then no echo response is sent

### Requirement: Persisted direct fallback

A failed direct delivery may fall back to opportunistic delivery only when its destination-stripped LXMF wire fits the opportunistic packet limit. The fallback retains one message, requested method, correlation ID, attempt, and original deadline while persisting the actual method and fallback reason for runtime and restart projections.

#### Scenario: Direct delivery falls back
Given a persisted direct message whose stripped wire fits one opportunistic packet
When direct delivery fails before acceptance and opportunistic dispatch succeeds before the original deadline
Then the same message and correlation ID are reported as sent
And the requested method remains direct
And the actual method and persisted restart projection are opportunistic with a fallback reason

#### Scenario: Oversized direct delivery does not fall back
Given a direct message whose stripped wire exceeds the opportunistic packet limit
When direct delivery fails
Then opportunistic delivery is not attempted
And the direct failure is terminalized normally
