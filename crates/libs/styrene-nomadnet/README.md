# styrene-nomadnet

Internal NomadNet domain library. The first extraction owns Micron projection,
form submission encoding and native binary-response decoding. `styrened` uses
these functions through an explicit IPC conversion adapter.

The crate depends only on `styrene-micron` and `rmpv`. It has no daemon, IPC, UI,
network or runtime dependency. Public IPC DTOs remain in `styrene-ipc`; the domain
types here are not a new wire format. Native requests still use MessagePack.

```sh
cargo test --locked -p styrene-nomadnet
cargo clippy --locked -p styrene-nomadnet --all-targets -- -D warnings
```

The remaining coordinator extraction is specified in
`openspec/changes/nomadnet-crate-extraction/` at the repository root. Sessions,
cache, downloads, links and cleanup still live in the daemon at this gate.
This extraction does not implement cache expiry or failed-route recovery.
