# Cleave Reunification Report

## Root Intent
Fix 6 issues from adversarial assessment of PQC tunnel implementation.

## Children Executed

### Child 0: Crypto Protocol Fix (C1 + C2)
**Files modified:** `crypto/kdf.rs`, `crypto/aead.rs`, `crypto/mod.rs`

**C1 fix (nonce reuse):**
- Data nonces: `[0x00; 4] || sequence.to_be_bytes()` — covers 0..2^64
- Confirm nonces: `[0xFF; 4] || [0x00; 7] || role` — role 0x01/0x02
- Nonce spaces are disjoint by first-4-byte prefix. Proven by test `confirm_nonce_never_collides_with_data_nonce`.

**C2 fix (confirmation reflection):**
- KDF now expands to 96 bytes: `session_key(32) || initiator_confirm_tag(32) || responder_confirm_tag(32)`
- Tags are cryptographically distinct (different HKDF output positions). Proven by test `confirm_tags_are_role_bound`.
- `encrypt_confirm(tag, role)` / `decrypt_confirm(data, role)` — role byte changes the nonce, role mismatch fails decryption. Proven by `confirm_wrong_role_fails_decrypt`.

### Child 1: Session Hardening (C3 + W1 + W2 + W3)
**Files modified:** `session/mod.rs`

**C3 fix (dead code):** Removed the orphaned `EphemeralSecret` generation. `initiate()` now generates a single `StaticSecret` from random bytes.

**W3 fix (zeroize):** Added `secret_bytes.zeroize()` after `diffie_hellman()` in `process_initiate()`.

**W1 fix (sliding window):** Replaced strict monotonic counter with 64-bit sliding window:
- `replay_window_top: u64` — highest authenticated sequence
- `replay_window: u64` — bitfield of seen sequences within window
- `check_replay()` — stateless check before decryption
- `mark_seen()` — called only after GCM authentication passes
- Out-of-order within 64 packets: accepted. Proven by `replay_window_accepts_out_of_order`.
- Duplicate: rejected. Proven by `replay_window_rejects_duplicate_within_window`.
- Too old: rejected. Proven by `replay_window_rejects_too_old`.

**W2 fix (authenticated close):**
- `close()` returns `CloseAction::Authenticated(PqcDataPayload)` when established, `CloseAction::Unauthenticated(PqcClosePayload)` otherwise.
- Authenticated close embeds `CLOSE_MAGIC || reason || msg_len || msg` inside an encrypted data frame.
- `process_close()` rejects unauthenticated close for established sessions.
- `try_authenticated_close()` decodes the close sentinel from decrypted data.
- State guard: `close()` rejects `Closed` state. Proven by `close_rejects_already_closed`.

### Integration
Wired new role-bound KDF/AEAD into session handshake methods:
- Responder encrypts `responder_confirm_tag` with `CONFIRM_ROLE_RESPONDER`
- Initiator verifies responder's tag, then encrypts `initiator_confirm_tag` with `CONFIRM_ROLE_INITIATOR`
- Responder verifies initiator's tag
- PqcConfirm.encrypted_confirm differs from PqcRespond.encrypted_confirm. Proven by `confirmation_is_not_reflectable`.
- Reflected confirm rejected. Proven by `reflected_confirm_is_rejected`.

## Conflict Detection
**Artifact overlap:** Both children modified `session/mod.rs`. Resolved by executing sequentially — Child 0's crypto API changes were integrated into Child 1's session rewrite.

**No contradictions, no interface mismatches, no assumption violations.**

## Validation
- `cargo test --workspace`: 192 passed, 0 failed, 2 ignored (pre-existing)
- `cargo clippy -p styrene-tunnel -- -D warnings`: clean
- `cargo fmt -- --check`: clean

## Test Delta
- Before: 16 tests in styrene-tunnel (5 session + 6 AEAD + 3 KEM + 2 KDF)
- After: 31 tests in styrene-tunnel (15 session + 11 AEAD + 3 KEM + 3 KDF)
- Net: +15 tests, all targeting the specific vulnerabilities
