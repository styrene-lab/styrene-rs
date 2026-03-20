# Child 0: Crypto Protocol Fix (C1 + C2)

## Parent Context
Fix AES-GCM nonce reuse and confirmation reflection in PQC tunnel handshake.

## Directive
Modify `kdf.rs` and `aead.rs` so that:

1. **KDF outputs role-bound confirm tags** — expand HKDF output from 64 to 96 bytes:
   - `session_key` (32 bytes) — shared, used for data encryption
   - `initiator_confirm_tag` (32 bytes) — proves initiator has key
   - `responder_confirm_tag` (32 bytes) — proves responder has key
   - Responder encrypts `responder_confirm_tag`, initiator verifies it
   - Initiator encrypts `initiator_confirm_tag`, responder verifies it
   - Tags are different → ciphertexts are different → C2 fixed

2. **Confirm nonces are domain-separated from data nonces** — data nonces use
   `[0x00; 4] || sequence.to_be_bytes()`, confirm nonces use
   `[0xFF; 4] || [0x00; 7] || role_byte` where role=0x01 (initiator) or 0x02 (responder).
   These spaces never collide → C1 fixed.

## Files to Modify
- `crates/libs/styrene-tunnel/src/crypto/kdf.rs`
- `crates/libs/styrene-tunnel/src/crypto/aead.rs`
- `crates/libs/styrene-tunnel/src/crypto/mod.rs` (update re-exports if needed)

## Acceptance Criteria
- `HybridKeyMaterial` has `initiator_confirm_tag()` and `responder_confirm_tag()` (not `confirm_tag()`)
- `SessionCipher::encrypt_confirm()` takes a role parameter
- `SessionCipher::decrypt_confirm()` takes a role parameter
- Confirm nonce space ([0xFF prefix]) never overlaps data nonce space ([0x00 prefix])
- All existing tests updated, new tests for role differentiation
- `cargo test -p styrene-tunnel` passes

## Outcome
(filled after execution)
