# Operational Mobile Node Tasks

## 1. Mobile identity and destination metadata
<!-- specs: mobile-runtime -->
- [x] 1.1 Add failing tests for transport-backed delivery hash publication and offline absence
- [x] 1.2 Factor shared mobile service composition and publish the actual transport destination
- [x] 1.3 Add failing tests for normalized, invalid, and omitted display names
- [x] 1.4 Wire normalized identity state and standard delivery announce application data

## 2. Operational workers
<!-- specs: mobile-runtime -->
- [x] 2.1 Add failing tests proving mobile composition retains inbound, announce, and link workers
- [x] 2.2 Return ownership of both packet and resource tasks from the inbound worker
- [x] 2.3 Start and retain all mobile service workers before boot returns
- [x] 2.4 Inject lifecycle events and verify the mobile link worker emits typed daemon events

## 3. Managed shutdown
<!-- specs: mobile-runtime -->
- [x] 3.1 Add failing tests for worker cancellation on drop and transport shutdown on explicit shutdown
- [x] 3.2 Implement idempotent worker abortion and consuming asynchronous `MobileNode::shutdown`
- [x] 3.3 Expose delivery and connectivity accessors for embedded hosts
- [x] 3.4 Route UniFFI shutdown through the managed asynchronous node shutdown operation

## 4. Verification
<!-- specs: mobile-runtime -->
- [x] 4.1 Run focused mobile and worker tests plus the complete styrened suite
- [x] 4.2 Verify `styrened` formatting
- [x] 4.3 Verify the modified styrened library and mobile test with all features and Clippy warnings denied
- [x] 4.4 Compile the mobile FFI crate against the managed lifecycle API
