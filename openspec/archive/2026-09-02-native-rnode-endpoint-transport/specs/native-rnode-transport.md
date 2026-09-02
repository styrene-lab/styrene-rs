# Native RNode Transport - Delta Spec

## ADDED Requirements

### Requirement: Native RNode payloads require verified configuration

An enabled native RNode interface must not transmit or accept payload traffic as
ready until every configured radio parameter has been read back exactly.

#### Scenario: Configuration readback matches
Given a valid native RNode profile and an available serial device
When the RNode acknowledges every command with the configured value
Then the interface becomes online
And subsequent bounded KISS data frames are exchanged as RNS packets

#### Scenario: Configuration readback differs
Given an RNode returns a value different from the configured radio profile
When startup processes the response
Then the interface does not become online
And no payload is transmitted as radio data

### Requirement: Native radio profiles fail closed

Frequency, bandwidth, effective transmit power, spreading factor, and coding
rate must be validated before opening the serial device.

#### Scenario: Effective power is unsupported
Given a profile whose configured power is outside the supported RNode range
When daemon configuration is validated
Then startup returns a configuration error
And the serial device is not opened

### Requirement: Endpoint retransmission is explicit

Daemon packet retransmission must remain enabled by default and must be
independently configurable for endpoint deployments.

#### Scenario: Existing configuration omits the setting
Given a daemon configuration without `transport_retransmit`
When transport starts
Then packet retransmission is enabled

#### Scenario: Endpoint disables retransmission
Given a daemon configuration with `transport_retransmit = false`
When transport starts
Then the local node may receive and originate traffic
And the core transport does not retransmit transit packets

### Requirement: Native transport diagnostics are non-sensitive and best-effort

Native RNode operational failures must not expose payloads, stable serial
identifiers, credentials, network addresses, or key material, and diagnostic
output failure must not panic transport processing.

#### Scenario: Serial failure occurs during shutdown
Given an active native RNode interface whose serial write fails
When shutdown attempts to turn off the radio
Then shutdown closes the attempt without panicking
And retained diagnostics contain no serial path or payload bytes
