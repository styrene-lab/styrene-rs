# Mobile Transport Profiles Tasks

## 1. Configuration and validation
<!-- specs: mobile-transports -->
- [x] 1.1 Add failing tests for empty, malformed, duplicate, and legacy-hub duplicate profiles
- [x] 1.2 Implement normalized TCP server/client profiles and pre-start validation
- [x] 1.3 Update Rust and UniFFI mobile configuration conversion

## 2. Direct TCP startup
<!-- specs: mobile-transports -->
- [x] 2.1 Add a failing test for ephemeral server bound-address reporting
- [x] 2.2 Spawn ordered TCP server and client profiles on one shared RNS transport
- [x] 2.3 Await server bindings with a bounded startup timeout and expose them to Rust and UniFFI hosts
- [x] 2.4 Preserve offline null transport and legacy hub-client behavior

## 3. Direct LXMF delivery
<!-- specs: mobile-transports -->
- [x] 3.1 Add a failing two-mobile-node no-hub discovery test
- [x] 3.2 Announce both delivery destinations and await peer resolution
- [x] 3.3 Add and satisfy bidirectional direct LXMF content and source-attribution assertions

## 4. Interface lifecycle
<!-- specs: mobile-transports -->
- [x] 4.1 Add failing cancellation tests for interface manager and mobile shutdown
- [x] 4.2 Add interface-manager cancellation without disturbing active transport changes
- [x] 4.3 Dispatch interface cancellation from `TokioTransportAdapter::shutdown`

## 5. Verification
<!-- specs: mobile-transports -->
- [x] 5.1 Run focused direct-mobile tests and the complete styrened suite
- [x] 5.2 Verify formatting for modified files
- [x] 5.3 Run focused warning-denied Clippy
- [x] 5.4 Compile all-feature mobile UniFFI bindings
