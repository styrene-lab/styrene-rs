# styrene-identity

Signing trait and HKDF key derivation hierarchy for Styrene mesh nodes. One root secret derives all protocol-specific keys (RNS, Yggdrasil, WireGuard, SSH, age, git signing) via deterministic HKDF-SHA256 with domain separation.

## Module Map

| File | Purpose |
|------|---------|
| `src/lib.rs` | Re-exports, module gating by feature flag |
| `src/derive.rs` | HKDF key derivation hierarchy. `KeyDeriver` caches the PRK, derives flat-purpose keys (7 protocols) and two-level parameterized keys (SSH user, agent). Pinned test vectors in Appendix C. |
| `src/signer.rs` | `IdentitySigner` async trait (root_secret, sign), `SignerChain` (A->D fallback), `RootSecret` (zeroize-on-drop), `SignerTier` enum |
| `src/file_signer.rs` | Tier D: argon2id + ChaCha20Poly1305 encrypted file at `~/.config/styrene/identity.key`. `PassphraseProvider` trait for secure passphrase delivery. |
| `src/vault.rs` | Safe lifecycle wrapper: `init()` with O_EXCL, `backup()`, `unlock()`. Refuses overwrites. Validates agent names at config time. |
| `src/pubkey.rs` | Ed25519/X25519 public key derivation and signing from 32-byte seeds. Stack-allocated, no persistence. |
| `src/yubikey_signer.rs` | Tier A: FIDO2 hmac-secret via `ctap-hid-fido2`. One-time `setup_credential()`, then `derive_root()` per call. Key never leaves secure element. |
| `src/ssh_agent.rs` | SSH agent protocol via `ssh-agent-lib`. Serves user keys, git signing key, agent keys, host key -- all derived in memory from root secret. |

## Key Types

- **`IdentitySigner`** -- async trait: `tier()`, `label()`, `is_available()`, `root_secret()`, `sign()`
- **`SignerChain`** -- ordered vec of signers, tries each until one succeeds (A before D)
- **`SignerTier`** -- `HardwareHsm` | `DeviceHsm` | `CredentialManager` | `EncryptedFile` (ordered)
- **`RootSecret`** -- 32-byte zeroize-on-drop wrapper, redacted Debug
- **`KeyDeriver`** -- caches HKDF PRK, derives by `KeyPurpose` or parameterized (agent/SSH user)
- **`KeyPurpose`** -- enum: RnsEncryption, RnsSigning, Yggdrasil, WireGuard, SshHost, Age, GitSigning
- **`DerivedKeys`** -- all 7 flat-purpose keys, zeroize-on-drop
- **`FileSigner`** -- Tier D impl, requires `PassphraseProvider`
- **`IdentityVault`** -- lifecycle wrapper around `FileSigner`
- **`YubiKeySigner`** -- Tier A impl, requires `PinProvider`
- **`StyreneAgent`** -- SSH agent session impl (ssh-agent-lib `Session`)

## Feature Flags

| Feature | Default | Enables |
|---------|---------|---------|
| `file-signer` | yes | `FileSigner`, `IdentityVault` (argon2, chacha20poly1305, signing) |
| `signing` | no | `pubkey` module (ed25519-dalek, x25519-dalek) |
| `yubikey` | no | `YubiKeySigner` (ctap-hid-fido2, base64, signing) |
| `ssh-agent` | no | `StyreneAgent` (ssh-agent-lib, ssh-key, tokio, signing) |

## Test Commands

```bash
# Unit tests (default features -- file-signer + signing)
cargo test -p styrene-identity

# All features except yubikey (yubikey tests need hardware)
cargo test -p styrene-identity --features ssh-agent

# YubiKey tests (need physical key, run manually)
cargo test -p styrene-identity --features yubikey -- --ignored
```

## Gotchas

- **Two-level HKDF for parameterized keys**: SSH user keys and agent keys use a second HKDF-Extract with a distinct salt (`styrene-identity-ssh-user-v1` / `styrene-identity-agent-v1`). This is intentional -- it makes collisions with flat-namespace purposes structurally impossible, not just probabilistically unlikely.
- **Passphrase never from env vars**: `FileSigner` and `YubiKeySigner` require a `PassphraseProvider` / `PinProvider` trait object. Environment variables are explicitly rejected because they leak to child processes and `/proc/<pid>/environ`.
- **O_EXCL for identity creation**: `FileSigner::generate()` and `IdentityVault::init()` use `create_new(true)` (kernel-level `O_EXCL`) to atomically prevent overwrites. No TOCTOU race.
- **Legacy file format support**: `FileSigner::load()` accepts both v1 (97 bytes with STID header) and legacy v0 (92 bytes without header) for backward compatibility.
- **Identity file at `~/.config/styrene/identity.key`**: Not `~/.styrene/` -- the identity file is separate from the secrets store location.
- **ed25519-dalek pinned to =2.1.1**: Exact version pin in Cargo.toml, likely due to API stability concerns.
- **SignerChain.sign() re-derives on every call**: No caching of root secret across calls. YubiKey requires physical presence each time. FileSigner decrypts each time.
- **SSH agent re-derives public map on every sign()**: The `derive_public_map()` call happens twice per sign (once to find the key spec, once to derive the seed). This is deliberate -- no private material is cached between calls.

## Current Status

- Tier A (YubiKey) and Tier D (EncryptedFile) are implemented and tested
- Tier B (DeviceHsm -- iOS Secure Enclave, Android Keystore) is defined in the enum but not implemented
- Tier C (CredentialManager -- Bitwarden/1Password) is defined in the enum but not implemented
- Argon2id params: m=64MiB, t=3, p=1 (exceeds OWASP minimum)
