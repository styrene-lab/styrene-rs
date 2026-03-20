# Child 1: Session Hardening (C3 + W1 + W2 + W3)

## Parent Context
Fix session state machine issues: dead code, replay protection, unauthenticated close, key zeroization.

## Directive
Modify `session/mod.rs` to fix four issues:

1. **C3: Remove dead EphemeralSecret** — In `initiate()`, lines 157-159 generate an
   `EphemeralSecret` + `PublicKey` that are immediately overwritten by lines 164-171.
   Delete the dead code. Keep only the `StaticSecret` path.

2. **W3: Zeroize responder X25519 secret** — In `process_initiate()`, `secret_bytes`
   (line 312) is a local `[u8; 32]` that is never zeroized. Add `secret_bytes.zeroize()`
   after the DH is complete (after `our_secret.diffie_hellman()`). Import `Zeroize` trait.

3. **W1: Sliding window replay protection** — Replace the strict monotonic counter in
   `decrypt_data()` with a 64-packet sliding window. Implementation:
   - Add `replay_window: u64` field (bitfield, 64 bits) to `PqcSession`
   - Add `replay_window_base: u64` field (lowest sequence in window)
   - In `decrypt_data()`: if sequence < base → reject. If sequence >= base + 64 →
     slide window forward. If sequence in window → check bit, reject if set,
     mark if not. Standard IPsec anti-replay (RFC 4303 section 3.4.3).

4. **W2: Authenticated close** — Make `close()` and `process_close()` use the session
   cipher when available (state == Established). `close()` should encrypt the close
   payload as a PqcData frame with a sentinel sequence (e.g., `u64::MAX`). Add a
   `close_via_data()` method that returns a `PqcDataPayload` containing the encrypted
   close reason. `process_close()` should verify authentication when cipher is available.
   Keep the unauthenticated path for pre-established states (Initiating/Responding can
   still be closed without auth since there's no shared key yet).

## Files to Modify
- `crates/libs/styrene-tunnel/src/session/mod.rs`

## Acceptance Criteria
- No `EphemeralSecret` in `initiate()`
- `secret_bytes` zeroized in `process_initiate()`
- `decrypt_data()` accepts out-of-order packets within 64-packet window
- `decrypt_data()` rejects packets outside window or already seen
- Close from Established state is encrypted
- All existing tests updated, new tests for sliding window + authenticated close
- `cargo test -p styrene-tunnel` passes

## Outcome
(filled after execution)
