# Extraction verification — 2026-09-06

Backend base: ecc26b68 (content gate), following original base
cdeda5c80d7cacf1811d325239e97d4c980ba773. This completes the browsing-coordinator
extraction. No duplicate runtime coordinator remains in the daemon.

## Final ownership

- Domain crate: address parsing, page projection, forms, binary response decoding,
  sessions/history/cache, downloads/saves, cancellation and link-reference cleanup.
- Daemon: DiscoveryService/MeshTransport adapters, local private identity selection,
  transport request receipt polling/cancellation, authorization and IPC conversion.
- RNS: transport implementation and route/link recovery policy.
- Micron: parser implementation.
- PageService: host-side filesystem configuration, serving and dynamic execution;
  outside the browsing-coordinator extraction.

Public IPC DTO declarations and serialization are unchanged. `models.rs` has no
serde derives; explicit field conversions preserve observations and owner/session
identifiers. The domain never retains the selected local private identity. The
legacy public IPC address API is retained and parity-tested; publication rules
prevent making it depend on the internal domain crate.

## Final checks

| Check | Result |
|---|---|
| `cargo test --locked -p styrene-nomadnet -p styrened --lib` | 43 domain tests; 547 daemon tests passed, 3 ignored |
| `cargo test --locked -p styrened --test nomadnet_split --test nomadnet_fixtures --test nomadnet_pages_offline --test daemon_facade_contract` | 40 passed: 5 new split tests, 6 fixture tests, 1 offline-serving test, 28 facade tests |
| `cargo clippy --locked -p styrene-nomadnet -p styrened --lib --tests -- -D warnings` | Passed |
| `cargo check --locked -p styrened --lib --no-default-features` | Passed on macOS host |
| `cargo fmt --all -- --check` | Passed |
| Workspace dependency/publication policy and its 9 tests | Passed |
| OpenSpec validation | Passed |

The 33 coordinator tests moved from styrened, retaining private lifecycle assertions.
Four transport request/drop tests remain with the daemon adapter. Five original
content tests and five address tests run independently in the domain crate.

New split tests verify:
- Native page navigation through the daemon facade, Back/cache behavior, password
  omission, owner mismatch rejection and no transport use for local content.
- Native transport receipt conversion including all three generation fields, selected
  identity use, shared link references across two sessions, download completion,
  explicit atomic save and cross-owner access rejection.
- A real Unix IPC server/client negotiation, page fetch and session close using the
  extracted coordinator.
- Legacy IPC/domain address compatibility on accepted and rejected inputs.
- Authorization denial before domain/transport work.

Remote data in the split test comes from queued transport fixtures. The first run
exposed that MockTransport did not implement native link methods; the fixture now
explicitly delegates those methods to its queued link/identity behavior and supports
queued request receipts. This is fixture coverage, not proof of a remote link handshake.

Evidence is retained by the coordination checkout under
`lab/runs/nomadnet-extraction/`. No daemon deployment, desktop revision pin update,
crates.io publication, remote push, Nucleus validation or mobile device validation
was performed. Host compilation is not Apple/Android device evidence. Cache expiry,
failed-route recovery and UI wait policy remain separate follow-up changes.
