# Operational Embedded Mobile Node

## Intent

Make `styrened::mobile::MobileNode` a complete embedded LXMF node when configured with a hub transport. A Swift, Kotlin, or Rust host must receive the node's actual delivery destination, announce its configured display name, process inbound messages and peer announces through the same service workers as the daemon, and shut down its background work explicitly.

This closes the immediate runtime gap blocking applications such as 4con from attaching to the operational `Daemon` interface on phones.

## Scope

### Included

- Publish the configured mobile node's LXMF delivery destination through `IdentityService` and a direct `MobileNode` accessor.
- Normalize and encode `display_name` into delivery announce application data and local identity state.
- Start inbound packet/resource, announce, and link workers during mobile boot.
- Retain all worker handles, abort them when the node is dropped, and provide asynchronous explicit shutdown that also invokes transport shutdown.
- Preserve the no-hub offline boot path without inventing a routable destination.
- Unit tests with an injected operational transport and host-level tests for real boot metadata.
- Keep UniFFI behavior compatible while routing its shutdown operation through the explicit lifecycle API.

### Excluded

- New Bluetooth, BLE, local Wi-Fi, AutoInterface, or platform interface configuration APIs.
- Propagation message signature-policy changes.
- Android Keystore custody, encrypted-file passphrase design, or iOS background scheduling.
- Changes to LXMF wire format, daemon IPC, game protocols, or discovery policy.
- A claim that hub TCP is the only or preferred phone transport; interface selection is the next mobile transport change.

## Success Criteria

- A hub-configured mobile boot exposes the same nonzero delivery hash used by its transport destination.
- The configured normalized display name is available from `IdentityService` and included in outgoing announce application data.
- Injected inbound, announce, and link events are consumed after boot by the corresponding service workers.
- Dropping a `MobileNode` aborts every retained worker; explicit shutdown additionally calls `MeshTransport::shutdown` and is safe to call once.
- Booting without a hub remains valid, reports no delivery destination, and does not claim connectivity.
- Existing mobile FFI methods continue to compile and its shutdown path invokes managed node shutdown.
- Package formatting, warning-denied Clippy, and the complete `styrened` test suite pass.
