# Styrene DX Operator Console Tasks

## 1. Foundation Contracts
<!-- specs: runtime-session -->
- [x] 1.1 Add failing tests for explicit profile selection, failed Live connection, and profile teardown
- [x] 1.2 Define `BackendSession`, runtime profile, connection generation, and capability contracts
- [x] 1.3 Add deterministic empty, healthy, degraded, high-cardinality, and active-scenario fixtures
- [x] 1.4 Replace warning-denied Clippy failures with used code, smaller boundaries, or narrowly justified allowances

## 2. Request Broker
<!-- specs: runtime-session -->
- [x] 2.1 Add correlation, timeout, cancellation, disconnect, stale-generation, and backpressure tests
- [x] 2.2 Split IPC reading, writing, event fanout, and in-flight request tracking
- [x] 2.3 Remove the global `Arc<Mutex<DaemonBridge>>` request bottleneck
- [x] 2.4 Record request latency, queue depth, reconnect, and dropped-update diagnostics

## 3. Application Shell And Stores
<!-- specs: app-shell -->
- [x] 3.1 Add Command, Network, Messages, Fleet, Propagation, Content, Lab, and System routes
- [x] 3.2 Add persistent runtime, identity, alert, activity, and context-inspector chrome
- [x] 3.3 Introduce domain stores and snapshot/event reducers with stale-generation rejection
- [x] 3.4 Add loading, empty, ready, degraded, and error fixtures for every route

## 4. Command And Network Truth
<!-- specs: network-observability -->
- [x] 4.1 Add graph-model tests distinguishing discovery, route, link, interface, and association edges
- [x] 4.2 Implement the Command summary from authoritative store selectors
- [x] 4.3 Split the network graph into model, layout, renderer, interaction, filters, and inspector components
- [x] 4.4 Add Discovery, Routes, Links, Interfaces, and Combined network modes with a legend
- [x] 4.5 Add high-cardinality responsiveness and incremental-update regression coverage

## 5. Messages And Content
<!-- specs: operator-workflows -->
- [x] 5.1 Migrate conversations and composition to `MessageStore` and typed commands
- [x] 5.2 Show delivery method, status, receipt, retry, resource, propagation, and correlated failure details
- [x] 5.3 Replace page URL splitting with typed local and remote page addresses
- [x] 5.4 Show page request stages, timing, bytes, source, rendered content, and actionable errors
- [x] 5.5 Add fixture and component tests for message and content lifecycle states

## 6. Fleet And Propagation
<!-- specs: operator-workflows -->
- [x] 6.1 Build capability-driven fleet inventory and job state
- [x] 6.2 Add confirmation and audit outcomes for execution, reboot, profile, block, and related privileged actions
- [x] 6.3 Build propagation disabled, queue, peer, sync, capacity, expiry, and failure views
- [x] 6.4 Add typed daemon contracts for missing incremental data instead of full-list polling workarounds
- [x] 6.5 Add permission-denied, unsupported, timeout, and partial-failure tests

## 7. Protocol Lab
<!-- specs: protocol-lab -->
- [x] 7.1 Define the UI-facing scenario catalog and `ScenarioBackend` contract
- [x] 7.2 Adapt pinned harness scenarios without duplicating topology or process supervision
- [x] 7.3 Add topology, controls, milestones, assertions, revision provenance, and evidence views
- [x] 7.4 Add cancellation, cleanup, rerun, and artifact-export behavior
- [x] 7.5 Run fixture scenarios in ordinary tests and pinned live scenarios only in the dedicated interop gate

## 8. System, Safety, And Accessibility
<!-- specs: operator-safety, app-shell -->
- [x] 8.1 Add profile, identity, interface, policy, storage, and diagnostics settings
- [x] 8.2 Enforce Operate/Lab control boundaries and capability-aware destructive actions
- [x] 8.3 Prevent Fixture networking and implicit Embedded production-port binding
- [x] 8.4 Add keyboard navigation, focus, labeling, contrast, and reduced-motion coverage
- [x] 8.5 Verify redaction in activity, diagnostics, and evidence export

## 9. Verification
<!-- specs: runtime-session, app-shell, network-observability, operator-workflows, protocol-lab, operator-safety -->
- [x] 9.1 Run `cargo test -p styrene-dx`
- [x] 9.2 Run warning-denied Clippy and formatting checks
- [x] 9.3 Run Fixture, Live-failure, and Embedded desktop launch smoke tests
- [x] 9.4 Run applicable pinned Lab interoperability scenarios
- [x] 9.5 Run ordinary workspace validation without Python or network dependencies
