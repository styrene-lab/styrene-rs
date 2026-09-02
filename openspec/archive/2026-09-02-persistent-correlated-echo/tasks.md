## 1. Persistent echo configuration
<!-- specs: lxmf-echo-routing -->

- [x] Add backward-compatible persisted modes and service conversion tests
- [x] Apply settings in both production roots and persist IPC updates
- [x] Verify facade and IPC query/set echo behavior

## 2. Safe canonical echo handling
<!-- specs: lxmf-echo-routing -->

- [x] Add structured-field outbound messaging support
- [x] Echo accepted trusted packet and resource content with correlation marker
- [x] Test loop, protocol, duplicate, trust/stamp, and destination rejection

## 3. Persisted direct fallback
<!-- specs: lxmf-echo-routing -->

- [x] Persist actual opportunistic method and fallback reason without replacing message identity or deadline
- [x] Dispatch eligible destination-stripped fallback in initial and restart/retry lifecycle paths
- [x] Test eligibility, lifecycle identity, persistence, and restart projection

## 4. Verification
<!-- specs: lxmf-echo-routing -->

- [x] Run formatting, focused tests, package checks, and Clippy
- [x] Validate the OpenSpec change
