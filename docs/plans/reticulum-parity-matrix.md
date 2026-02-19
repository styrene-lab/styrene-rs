# Reticulum Parity Matrix

Last verified: 2026-02-16 (`cargo fmt -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace --all-features`)

Status legend: not-started | partial | done

| Python Module | Rust Module | Status | Tests | Notes |
| --- | --- | --- | --- | --- |
| `RNS/Reticulum.py` | `crates/internal/reticulum-legacy/src/lib.rs` + `crates/internal/reticulum-legacy/src/config.rs` | done | `crates/internal/reticulum-legacy/tests/config_parity.rs` | Runtime initialization and config defaults are parity-covered. |
| `RNS/Identity.py` | `crates/internal/reticulum-legacy/src/identity.rs` | done | `crates/internal/reticulum-legacy/tests/identity_parity.rs`, `crates/internal/reticulum-legacy/tests/lxmf_signature.rs` | Identity serialization/signing parity is fixture-covered. |
| `RNS/Destination.py` | `crates/internal/reticulum-legacy/src/destination/*` | done | `crates/internal/reticulum-legacy/tests/destination_parity.rs`, `crates/internal/reticulum-legacy/tests/lxmf_address_hash.rs` | Destination addressing and hash derivation parity is covered. |
| `RNS/Packet.py` | `crates/internal/reticulum-legacy/src/packet.rs` | done | `crates/internal/reticulum-legacy/tests/packet_parity.rs`, `crates/internal/reticulum-legacy/tests/lxmf_packet_limits.rs`, `crates/internal/reticulum-legacy/tests/link_proof_packet.rs` | Packet framing, limits, and proof packet behavior are covered. |
| `RNS/Transport.py` | `crates/internal/reticulum-legacy/src/transport/*` | done | `crates/internal/reticulum-legacy/tests/transport_tables.rs`, `crates/internal/reticulum-legacy/tests/transport_delivery.rs`, `crates/internal/reticulum-legacy/tests/announce_scheduler.rs` | Routing, announce scheduling, and transport table mechanics are covered. |
| `RNS/Link.py` | `crates/internal/reticulum-legacy/src/destination/link.rs` + `crates/internal/reticulum-legacy/src/transport/link_table.rs` | done | `crates/internal/reticulum-legacy/tests/link_event_layout.rs`, `crates/internal/reticulum-legacy/tests/lxmf_receipt_callbacks.rs`, `crates/internal/reticulum-legacy/tests/lxmf_receipt_proof.rs` | Link lifecycle/events and receipt flows are covered. |
| `RNS/Interfaces/*` | `crates/internal/reticulum-legacy/src/iface/*` | done | `crates/internal/reticulum-legacy/tests/iface_parity.rs`, `crates/internal/reticulum-legacy/tests/tcp_hdlc_test.rs` | Interface framing/IO parity for supported interfaces is covered. |
| `RNS/Cryptography/*` | `crates/internal/reticulum-legacy/src/crypt/*` + `crates/internal/reticulum-legacy/src/crypt.rs` | done | `crates/internal/reticulum-legacy/tests/crypto_parity.rs`, `crates/internal/reticulum-legacy/tests/lxmf_group_encrypt.rs`, `crates/internal/reticulum-legacy/tests/hash_parity.rs` | Core crypto and hash compatibility are fixture-tested. |
| `RNS/Resource.py` | `crates/internal/reticulum-legacy/src/resource.rs` | done | `crates/internal/reticulum-legacy/tests/resource_channel_parity.rs` | Resource advertisement/transfer channels are covered. |
| `RNS/Channel.py` | `crates/internal/reticulum-legacy/src/channel.rs` | done | `crates/internal/reticulum-legacy/tests/resource_channel_parity.rs` | Channel framing and interaction with resources are covered. |
| `RNS/Buffer.py` | `crates/internal/reticulum-legacy/src/buffer.rs` | done | `crates/internal/reticulum-legacy/tests/buffer_parity.rs` | Buffer management parity is covered by fixture tests. |
| `RNS/Discovery.py` | `crates/internal/reticulum-legacy/src/transport/discovery.rs` | done | `crates/internal/reticulum-legacy/tests/discovery_parity.rs`, `crates/internal/reticulum-legacy/tests/python_announce.rs` | Discovery/announce behavior is covered with Python fixtures. |
| `RNS/Resolver.py` | `crates/internal/reticulum-legacy/src/utils/resolver.rs` | done | `crates/internal/reticulum-legacy/tests/resolver_parity.rs` | Resolver parity is covered for resolution paths. |
| `RNS/Utilities/*` | `crates/internal/reticulum-legacy/src/utils/*` + `crates/internal/reticulum-legacy/src/hash.rs` | done | `crates/internal/reticulum-legacy/tests/hash_parity.rs`, `crates/internal/reticulum-legacy/tests/api_helpers.rs` | Utility/helper parity is covered for exported helpers. |
| `RNS/CRNS/*` | `crates/internal/reticulum-legacy/src/bin/*` | done | `crates/internal/reticulum-legacy/tests/cli_parity.rs` | CLI tools and help/flag parity checks are covered. |
