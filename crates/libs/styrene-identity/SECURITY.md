# Security Model — styrene-identity

## Cryptographic Architecture

- **HKDF-SHA256** with fixed domain-separation salt (`styrene-identity-v1`)
- **Two-level derivation** for parameterized families (agent keys, SSH user keys) with distinct level-2 salts (`styrene-identity-agent-v1`, `styrene-identity-ssh-user-v1`)
- **Pinned test vectors** ensure derivation stability across versions
- **Ed25519** signing via `ed25519-dalek` (pinned to `=2.1.1`)
- **Argon2id** with hardened parameters (m=64MiB, t=3, p=1) for file-based encryption
- **Versioned file format** with `STID` magic bytes and version marker (backward-compatible with legacy headerless files)

## Key Material Lifecycle

- `RootSecret`: Zeroize-on-drop, Debug-redacted
- `KeyDeriver`: PRK stored as `[u8; 32]` with explicit `Drop` zeroization. No non-zeroizable `Hkdf` struct is persisted.
- `DerivedKeys`: Zeroize-on-drop via `#[zeroize(drop)]`
- File signer: Decrypted plaintext `Vec` is explicitly zeroized after copy. Passphrase wrapped in `Zeroizing` for panic safety.
- SSH agent: Private seeds derived on-demand per `sign()` call, zeroized immediately after signing. Public key map holds no private material.

## Secret Input Handling

Passphrase and PIN are provided via trait-based providers (`PassphraseProvider`, `PinProvider`), never via environment variables. Environment variables are rejected because:
- Visible to co-tenant processes via `/proc/<pid>/environ`
- Inherited by child processes
- May be logged in shell history

## File Permissions

Identity files are written with `mode(0o600)` set atomically at creation time via `OpenOptions` on Unix. No TOCTOU race between creation and permission setting.

## Accepted Risks

### A1. `Hkdf::from_prk()` intermediates not zeroized

`KeyDeriver::expander()` reconstructs an `Hkdf` from stored PRK bytes on each derivation call. The `hkdf` crate's `Hkdf` struct does not implement `Zeroize`. These transient stack-allocated values are dropped at end of scope but not explicitly wiped.

**Rationale**: The PRK bytes themselves (which are root-equivalent) *are* zeroized on `KeyDeriver::drop()`. The transient `Hkdf` wrapper exists only for the duration of a single `expand()` call. The risk is stack residue, which requires memory forensics to exploit.

### A2. 128-bit RNS identity space

RNS identity hashes are truncated SHA-256 (16 bytes / 128 bits). This provides:
- 2^128 preimage resistance (infeasible for targeted attacks)
- 2^64 birthday bound (sufficient for mesh networks far below 2^32 nodes)

This is a protocol-level design decision in Reticulum, not a styrene-identity choice.

### A3. No replay protection in `sign()` trait

The `IdentitySigner::sign()` method signs arbitrary data with no built-in nonce, timestamp, or sequence number. Replay protection is the responsibility of the protocol layer (e.g., RNS link establishment, LXMF message IDs, aether request correlation).

### A4. SSH agent double `root_secret()` on sign

The SSH agent calls `root_secret()` twice per `sign()` request — once to build the public key map, once to derive the matching seed. For hardware signers (YubiKey), this requires two physical interactions. This is a deliberate trade-off: the alternative (caching all seeds) would hold all private key material in memory simultaneously, increasing the blast radius of a memory disclosure.

### A5. Non-Unix file permissions

On non-Unix platforms, the identity file is written without platform-specific ACL restrictions. The file-signer is effectively Unix-only in production deployments.

### A6. `HOME` fallback

`FileSigner::default_path()` falls back to `"."` if `HOME` is unset. Callers (styrened, CLI tools) should always provide an explicit path rather than relying on the default.
