// This is documentation for what the Rust-side vector generation would look like.
// For now, the golden vectors are Python-generated and Rust-consumed.
// The reverse direction (Rust-generated, Python-consumed) can be added
// by running `cargo test -p styrene-ipc-server --test wire_compat`
// with a flag to emit vectors.
//
// The key finding: msgpack map key ordering differs between Python and Rust,
// but both sides decode to identical semantic content. The wire format
// is interoperable despite ordering differences.
