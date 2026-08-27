# Mobile Reticulum Transport Profiles

## Intent

Allow two embedded `MobileNode` instances to communicate directly over a local TCP network without a propagation hub or centralized game service. One mobile host must be able to listen on a TCP interface, expose its actual bound address, and accept another mobile node configured as a TCP client. Both nodes then use ordinary Reticulum announce, path, link, and LXMF delivery behavior.

This provides the concrete no-hub transport needed by peer applications such as 4con while preserving optional hub TCP and propagation configuration.

## Scope

### Included

- Explicit TCP server and TCP client interface profiles in Rust `MobileConfig`.
- Support for multiple configured TCP interfaces on one embedded transport.
- Ephemeral TCP server ports with actual bound-address reporting to the host.
- Existing `hub_address` mapped into the same TCP-client startup path for current mobile FFI consumers.
- UniFFI records for TCP interface profiles and bound-listener access.
- Validation for empty addresses, duplicate profiles, and configurations that cannot bind a server.
- Managed shutdown of all configured interface tasks through the existing transport lifecycle.
- A real two-node test proving announce discovery and bidirectional LXMF delivery with no hub configured.

### Excluded

- UDP, broadcast, multicast, AutoInterface, BLE, Bluetooth Classic, serial, and platform peer-discovery APIs.
- NAT traversal, internet rendezvous, centralized matchmaking, or propagation-node delivery.
- QR and camera UX, local-network permission prompts, and mobile app packaging.
- IFAC configuration; UDP lacks IFAC support and TCP IFAC policy needs a separate credential design.
- Removing the existing `hub_address` field while shipped FFI callers still depend on it.

## Success Criteria

- A mobile TCP server configured with `127.0.0.1:0` reports its actual nonzero bound port before boot returns.
- A second mobile node connects as a TCP client without either node having `hub_address` or `hub_delivery_hash` configured.
- After both nodes announce, each resolves the other's delivery destination through normal Reticulum path discovery.
- Each node sends an LXMF message directly to the other and receives correct content and source attribution.
- Duplicate or empty interface profiles fail validation before transport startup.
- Explicit node shutdown stops configured interfaces and retained service workers.
- Rust mobile tests, the complete `styrened` suite, focused warning-denied Clippy, and mobile FFI compilation pass.
