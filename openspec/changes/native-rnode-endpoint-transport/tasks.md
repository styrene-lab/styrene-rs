# Native RNode Endpoint Transport Tasks

## 1. RNode Protocol And Serial Lifecycle
<!-- specs: native-rnode-transport -->

- [x] 1.1 Adapt the bounded RNode command engine to the current interface MTU metadata contract
- [x] 1.2 Add the opt-in native serial interface with exact readback gating and bounded KISS framing
- [x] 1.3 Keep diagnostics redacted and best-effort across startup, I/O, and shutdown failures
- [x] 1.4 Add protocol, framing, mismatch, fragmentation, and idempotent shutdown tests

## 2. Daemon Configuration And Composition
<!-- specs: native-rnode-transport -->

- [x] 2.1 Add validated native RNode interface configuration without persisting stable device identifiers in repository fixtures
- [x] 2.2 Add `transport_retransmit` with a compatibility-preserving default of `true`
- [x] 2.3 Apply retransmission policy and native interfaces in every production daemon transport constructor
- [x] 2.4 Add parsing, validation, default, disabled-policy, and enabled-interface tests

## 3. Verification
<!-- specs: native-rnode-transport -->

- [x] 3.1 Run focused RNode, daemon config, and transport tests with the serial feature
- [x] 3.2 Run default and mobile-minimal feature checks, formatting, warning-denied Clippy, and offline policy validation
- [x] 3.3 Validate OpenSpec and record physical Station G2 acceptance as deployment evidence rather than a committed identifier
