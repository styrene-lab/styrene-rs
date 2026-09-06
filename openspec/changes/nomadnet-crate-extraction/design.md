# Design

Dependency direction: styrened -> styrene-nomadnet -> styrene-micron and native
MessagePack support. The domain crate must not depend on IPC, daemon, UI or session
runtime crates. Keep it internal (`publish = false`).

Gate 1 extracts synchronous content behavior from native_browse.rs. Domain field,
submission, link and warning types belong to the new library; explicit daemon
adapters convert existing IPC DTOs. Preserve password redaction, field ordering,
submission limits, native MessagePack bytes and strict binary-response consumption.
Existing daemon tests continue to verify the adapter and full coordinator path.

The workspace prohibits public crates depending on internal crates. Therefore IPC
cannot re-export the new internal library. Keep its existing public DTOs and address
API unchanged; do not weaken publication policy to enable an early re-export.
Address ownership can move behind runtime conversions at the coordinator gate, with
an explicit compatibility strategy for the public IPC address API.

Gate 2 defines a domain BrowseBackend port, discovery capability lookup and operation
observations independent of IPC. Move sessions, cache, history, download ownership,
cleanup and their scripted tests to the library. Daemon adapters retain MeshTransport,
DiscoveryService, RNS identity selection and IPC projection. Prefer only port methods
used by this first consumer; no plugin mechanism, generic service framework or new
crate per DTO. Move in reviewable steps without two live coordinator implementations.

Gate 3 verifies composition with CLI/mobile IPC consumers and controlled fixture
requests. The desktop pin and installed daemon remain unchanged until a reviewed
immutable handoff. Correct TTL/no-cache, wait lifecycle and transport recovery in
separate changes, as recorded in the coordination repository's browse-reuse audit.

## Coordinator port contract to implement at gate 2

| Port responsibility | Required evidence/ownership |
|---|---|
| Discovery lookup | Distinguish unknown destination from known non-NomadNet destination. No automatic probes in the lookup. |
| Ensure path | Absolute deadline and cancellation; return route observation, not a reachability claim. |
| Resolve destination / open native link | Typed domain destination and link reference; retain Created versus Reused and wait for proven activation. |
| Identify link | Adapter uses selected RNS identity; domain must not acquire daemon identity custody or Styrene identity lifecycle responsibilities. |
| Submit / observe request | Correlation, receipt state, packet/resource result and bounded bytes. Do not replace authoritative receipt evidence with inferred UI success. |
| Release link | Close only owned references, supervise cleanup and retain failures for owner shutdown. |

Move observation models together with the coordinator, then convert them to existing
IPC types in styrened. Preserve generation/correlation and owner identifiers. A
session operation keeps one absolute deadline across stages. The existing
SessionReservation, UnretainedLink and ActiveRequest drop/cleanup behavior must
have equivalent tests before replacing the daemon coordinator.
