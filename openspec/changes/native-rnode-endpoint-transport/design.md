# Native RNode Endpoint Transport Design

## Boundaries

`styrene-rns` owns byte framing, RNode command sequencing, bounded queues, exact
readback validation, and serial lifecycle. `styrened` owns configuration parsing,
interface construction, and the endpoint retransmission policy.

The existing host-driven mobile RNode bridge remains authoritative for iOS and
Android. Native serial support is a daemon capability behind the existing
`serial` dependency boundary and must not become part of minimal mobile builds.

## Startup

The daemon validates all enabled interface profiles before starting transport.
For each native RNode profile it opens the configured serial path, sends the
radio configuration in deterministic order, and waits for exact readback before
marking the interface online. Payload writes before readiness are rejected or
held only within existing bounded queues; they are never sent as radio data.

The runtime applies `transport_retransmit` to the core transport configuration.
The setting defaults to `true` for compatibility. Endpoint deployments may set
it to `false` while retaining a `full_node` local runtime for endpoint services.

## Safety

Frequency, bandwidth, power, spreading factor, and coding rate are validated
before serial I/O. Diagnostics identify the failed command or setting but omit
serial identifiers, addresses, credentials, payload bytes, and key material.
Embedded diagnostic writes remain best-effort and cannot panic a transport path.

Shutdown cancels workers, attempts a bounded radio-off command, closes the
serial stream, and remains idempotent after partial startup failure.

## Verification

Pure protocol tests cover fragmented command responses, readback mismatch,
payload gating, framing bounds, and shutdown. Daemon tests cover TOML defaults,
invalid profiles, enabled-interface selection, and retransmission propagation.
Feature checks prove default and serial builds while retaining mobile-minimal
build coverage in CI.

The source implementation was accepted against a physical Station G2 in the
lab. That deployment evidence is recorded without committing its serial path or
other stable device identifiers; this consolidation does not claim a fresh
physical-device run.
