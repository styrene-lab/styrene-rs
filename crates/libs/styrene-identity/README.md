# styrene-identity

Deterministic key hierarchy for Styrene mesh nodes. One root secret derives
all protocol-specific keys — RNS, Yggdrasil, WireGuard, SSH, age, git
signing, and per-agent delegation keys — via HKDF-SHA256 with domain
separation.

Published on [crates.io](https://crates.io/crates/styrene-identity).

## Quick start

```toml
[dependencies]
styrene-identity = "0.1"
```

### Generate an identity

```rust,no_run
use styrene_identity::file_signer::{ClosurePassphraseProvider, FileSigner};
use styrene_identity::signer::IdentitySigner;

let provider = Box::new(ClosurePassphraseProvider::new(|| {
    Ok(b"my-passphrase".to_vec())
}));
let signer = FileSigner::new("~/.config/styrene/identity.key", provider);
signer.generate(b"my-passphrase").expect("generate identity");
```

### Derive keys from a root secret

```rust
use styrene_identity::derive::{KeyDeriver, KeyPurpose};

let root_secret = [0x42u8; 32]; // in practice, from a signer
let deriver = KeyDeriver::new(&root_secret);

// Flat-purpose keys
let git_seed = deriver.derive(KeyPurpose::GitSigning);   // Ed25519 seed
let age_key  = deriver.derive(KeyPurpose::Age);           // X25519 private key
let ssh_seed = deriver.derive(KeyPurpose::SshHost);       // Ed25519 seed

// Parameterized keys (two-level HKDF — structurally collision-free)
let github_ssh = deriver.derive_ssh_user_key("github").unwrap();
let agent_key  = deriver.derive_agent_key("omegon-primary").unwrap();

// All keys are deterministic: same root → same keys, always.
```

### Public key derivation

```rust
use styrene_identity::derive::{KeyDeriver, KeyPurpose};
use styrene_identity::pubkey::{ed25519_verifying_key, x25519_public_key};

let deriver = KeyDeriver::new(&[0x42u8; 32]);

let git_vk = ed25519_verifying_key(&deriver.derive(KeyPurpose::GitSigning));
let age_pk = x25519_public_key(&deriver.derive(KeyPurpose::Age));
```

### Lifecycle management with IdentityVault

```rust,no_run
use styrene_identity::vault::IdentityVault;
use styrene_identity::file_signer::ClosurePassphraseProvider;

let provider = Box::new(ClosurePassphraseProvider::new(|| {
    Ok(b"my-passphrase".to_vec())
}));
let vault = IdentityVault::with_default_path(provider);

// Create — refuses to overwrite (O_EXCL, no TOCTOU race)
vault.init(b"my-passphrase").unwrap();

// Backup before risky operations
vault.backup("/tmp/identity.key.bak").unwrap();

// Check existence
assert!(vault.exists());
```

## Derivation hierarchy

```text
root_secret (32 bytes)
  │
  HKDF-Extract(salt="styrene-identity-v1", IKM=root_secret) = PRK
  │
  ├─ Expand(PRK, "styrene-rns-encryption-v1")      → RNS X25519
  ├─ Expand(PRK, "styrene-rns-signing-v1")          → RNS Ed25519
  ├─ Expand(PRK, "styrene-yggdrasil-v1")            → Yggdrasil Ed25519
  ├─ Expand(PRK, "styrene-wireguard-v1")            → WireGuard Curve25519
  ├─ Expand(PRK, "styrene-ssh-host-v1")             → SSH host Ed25519
  ├─ Expand(PRK, "styrene-age-v1")                  → age X25519
  ├─ Expand(PRK, "styrene-git-signing-v1")          → git signing Ed25519
  │
  ├─ SSH user keys (two-level HKDF)
  │   salt="styrene-identity-ssh-user-v1"
  │   ├─ "github"  → per-host SSH Ed25519
  │   └─ "work"    → per-host SSH Ed25519
  │
  └─ Agent signing keys (two-level HKDF)
      salt="styrene-identity-agent-v1"
      ├─ "omegon-primary"   → agent commit signing Ed25519
      └─ "omegon-cleave-0"  → worker commit signing Ed25519
```

Parameterized key families use two-level HKDF with distinct salts per family.
Collisions between flat purposes, SSH user keys, and agent keys are
**structurally impossible** — they derive from different IKM, different salts,
and different HKDF trees.

## Signer tiers

The `IdentitySigner` trait abstracts over four storage tiers. All tiers
produce the same 32-byte root secret — they are different access paths
to the same identity.

| Tier | Backend | Feature | Status |
|------|---------|---------|--------|
| A | YubiKey FIDO2 hmac-secret | `yubikey` | Implemented |
| B | iOS Secure Enclave / Android StrongBox | — | Planned |
| C | Bitwarden / 1Password / macOS Keychain | — | Planned |
| D | Encrypted file (argon2id + ChaCha20Poly1305) | `file-signer` (default) | Implemented |

`SignerChain` tries signers in tier order (A→D), using the first available:

```rust,ignore
use styrene_identity::signer::SignerChain;

let chain = SignerChain::new_sorted(vec![
    Box::new(yubikey_signer),  // tried first
    Box::new(file_signer),     // fallback
]);
let root = chain.root_secret().await?;
```

## Feature flags

| Feature | Default | Enables |
|---------|---------|---------|
| `file-signer` | **yes** | `FileSigner`, `IdentityVault` (argon2, chacha20poly1305) |
| `signing` | via file-signer | `pubkey` module (ed25519-dalek, x25519-dalek) |
| `yubikey` | no | `YubiKeySigner` (FIDO2 hmac-secret) |
| `ssh-agent` | no | `StyreneAgent` SSH agent protocol |

Minimal dependency footprint — disable `default-features` and pick only
what you need:

```toml
# Just the derivation hierarchy, no file I/O or crypto
styrene-identity = { version = "0.1", default-features = false }

# Derivation + public key helpers, no file signer
styrene-identity = { version = "0.1", default-features = false, features = ["signing"] }

# Full file-based identity (default)
styrene-identity = "0.1"
```

## File format

The Tier D identity file (`~/.config/styrene/identity.key`) is 97 bytes:

```text
STID [version:1] [salt:32] [nonce:12] [ciphertext:32+16]
 4B      1B          32B       12B          48B
```

- **Encryption**: argon2id (m=64MiB, t=3, p=1) → ChaCha20Poly1305
- **Permissions**: 0o600, set atomically at creation via `O_EXCL`
- **Backward compat**: legacy 92-byte headerless files (pre-v1) are still readable

## Identity hash

The canonical identity hash is SHA-256 of the RNS signing Ed25519 public key,
truncated to 16 bytes (32 hex chars). This is the mesh identity used by
Signum, styrened, and cross-service attribution:

```rust
use styrene_identity::derive::{KeyDeriver, KeyPurpose};
use styrene_identity::pubkey::ed25519_verifying_key;
use sha2::{Digest, Sha256};

let deriver = KeyDeriver::new(&[0x42u8; 32]);
let seed = deriver.derive(KeyPurpose::RnsSigning);
let pubkey = ed25519_verifying_key(&seed);
let hash = Sha256::digest(pubkey.as_bytes());
let identity_hash = hex::encode(&hash[..16]); // 32 hex chars
```

## Git commit signing

Derived keys work with `git`'s SSH signing (`gpg.format = ssh`). Agent keys
enable cryptographic distinction between human and agent commits:

| Committer | Key | Comment in `git log --show-signature` |
|-----------|-----|---------------------------------------|
| Human | `GitSigning` | `styrene-git-signing` |
| Agent | `Agent("omegon-primary")` | `styrene-agent:omegon-primary` |

All keys trace back to the same root — one identity, multiple signers.

## Security properties

- **Zeroize-on-drop** for all secret material (`RootSecret`, `KeyDeriver` PRK, `DerivedKeys`, derived seeds)
- **No private keys on disk** — the SSH agent derives keys in memory per request
- **Domain-separated HKDF** — fixed salt prevents collision with any other HKDF usage
- **Hardened KDF** — argon2id params exceed OWASP minimums
- **Atomic file creation** — `O_EXCL` prevents overwrites with no TOCTOU race
- **Credential injection** — passphrases and PINs via traits, never environment variables

See [SECURITY.md](SECURITY.md) for the full threat model and accepted risks.

## Linkability warning

**All keys derived from one root are cryptographically linked.** This is
by design for attribution and recovery, but it means derived keys cannot
provide anonymity. For anonymous or pseudonymous identities, use an
independent root:

```rust
use styrene_identity::signer::RootSecret;

// Ephemeral: CSPRNG-generated, no file, zeroized on drop
let anon = RootSecret::ephemeral();

// Or: separate persistent identity
// nex identity init --path ~/.config/styrene/pseudonym.key
```

See [docs/unlinkability.md](docs/unlinkability.md) for the full model,
anti-patterns, and decision matrix.

## Test vectors

From a root secret of `0x42` repeated 32 times:

```text
RnsEncryption = aefdbd63fb6746c2edb73bba3bcb34f61909077f65fe033c9372b55f6ace0c0c
GitSigning    = 6eb3d3ef12a2447f6de281d6f896eba20ad0b0add3bc6fce80499f36b7343842
SSH(github)   = 3c261af80e084a637fd20e0f7274a4106702894f0d23c47e855f6c9adce20d75
Agent(omegon) = 4dd66edcda091a5e3d15aa3fb8ec32d81e212d94760b61915b1d6f204b0672e2
```

These are pinned in the test suite. Any implementation of the derivation
hierarchy must reproduce them.

## Ecosystem usage

| Crate/Binary | Dependency | Purpose |
|------|------------|---------|
| **nex** | `styrene-identity = "0.1"` | `nex identity init/show/link` — generate and manage identities |
| **aether** | path dep | Mesh node identity and RBAC |
| **auspex** | path dep (signing only) | Operator identity for monitoring agents |
| **vox** | path dep | LXMF mesh identity |
| **styrened** | workspace member | Daemon identity — RNS, SSH agent, mesh signing |

## License

MIT
