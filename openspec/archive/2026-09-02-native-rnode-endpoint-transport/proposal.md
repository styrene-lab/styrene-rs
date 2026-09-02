# Native RNode Endpoint Transport

## Intent

Make the validated Station G2 deployment reproducible from canonical `styrened`
without weakening existing transport defaults or replacing mobile host-driven
RNode sessions.

## Scope

- Add an opt-in native serial RNode interface backed by the shared bounded RNode
  protocol engine.
- Validate and apply frequency, bandwidth, transmit power, spreading factor,
  coding rate, and radio state before accepting payload traffic.
- Add daemon configuration for native RNode interfaces and transport packet
  retransmission.
- Preserve retransmission as enabled by default; allow endpoint deployments to
  disable transit retransmission explicitly.
- Exclude platform UI, generated bindings, packaged applications, device
  credentials, deployment addresses, and stable serial identifiers.

## Success criteria

- A configured native RNode reaches ready state only after exact command
  readback and then carries ordinary RNS packets over bounded KISS framing.
- Invalid radio profiles fail before transport startup with actionable,
  credential-free diagnostics.
- `transport_retransmit` defaults to `true` and configures every daemon transport
  constructor consistently.
- Existing TCP, UDP, serial KISS, mobile host-channel, and minimal feature builds
  continue to pass their established validation.
