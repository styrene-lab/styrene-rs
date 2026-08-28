# Shared Dioxus Mobile UI Design

## Product Boundary

The mobile application is one Dioxus product with platform-adaptive packaging.
Its primary routes are Messages, People, Network, and More. Desktop-only Command,
Fleet, Propagation administration, unrestricted Content administration, Lab, and
System controls do not appear in the base mobile shell.

Desktop and mobile can reuse components and domain stores where their workflows
match. They do not need identical navigation chrome or information density.

## Application Structure

```text
Shared Dioxus components and routes
              |
Shared presentation stores and reducers
              |
styrene-session / styrene-ipc typed contracts
              |
Embedded Styrene runtime
              |
Rust platform-service traits
              |
Rust mobile implementations
```

The Rust application owns navigation, selection, drafts, filters, capability
presentation, message views, people views, and network views. The backend owns
identity, protocol, storage, routing, delivery, and transport truth.

## Platform Services

`styrene-ui-platform` defines typed Rust asynchronous services for:

- Bluetooth discovery, approval, reconnect, GATT access, and byte transport.
- Android USB discovery, permission, and byte transport.
- Keychain or Keystore-backed secret storage.
- Notification authorization, scheduling, and activation.
- Application lifecycle and background execution opportunities.
- Sharing, clipboard, camera, and platform settings links where required.

Adapters return typed availability, authorization, progress, completion, and
failure results. They do not mutate Dioxus stores directly. Shared Rust commands
apply adapter results to presentation state and backend sessions.

## Embedded Runtime

Dioxus mobile uses the shared Embedded session directly in Rust. It does not use
a local daemon socket unless a selected product mode requires one, and it does
not use a generated-language bridge as its daemon contract.

Platform RNode adapters exchange bounded byte or packet-channel events with the
Rust host boundary. Rust retains KISS framing, configuration semantics, outbound
retention policy, and attachment to the embedded node.

## State And Rendering

One reducer set consumes typed snapshots and events for both platforms. Platform
differences enter through explicit capability and layout inputs. Components do
not branch on platform names when a capability or adaptive layout class can
express the difference.

The UI keeps LXMF method, active bearer, and delivery evidence separate. It does
not derive one from another. Preview or fixture data remains visibly marked and
cannot enter live backend stores.

## Migration Slices

1. Package a fixture-only Dioxus shell on iOS and Android.
2. Connect Embedded session startup and read-only identity/network state.
3. Migrate conversation list, thread, composition, and delivery evidence.
4. Migrate People, settings, propagation entry points, and experimental pages.
5. Integrate Bluetooth, Android USB fallback, secure storage, notifications, and lifecycle in Rust.
6. Run release gates on the Dioxus application.

## Testing And Evidence

Renderer-neutral reducer and selector tests run on the host. Dioxus component
tests use deterministic empty, loading, ready, degraded, error, and
high-information fixtures. iOS Simulator and Android emulator run the same state
corpus and accessibility labels.

Physical tests are required for Bluetooth, Android USB, secure storage recovery,
notification activation, and lifecycle behavior that simulators cannot prove.
Evidence reports identify the exact application revision, backend revision,
device class, OS, bearer, scenario, and outcome.

## Failure And Rollback

A platform adapter failure degrades only the affected capability. It must not
fabricate backend state or crash unrelated workflows. Embedded startup failure
returns to a recoverable setup state after owned resources shut down.

If a workflow fails its release gate, the Dioxus route stays behind a development
gate. The backend and stored user data do not roll back with the renderer.
