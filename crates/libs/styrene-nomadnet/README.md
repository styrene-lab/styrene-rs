# styrene-nomadnet

Internal NomadNet browsing domain library. It owns page addressing, Micron
projection, native form/response encoding, browser sessions and history, page
cache, file downloads and explicit saves, and reference-counted link cleanup.

## Boundaries

| Module | Responsibility |
|---|---|
| `coordinator` | Browser state, absolute operation deadlines, cancellation, bounded retention and cleanup |
| `models` | Domain requests, observations and page/download results; no serde or IPC contract |
| `address` (re-exported) | Canonical native page/file addressing |
| Crate root | Content projection, redacted form submission and native binary-response decoding |

The coordinator accepts `Arc<dyn BrowseBackend>` and `Arc<dyn Discovery>`. Ports
use RNS foundation types, domain observations and Tokio cancellation/deadlines.
They do not implement routing or establish their own runtime. The host supplies
an active Tokio runtime. The library performs explicit file saving on request.

`styrened::services::native_browse` supplies the MeshTransport/discovery adapters
and retains the selected local RNS private identity. The domain asks whether
identification is enabled and asks the adapter to identify a link; it never holds
the local private key. The daemon facade retains authorization and composition.
`nomadnet_conversion` maps fields to and from unchanged public IPC DTOs.

The public legacy IPC address API remains available because public IPC cannot
depend on this internal crate. Its behavior is checked against the domain parser
in integration tests. Keep those compatibility tests current when addressing changes.

The crate has no daemon, IPC, UI or styrene-session dependency. Micron parsing stays
in styrene-micron; route/link repair stays in styrene-rns. Native page hosting and
dynamic page execution remain in the daemon PageService; this extraction covers
the browsing coordinator, not the host's filesystem/execution configuration.

## Validation

```sh
cargo test --locked -p styrene-nomadnet
cargo test --locked -p styrened --lib services::native_browse
cargo test --locked -p styrened --test nomadnet_split
cargo clippy --locked -p styrene-nomadnet -p styrened --lib --tests -- -D warnings
just check-workspace-policy
```

The coordinator tests own scripted lifecycle/cancellation cases. Daemon adapter
tests own transport request cancellation. `nomadnet_split` exercises the facade,
a real local Unix IPC connection, shared links, observation metadata, download
saving, owner isolation, authorization and address compatibility with controlled
transport responses. It does not prove public-mesh or hardware interoperability.

This behavior-preserving extraction does not fix cache expiry, failed-route
recovery or the desktop's observation deadline. See the coordination checkout's
`docs/browse-reuse-audit.md` for those separate changes. The crate remains unpublished.
