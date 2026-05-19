---
id: styrene-identity-spec
title: "Styrene Identity: Key Derivation and Signing Specification"
version: 1.0.0
status: draft
authors: [cwilson613]
date: 2026-04-17
supersedes: [styrene-identity (design), yubikey-rns-identity (research)]
---

# Styrene Identity: Key Derivation and Signing Specification

## 1. Overview

Styrene Identity provides a unified cryptographic root from which all protocol-specific keys are deterministically derived. A single 32-byte root secret, stored in hardware (YubiKey), a credential manager (Bitwarden), or an encrypted file, feeds an HKDF-SHA256 derivation hierarchy that produces keys for mesh networking (RNS, Yggdrasil, WireGuard), SSH authentication, git commit signing, age file encryption, agent delegation, and identity-bound X.509 control-plane certificates.

This document specifies the derivation hierarchy, signer tiers, SSH agent protocol, agent delegation model, and security properties of the system as implemented in the `styrene-identity` Rust crate.

### 1.1 Design Goals

1. **Single root, many keys.** One secret produces all protocol keys deterministically. Recovery of the root (via seed phrase or hardware token) recovers everything.
2. **Hardware-first, software-fallback.** YubiKey (Tier A) is the primary signer. Bitwarden (Tier C) distributes the root to devices without the token. Encrypted file (Tier D) is the universal fallback.
3. **No private keys on disk.** The SSH agent derives keys in memory, signs, and zeroizes. File export exists only as an explicit opt-in escape hatch.
4. **Agent accountability.** Agents receive their own signing keys, derived from the same root, enabling cryptographic distinction between human and agent commits.
5. **Collision resistance by construction.** Parameterized key families (SSH user keys, agent keys, TLS certificate keys) use two-level HKDF, making collisions with flat-namespace purposes structurally impossible.
6. **Scoped control-plane PKI.** Managed agents can receive deterministic server/client certificates rooted in a Styrene identity without inventing a separate secret lifecycle.

### 1.2 Scope

This specification covers:
- The HKDF derivation hierarchy (Section 2)
- Signer tier model and trait interface (Section 3)
- YubiKey FIDO2 integration (Section 4)
- SSH agent protocol (Section 5)
- Git commit signing and agent delegation (Section 6)
- Identity-bound PKI for TLS and mTLS control planes (Section 7)
- Security properties, threat model, and known limitations (Section 8)
- Recovery and rotation (Section 9)

This specification does NOT cover:
- The identity manifest and cross-protocol binding wire format (see `styrene-identity.md`)
- Web of Trust and trust propagation algorithms (see `styrene-trust-model.md`)
- RBAC enforcement at protocol endpoints (see `rbac-mesh-identity-design.md`)
- Post-quantum hybrid signing (ML-DSA-65 layering)

---

## 2. HKDF Derivation Hierarchy

### 2.1 Root Secret

The root secret is a 32-byte (256-bit) value that serves as the input keying material (IKM) for HKDF-SHA256. It MUST have at least 128 bits of entropy. Sources include:

| Source | Tier | Entropy |
|--------|------|---------|
| FIDO2 hmac-secret PRF output | A | 256 bits (hardware PRF) |
| Bitwarden secure note / SSH key item | C | 256 bits (stored) |
| `argon2id(passphrase, salt)` output | D | Passphrase-dependent |
| `OsRng::fill_bytes()` | D (generation) | 256 bits (CSPRNG) |

The `RootSecret` type enforces zeroize-on-drop semantics via the `zeroize` crate. However, the `RootSecret` is not consumed by `KeyDeriver::new()` — it persists until the calling scope ends. The root is zeroized when `RootSecret` drops, not immediately after HKDF-Extract.

The `KeyDeriver` stores the 32-byte PRK directly (not in an `Hkdf` struct) and implements `Drop` to zeroize it. This ensures no root-equivalent material survives past the deriver's lifetime.

### 2.2 Extract Phase

HKDF-Extract runs once per root secret, producing a pseudo-random key (PRK):

```
PRK = HMAC-SHA256(salt="styrene-identity-v1", IKM=root_secret)
```

The salt is a fixed, non-secret, ASCII byte string providing domain separation per RFC 5869 Section 3.1. This ensures that Styrene Identity derivations cannot collide with any other HKDF usage in the system (e.g., RNS DH-derived session keys, TLS key schedules) even if the same root secret were accidentally reused as IKM elsewhere.

The PRK is cached in the `KeyDeriver` struct for the lifetime of the derivation session. It is NOT persisted.

### 2.3 Expand Phase — Flat Purposes

Each flat purpose derives a 32-byte output key material (OKM) via HKDF-Expand:

```
OKM = HKDF-Expand(PRK, info=purpose_string, L=32)
```

| Purpose | Info String | Output Type | Usage |
|---------|------------|-------------|-------|
| `RnsEncryption` | `"styrene-rns-encryption-v1"` | X25519 private key | RNS ECDH key exchange |
| `RnsSigning` | `"styrene-rns-signing-v1"` | Ed25519 seed | RNS packet signing |
| `Yggdrasil` | `"styrene-yggdrasil-v1"` | Ed25519 seed | Yggdrasil node key |
| `WireGuard` | `"styrene-wireguard-v1"` | Curve25519 private key | WireGuard tunnel |
| `SshHost` | `"styrene-ssh-host-v1"` | Ed25519 seed | SSH server host key |
| `Age` | `"styrene-age-v1"` | X25519 private key | age file encryption |
| `GitSigning` | `"styrene-git-signing-v1"` | Ed25519 seed | User's git commit signing |

All info strings are versioned (`-v1` suffix). Version bumps produce entirely new key material, enabling non-destructive algorithm upgrades.

### 2.4 Expand Phase — Parameterized Families (Two-Level HKDF)

Parameterized key families use a two-level derivation to prevent info-string collisions:

**Level 1:** Derive a family-specific master key from the root PRK.

```
master = HKDF-Expand(PRK, info=family_master_string, L=32)
```

**Level 2:** Derive a per-label key from the master, using a fresh HKDF instance with a **family-specific salt**:

```
master_PRK = HMAC-SHA256(salt=family_salt, IKM=master)
OKM = HKDF-Expand(master_PRK, info=label_bytes, L=32)
```

The master key is zeroized immediately after the level-2 HKDF-Extract.

| Family | Master Info String | Level-2 Salt | Label Examples | Comment Format |
|--------|--------------------|-------------|----------------|----------------|
| SSH user keys | `"styrene-ssh-user-master-v1"` | `"styrene-identity-ssh-user-v1"` | `"github"`, `"work"` | `styrene-ssh-user-{label}` |
| Agent signing keys | `"styrene-agent-master-v1"` | `"styrene-identity-agent-v1"` | `"omegon-primary"`, `"omegon-cleave-0"` | `styrene-agent:{name}` |
| TLS certificate keys | `"styrene-tls-cert-master-v1"` | `"styrene-identity-tls-cert-v1"` | `"auspex-control/dev/server/0"` | X.509 Ed25519 key material |

Empty labels and agent names are rejected with `DeriveError::EmptyLabel`.

**Collision resistance:** Level-2 keys are derived from a different IKM (the family master) AND a different salt than level-1 flat keys. Additionally, each family uses its own distinct level-2 salt, so even identical labels across families (e.g., SSH user `"github"` vs agent `"github"` vs TLS certificate `"github"`) produce different keys through three independent mechanisms: different master key, different salt, different HKDF tree. This is a structural guarantee, not a probabilistic one.

### 2.5 Full Derivation Tree

```
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
  ├─ Expand(PRK, "styrene-ssh-user-master-v1")      → SSH user master
  │   └─ Extract(salt="styrene-identity-ssh-user-v1", master) then Expand(info=label)
  │       ├─ "github"                               → SSH key for GitHub
  │       ├─ "work"                                 → SSH key for work
  │       └─ ...
  │
  ├─ Expand(PRK, "styrene-agent-master-v1")         → agent master
  │   └─ Extract(salt="styrene-identity-agent-v1", master) then Expand(info=name)
  │       ├─ "omegon-primary"                       → primary agent signing key
  │       ├─ "omegon-cleave-0"                      → cleave worker 0
  │       ├─ "auspex-deploy"                        → deployed agent
  │       └─ ...
  │
  └─ Expand(PRK, "styrene-tls-cert-master-v1")      → TLS certificate master
      └─ Extract(salt="styrene-identity-tls-cert-v1", master) then Expand(info=label)
          ├─ "auspex-control/dev/ca/default/0"       → scoped CA key
          ├─ "auspex-control/dev/server/agent/0"     → server leaf key
          └─ "auspex-control/dev/client/operator/0"  → mTLS client leaf key
```

### 2.6 Key Type Constraints

| Output Type | Library | Clamping | Notes |
|-------------|---------|----------|-------|
| Ed25519 seed | `ed25519-dalek` | Internal (SHA-512 of seed) | Any 32 bytes valid as seed per RFC 8032 |
| X25519 private key | `x25519-dalek` | Internal (RFC 7748 §5) | Any 32 bytes valid; clamped during DH |
| Curve25519 private key | Same as X25519 | Same | WireGuard uses X25519 DH |

No pre-processing of HKDF output is required. All target libraries apply necessary clamping internally.

---

## 3. Signer Tier Model

### 3.1 Tier Definitions

| Tier | Name | Key Storage | Auth | Portability | Example |
|------|------|-------------|------|-------------|---------|
| A | Hardware HSM | Secure element, non-exportable | PIN + touch | Physical token | YubiKey 5 FIDO2 |
| B | Device HSM | Platform secure element | Biometric / PIN | Device-bound | iOS Secure Enclave, Android StrongBox |
| C | Credential Manager | Software vault, encrypted | Master password | Cross-device via sync | Bitwarden, 1Password, macOS Keychain |
| D | Encrypted File | Disk, argon2id + ChaCha20Poly1305 | Passphrase | File copy | `~/.config/styrene/identity.key` |

Tiers are ordered: A < B < C < D (lower number = higher assurance). Policy can enforce minimum tiers for specific operations.

### 3.2 IdentitySigner Trait

```rust
#[async_trait]
pub trait IdentitySigner: Send + Sync {
    fn tier(&self) -> SignerTier;
    fn label(&self) -> &str;
    fn is_available(&self) -> bool;
    async fn root_secret(&self) -> Result<RootSecret, SignerError>;
    async fn sign(&self, data: &[u8]) -> Result<Vec<u8>, SignerError>;
}
```

**Contracts:**
- `root_secret()` MUST return the same 32 bytes for the same signer configuration across calls and across machines. This is the portability guarantee.
- `root_secret()` MAY require user interaction (touch, biometric, passphrase prompt). Callers MUST NOT assume it is non-blocking.
- `root_secret()` exposes the raw 32-byte root to the caller for ALL tiers, including Tier A (YubiKey). The YubiKey's hmac-secret extension provides a PRF output, not on-device signing. True hardware-contained signing (key never in process memory) would require a different trait design and is not supported in the current architecture.
- `sign()` derives the Ed25519 signing key from `root_secret()` via `KeyDeriver::derive(KeyPurpose::RnsSigning)` and zeroizes the seed after signing. This method is specific to RNS mesh signing — SSH and git signing are handled by the SSH agent, which performs its own purpose-specific derivation.
- `is_available()` MUST NOT require user interaction. It is a synchronous check for hardware presence or file existence.

**Credential input:** Signers obtain secrets via injectable provider traits, not environment variables:
- `YubiKeySigner` accepts a `PinProvider` (trait with `get_pin() -> Option<String>`)
- `FileSigner` accepts a `PassphraseProvider` (trait with `get_passphrase() -> Vec<u8>`)

Implementations should source credentials from platform keychains, interactive prompts, or secure IPC. Environment variables are explicitly discouraged as they are visible to co-tenant processes.

### 3.3 Signer Resolution (SignerChain) — NOT YET IMPLEMENTED

When multiple signers are configured, the intended behavior is automatic tier-ordered fallback:

1. Tier A (YubiKey) — if hardware detected
2. Tier C (Bitwarden) — if vault session active
3. Tier D (Encrypted file) — if identity file exists
4. Error — no signer available

The first available signer is used. All signers for the same identity MUST produce the same root secret (they are different access paths to the same 32 bytes).

**Status:** `SignerChain` is not yet implemented. The caller currently selects a signer explicitly.

### 3.4 Implemented Signers

| Signer | Tier | Feature Gate | Status |
|--------|------|-------------|--------|
| `FileSigner` | D | `file-signer` (default) | Implemented |
| `YubiKeySigner` | A | `yubikey` | Implemented |
| `BitwardenSigner` | C | `bitwarden` | Planned |
| `KeychainSigner` | C | `keychain` | Planned |

---

## 4. YubiKey FIDO2 Integration (Tier A)

### 4.1 Mechanism

The YubiKey 5 series (firmware 5.2.3+) supports the FIDO2 `hmac-secret` extension (CTAP2 extension ID: `hmac-secret`). This extension provides a hardware-backed PRF: given a credential and a 32-byte salt, the YubiKey returns a deterministic 32-byte HMAC-SHA256 output computed from an internal secret that never leaves the secure element.

### 4.2 Salt

The StyreneID salt is:

```
STYRENE_IDENTITY_SALT = SHA-256("styrene-identity-root-v1")
```

This salt is:
- **Distinct** from the RNS-specific Python salts (`SHA-256("styrene-encryption-v1")`, `SHA-256("styrene-signing-v1")`) used in `styrened`'s legacy direct-derivation path
- **Fixed and public** — not a secret; it serves as a domain separator
- **32 bytes** — matching the hmac-secret extension's salt size requirement

### 4.3 Credential Setup (One-Time)

```
1. Open FIDO2 device via HID transport
2. make_credential(
     rp_id = "styrene.mesh",
     user_id = b"styrene-operator",
     key_type = Ed25519 (alg -8),
     extensions = [hmac-secret: true],
     authenticator_selection = {resident_key: required, user_verification: required}
   )
3. Store credential_id (base64) in configuration
```

### 4.4 Root Secret Derivation (Every Session)

```
1. Open FIDO2 device
2. get_assertion(
     rp_id = "styrene.mesh",
     challenge = [0; 32],  // dummy, not used by hmac-secret
     credential_id = stored_credential_id,
     extensions = [hmac-secret: STYRENE_IDENTITY_SALT],
     user_verification = required | discouraged (per config)
   )
3. Extract hmac-secret output from assertion extensions → 32-byte root_secret
4. Feed into KeyDeriver::new(root_secret) → HKDF hierarchy
```

### 4.5 Security Properties

- The YubiKey's internal PRF seed is non-extractable. No API exists to read it.
- The 32-byte PRF output is computationally indistinguishable from random to any party without the YubiKey.
- Same YubiKey + same credential + same salt = same root secret on any machine (portable).
- PIN protects against unauthorized use of stolen token (8 attempts before lockout).
- **The root secret exists in process memory after extraction.** This is Tier A for the extraction step only — the YubiKey provides a PRF, not an on-device signing service. All derived keys (SSH, git, age, etc.) are computed in software from the extracted root. A process memory dump during derivation reveals the root and all derivable keys.
- The root is NOT cached across calls in the current implementation. Each `root_secret()` call contacts the YubiKey, which may require a physical touch. This provides strong freshness but has UX implications (see Section 8.6).

### 4.6 PIN and Passphrase Input

The `YubiKeySigner` obtains the PIN via an injected `PinProvider` trait. The `FileSigner` obtains the passphrase via an injected `PassphraseProvider` trait. This decouples credential acquisition from the signer logic and avoids environment variable exposure.

Implementations should source credentials from:
- Platform keychains (macOS Keychain, GNOME Keyring)
- Interactive TTY prompts
- Secure IPC (Unix domain sockets)

A `NoPinProvider` (always returns `None`) is provided for touch-only YubiKey configurations.

### 4.7 Double-PRF Composition

The system composes two PRFs: `HKDF-Extract(salt, HMAC-SHA256(device_secret, app_salt))`. This is provably secure under standard assumptions — the output of the inner PRF is computationally indistinguishable from uniform random, which is valid IKM for the outer HKDF. This pattern is standard (TLS 1.3, Signal Protocol).

---

## 5. SSH Agent

### 5.1 Architecture

The `StyreneAgent` implements the SSH agent protocol (RFC draft-miller-ssh-agent) via the `ssh-agent-lib` crate's `Session` trait. It listens on a Unix domain socket (`SSH_AUTH_SOCK`) and serves Ed25519 public keys derived from the HKDF hierarchy.

```
SSH client (git, ssh, scp)
  │
  └─ SSH_AUTH_SOCK ──→ StyreneAgent
       │
       ├─ request_identities()
       │    → derive root_secret from signer
       │    → KeyDeriver::new(root_secret)
       │    → derive public keys for all configured families
       │    → return Vec<Identity>
       │
       └─ sign(pubkey, data)
            → derive root_secret from signer
            → KeyDeriver::new(root_secret)
            → match pubkey to key family + label
            → derive Ed25519 seed for matched entry
            → sign data with ed25519-dalek
            → zeroize seed
            → return Signature
```

### 5.2 Key Families

The agent serves four key families, all from the same root:

| Family | Derivation Method | Comment Format | Use Case |
|--------|-------------------|----------------|----------|
| SSH user keys | `derive_ssh_user_key(label)` | `styrene-ssh-user-{label}` | SSH auth to hosts |
| Git signing key | `derive(GitSigning)` | `styrene-git-signing` | User's commit signing |
| Agent signing keys | `derive_agent_key(name)` | `styrene-agent:{name}` | Agent commit signing |
| SSH host key | `derive(SshHost)` | `styrene-ssh-host` | SSH server identity |

### 5.3 Configuration

```rust
let agent = StyreneAgent::new(signer, &["github", "work"])
    .with_git_signing()
    .with_agent_keys(&["omegon-primary", "omegon-cleave-0"])
    .with_host_key();
```

### 5.4 No Disk Export

Private keys are NEVER written to disk by the SSH agent. All derivation happens in memory per signing request. The `DerivedEntry` struct zeroizes its seed on drop.

For systems that require key files (CI pipelines, legacy automation), an explicit `--export-insecure` CLI flag (not part of this crate — downstream CLI responsibility) can export derived keys in OpenSSH format with a warning that hardware-backed security is lost.

---

## 6. Git Commit Signing and Agent Delegation

### 6.1 Git SSH Signing

Git 2.34+ supports `gpg.format = ssh`, enabling commit signing with Ed25519 SSH keys. The Styrene SSH agent serves these keys natively:

```bash
git config --global gpg.format ssh
git config --global user.signingkey "key::ssh-ed25519 AAAA..."
git config --global commit.gpgsign true
```

The signing key reference can use the `key::` prefix with the literal public key, or point to a file containing the public key. Since the agent serves the corresponding private key via `SSH_AUTH_SOCK`, no key file is needed.

### 6.2 User vs Agent Commits

| Committer | Signing Key | Key Derivation | Git Comment |
|-----------|------------|----------------|-------------|
| Human (you) | `GitSigning` | `derive(KeyPurpose::GitSigning)` | `styrene-git-signing` |
| Primary agent | `Agent("omegon-primary")` | `derive_agent_key("omegon-primary")` | `styrene-agent:omegon-primary` |
| Cleave worker 0 | `Agent("omegon-cleave-0")` | `derive_agent_key("omegon-cleave-0")` | `styrene-agent:omegon-cleave-0` |
| Deployed agent | `Agent("auspex-deploy")` | `derive_agent_key("auspex-deploy")` | `styrene-agent:auspex-deploy` |

All keys are Ed25519, all derived from the same root, all can be registered on GitHub as signing keys. GitHub displays "Verified" for commits signed by any registered key.

### 6.3 Distinguishing Commits

```bash
# In git log, the signature identifies the signer:
git log --show-signature
  Good "git" signature for styrene-git-signing with ED25519 key SHA256:...
  Good "git" signature for styrene-agent:omegon-primary with ED25519 key SHA256:...
```

The comment field in the SSH agent identity (`styrene-git-signing` vs `styrene-agent:{name}`) propagates through `git log --show-signature`, providing human-readable attribution.

### 6.4 Per-Worktree Configuration

Agents use worktree-specific git config to select their signing key:

```bash
# In the agent's worktree:
git config user.signingkey "key::ssh-ed25519 AAAA..."  # agent's public key
```

This is set programmatically by the agent runtime (omegon) at worktree creation, using the public key bytes from `KeyDeriver::derive_agent_key(name)`.

### 6.5 Agent Key Lifecycle

Agent keys are deterministic: the same root + the same agent name = the same key, always. There is no key generation, no key distribution, no key rotation independent of root rotation.

- **Adding an agent:** Configure the name in the SSH agent. The key is immediately derivable.
- **Removing an agent:** Remove the name from the SSH agent config. Revoke the public key from GitHub.
- **Compromised agent:** The agent's key is derived from the root, but the root is not compromised. Revoke the specific agent key on GitHub. Other agents and the user's personal key are unaffected.
- **Root rotation:** ALL keys change. All GitHub signing keys must be updated. See Section 9.

### 6.6 Future: Delegation Attestation

A future extension may add cryptographic delegation proofs:

```json
{
  "delegator": "styrene-git-signing public key",
  "delegatee": "styrene-agent:omegon-primary public key",
  "scope": "repo:styrene-lab/*",
  "expires": "2026-05-01T00:00:00Z",
  "signature": "Ed25519 signature by delegator"
}
```

This is NOT implemented. The current model relies on GitHub's signing key registration (all keys on the same account) and git log inspection for attribution.

---

## 7. Identity-Bound PKI

### 7.1 Purpose

The `pki` feature derives deterministic Ed25519 X.509 certificate material from
the same Styrene root used for SSH, git signing, mesh identity, and agent keys.
It exists to give Auspex and Omegon a shared control-plane TLS/mTLS foundation:

- Every managed agent can receive a scoped server certificate for HTTPS/WSS.
- Operators and control-plane callers can receive scoped client certificates for mTLS.
- Trust anchors can be regenerated from the root when appropriate, but should be distributed through normal deployment secret paths.

The PKI layer is not a general public CA and not an anonymity mechanism. It is a
private transport identity system for Styrene-managed control planes.

### 7.2 Certificate Roles

| Role | Derivation label shape | URI SAN shape | Extended Key Usage |
|------|------------------------|---------------|--------------------|
| CA | `{scope}/ca/{profile}/{ca_epoch}` | `spiffe://styrene.dev/identity/{hash}/ca/{scope}` | CA only |
| Server | `{scope}/server/{agent_label}/{leaf_epoch}` | `spiffe://styrene.dev/identity/{hash}/agent/{agent_label}` | ServerAuth |
| Client | `{scope}/client/{client_label}/{leaf_epoch}` | `spiffe://styrene.dev/identity/{hash}/client/{client_label}` | ClientAuth |

Server certificates may also include DNS and IP subject alternative names. These
names are canonicalized, sorted, and deduplicated before issuance so the same
semantic request produces the same certificate bytes.

### 7.3 Rotation Model

Rotation is explicit and label-driven:

| Rotation knob | Changes | Does not change |
|---------------|---------|-----------------|
| `leaf_epoch` | Leaf private key, certificate, fingerprint | Scoped CA |
| `ca_epoch` | Scoped CA key/cert and every issued leaf chain | Styrene identity hash |
| `profile` | Derivation labels and validity profile namespace | Root secret |
| Root secret | All certificates and all other derived keys | Nothing |

Auspex should use leaf epochs for routine certificate replacement and CA epochs
only when the trust anchor itself must rotate. Secret grants should carry the
issued material to the target environment; the target does not need access to the
root secret.

Certificate derivation labels MUST use the v2 length-prefixed form:

```
styrene/tls/v2/{kind}/{component_len_hex}:{component}/...
```

The length is a fixed-width 16-character hexadecimal byte count. Implementations
MUST NOT derive certificate keys by slash-joining raw profile, scope, label, or
epoch fields; raw joining is ambiguous when components themselves contain `/`.

### 7.4 Auspex/Omegon Deployment Contract

Auspex consumes this layer as a producer of deployment-ready TLS material:

1. Resolve the operator root via the active Styrene signer tier.
2. Derive a scoped CA for the control domain, such as `auspex-control/prod`.
3. Derive each Omegon instance's server chain from its managed agent label.
4. Derive client chains for Auspex callers that need mTLS access.
5. Deliver the PEM bundle via a deployment-specific secret grant:
   Kubernetes Secret, Vault Secrets Operator, SSH/shuttle bootstrap, or another
   secret broker.

Omegon should receive only the cert chain, private key, and CA bundle needed for
its listener. It should not receive the Styrene root unless it is itself acting
as an identity authority for a delegated trust domain.

### 7.5 Implemented API Surface

The crate exposes:

| API | Output |
|-----|--------|
| `derive_ca_certificate` | Scoped CA certificate and private key |
| `derive_server_certificate_chain` | Server leaf plus CA bundle |
| `derive_client_certificate_chain` | Client leaf plus CA bundle |
| `StyreneCertificateProfile` | Profile name, CA epoch, leaf epoch, validity years |
| `styrene_ca_uri`, `styrene_agent_uri`, `styrene_client_uri` | Deterministic URI SAN helpers |

Private key material is stored in zeroizing wrappers and redacted from `Debug`.
Callers still must treat returned PEM/DER bytes as secrets. Internally, key
material also passes through `rcgen::KeyPair`; this type is scoped tightly, but
does not expose an explicit zeroization contract.

---

## 8. Security Properties and Threat Model

### 7.1 What the System Guarantees

1. **Determinism.** The same root secret always produces the same keys for the same purposes. This enables seed-phrase recovery and cross-device consistency.
2. **Domain separation.** The HKDF salt ensures Styrene Identity derivations are independent of all other HKDF usages, even with identical IKM.
3. **Family isolation.** Two-level HKDF ensures no label in one family (SSH user) can produce a key that collides with any key in another family (agents) or any flat purpose.
4. **Zeroization.** All intermediate key material (`RootSecret`, derived seeds, `DerivedEntry`) is zeroized on drop via the `zeroize` crate.
5. **Hardware binding (Tier A).** The YubiKey's internal PRF seed never leaves the secure element. The root secret exists in process memory only during derivation.

### 7.2 What the System Does NOT Guarantee

1. **Process memory confidentiality.** A privileged attacker with access to process memory (root on the machine, debugger attached) can read the root secret during derivation. This is inherent to any software that processes secrets.
2. **Root compromise blast radius.** Compromising the root secret (on any machine, via any tier) reveals ALL derived keys for ALL protocols. This is the fundamental trade-off of deterministic key hierarchies. See Section 8.4.
3. **Forward secrecy.** Derived keys are static (not ephemeral). Compromise of a derived key does not compromise the root, but compromise of the root compromises all derived keys retroactively.
4. **Post-quantum security.** Ed25519 and X25519 are not quantum-resistant. The ML-DSA-65 hybrid layer (specified in `styrene-identity.md`) is designed but not implemented in this crate.

### 7.3 Threat Model

| Threat | Mitigation | Residual Risk |
|--------|-----------|---------------|
| Stolen YubiKey | PIN required; attacker needs PIN + token | PIN brute-force (8 attempts before lockout) |
| Memory dump on active machine | Secrets zeroized after use; short window | Attacker with continuous memory access wins |
| Compromised Bitwarden vault | Vault encrypted with master password + 2FA | If vault decrypted, root exposed |
| Stolen encrypted identity file | argon2id makes brute-force expensive | Weak passphrase = eventual compromise |
| Malicious agent (omegon) | Agent key ≠ user key; revokable independently | Agent can sign commits as itself |
| Root secret leaked | All keys compromised; rotation required | No forward secrecy; past signatures valid |
| Info string collision | Two-level HKDF; structural prevention | None (collision is impossible by construction) |

### 7.4 Root Compromise Blast Radius

If the root secret is compromised, the attacker obtains:
- All RNS encryption and signing keys → impersonate on mesh
- All SSH keys → access to servers in `authorized_keys`
- The git signing key → forge signed commits
- The age key → decrypt files encrypted to the identity
- All agent keys → forge agent-signed commits
- The WireGuard key → decrypt VPN traffic
- The Yggdrasil key → impersonate on overlay

**Mitigation:** This is a documented, accepted trade-off of deterministic key hierarchies (same model as BIP-32 HD wallets). Users MUST:
1. Store the root secret (or seed phrase) with the same care as a master password
2. Use the highest available signer tier (YubiKey > Bitwarden > file)
3. Understand that root rotation invalidates ALL derived public keys everywhere

### 7.5 SSH Agent Re-Derivation Cost

The SSH agent re-derives ALL keys on every `request_identities()` and `sign()` call. For `YubiKeySigner`, each call requires a physical touch. For `FileSigner`, each call re-runs argon2id (intentionally slow). There is no PRK caching.

This is a deliberate security decision (no cached secrets in memory between requests) with UX consequences. A future enhancement may add an optional time-bounded PRK cache with explicit security trade-off documentation, similar to `ssh-agent`'s `AddKeysToAgent` timeout.

### 7.6 Ed25519 SigningKey Zeroization

The `ed25519-dalek` `SigningKey` type does not implement `Zeroize`. After constructing `SigningKey::from_bytes(seed)`, the expanded key in memory is not zeroizable through the public API. Mitigation: the `SigningKey` is stack-allocated, scoped to the signing function, and dropped at function exit. The 32-byte seed IS zeroized explicitly.

---

## 9. Recovery and Rotation

### 8.1 Recovery via Seed Phrase

The root secret SHOULD be backed up as a BIP-39 24-word seed phrase at identity creation. The seed phrase deterministically recovers the root secret, and therefore all derived keys.

Recovery procedure:
1. Enter seed phrase → derive root secret
2. `KeyDeriver::new(root_secret)` → all keys immediately available
3. Re-register SSH public keys, git signing keys, etc.

### 8.2 Recovery via YubiKey

If the root was derived from a YubiKey (Tier A), the same YubiKey + same credential + same salt = same root on any new machine. No seed phrase needed unless the YubiKey is lost.

### 8.3 Root Rotation

Root rotation (intentional re-keying) changes ALL derived keys. Procedure:

1. Generate new root secret (or new YubiKey credential)
2. Derive new public keys for all purposes
3. Update `authorized_keys` on all SSH servers
4. Update GitHub signing keys
5. Re-encrypt age files (old key can still decrypt; new key for new files)
6. Publish new RNS identity manifest with migration assertion
7. Update WireGuard peer configurations

Root rotation is a heavyweight operation. It is NOT required for routine key management (adding/removing SSH labels, adding/removing agents). It is required only when the root itself is compromised or when an intentional identity change is desired.

### 8.4 Partial Revocation

Individual keys can be revoked without root rotation:
- Remove a specific SSH public key from `authorized_keys`
- Remove a specific agent signing key from GitHub
- Revoke a specific WireGuard peer

The revoked key remains derivable from the root (deterministic), but is no longer trusted by the relying party. This is analogous to revoking a certificate without rotating the CA.

---

## 10. Implementation Reference

### 10.1 Crate Structure

```
styrene-identity/
  src/
    lib.rs              # Module declarations, re-exports
    derive.rs           # HKDF hierarchy, KeyDeriver, KeyPurpose
    signer.rs           # IdentitySigner trait, SignerTier, RootSecret
    pubkey.rs           # Ed25519/X25519 public key helpers [feature: signing]
    pki.rs              # X.509 CA/client/server issuance [feature: pki]
    file_signer.rs      # Tier D: encrypted file signer [feature: file-signer]
    yubikey_signer.rs   # Tier A: YubiKey FIDO2 signer [feature: yubikey]
    ssh_agent.rs        # SSH agent protocol [feature: ssh-agent]
```

### 10.2 Feature Gates

| Feature | Dependencies | Default | Enables |
|---------|-------------|---------|---------|
| `file-signer` | argon2, chacha20poly1305, signing | Yes | `FileSigner` |
| `signing` | ed25519-dalek, x25519-dalek | Via file-signer | `pubkey` module |
| `pki` | signing, rcgen | No | Identity-bound X.509 certificate issuance |
| `yubikey` | ctap-hid-fido2, base64, signing | No | `YubiKeySigner` |
| `ssh-agent` | ssh-agent-lib, ssh-key, signing, tokio | No | `StyreneAgent` |

### 10.3 Test Coverage

51 tests across all features (yubikey + ssh-agent). Key properties tested:
- Derivation determinism (same input → same output)
- Purpose distinctness (no two purposes produce the same key)
- Family isolation (SSH user keys, agent keys, flat purposes all distinct)
- Salt domain separation (salted ≠ unsalted output)
- Ed25519 sign/verify roundtrip
- X25519 public key determinism
- SSH agent identity listing and signing
- Git signing and agent key signing via SSH agent
- X.509 deterministic issuance, SAN/EKU validation, rotation boundaries, redaction
- FileSigner encrypt/decrypt roundtrip
- YubiKey salt independence from RNS salts

---

## 11. Relationship to Other Styrene Components

| Component | Relationship |
|-----------|-------------|
| **nex** | Uses `styrene-identity` as credential root for workstation bootstrap. Profile secrets encrypted to age key. SSH keys served via agent. |
| **omegon** | Agents receive per-agent signing keys via the SSH agent. Control-plane listeners can receive identity-bound TLS material. Cleave workers get distinct keys per worker. |
| **auspex** | Monitors and deploys agent instances, derives scoped control-plane TLS/mTLS material, and brokers the resulting secrets to deployment targets. |
| **codex** | Documents signed or encrypted with user's identity keys. |
| **styrened** | RNS/LXMF mesh identity derived from same root. Legacy Python path uses direct hmac-secret derivation; Rust path uses HKDF hierarchy. |

---

## Appendix A: Info String Registry

All HKDF info strings used in the system, for collision auditing:

| Info String | Level | Family |
|-------------|-------|--------|
| `styrene-rns-encryption-v1` | 1 (flat) | Protocol |
| `styrene-rns-signing-v1` | 1 (flat) | Protocol |
| `styrene-yggdrasil-v1` | 1 (flat) | Protocol |
| `styrene-wireguard-v1` | 1 (flat) | Protocol |
| `styrene-ssh-host-v1` | 1 (flat) | Protocol |
| `styrene-age-v1` | 1 (flat) | Protocol |
| `styrene-git-signing-v1` | 1 (flat) | Signing |
| `styrene-ssh-user-master-v1` | 1 (master) | SSH |
| `styrene-agent-master-v1` | 1 (master) | Signing |
| `styrene-tls-cert-master-v1` | 1 (master) | PKI |
| *(label bytes)* | 2 (under SSH user master, salt=`styrene-identity-ssh-user-v1`) | SSH |
| *(agent name bytes)* | 2 (under agent master, salt=`styrene-identity-agent-v1`) | Signing |
| *(certificate label bytes)* | 2 (under TLS certificate master, salt=`styrene-identity-tls-cert-v1`) | PKI |

Level-2 info strings are in a separate HKDF tree (different IKM AND different salt) from level-1 strings, and each family uses its own salt. No collision is possible between levels or between families.

## Appendix B: Wire Format Constants

| Constant | Value | Usage |
|----------|-------|-------|
| HKDF salt | `b"styrene-identity-v1"` (20 bytes) | HKDF-Extract salt |
| YubiKey FIDO2 salt | `SHA-256(b"styrene-identity-root-v1")` (32 bytes) | hmac-secret extension |
| YubiKey RP ID | `"styrene.mesh"` | FIDO2 relying party |
| YubiKey user ID | `b"styrene-operator"` | FIDO2 user entity |
| File format (v1) | `STID[version:1][salt:32][nonce:12][ciphertext:32+16]` (97 bytes) | Tier D identity file |
| File format (legacy) | `[salt:32][nonce:12][ciphertext:32+16]` (92 bytes) | Pre-v1 identity file (read-only compat) |
| argon2id params | `m=65536 (64 MiB), t=3, p=1, Argon2id v0x13` | Key derivation from passphrase |
| Level-2 salt (SSH user) | `b"styrene-identity-ssh-user-v1"` | Two-level HKDF for SSH user keys |
| Level-2 salt (agent) | `b"styrene-identity-agent-v1"` | Two-level HKDF for agent keys |
| Level-2 salt (TLS certificate) | `b"styrene-identity-tls-cert-v1"` | Two-level HKDF for X.509 certificate keys |

**File format versioning (v1):**
The v1 format prepends `STID` magic bytes and a version byte (0x01) before the payload.
Invalid magic → immediate rejection (not a Styrene identity file).
Unknown version → clear error with the version number.
Legacy 92-byte files (no header) are still accepted for backward compatibility.

## Appendix C: Test Vectors

Reference vectors for verifying implementation correctness. All values are hex-encoded.

**Input:**
```
root_secret = 4242424242424242424242424242424242424242424242424242424242424242
HKDF salt   = "styrene-identity-v1" (ASCII bytes)
```

**HKDF-Extract:**
```
PRK = 44a3d380c110adb5e79640934b94ece0374f9dfea5ffafd1c702188aa3ea43a2
```

**Flat purposes (HKDF-Expand):**
```
RnsEncryption = aefdbd63fb6746c2edb73bba3bcb34f61909077f65fe033c9372b55f6ace0c0c
RnsSigning    = cca4a2bbaff94149aa3a9e4f5690a5d0a996da327e73b5e7bd59027706be736d
Yggdrasil     = 0709ca1cd9a0e813c16612be3b9b3daeb60dca6053320a35332825d246f6f4ab
WireGuard     = 3d16bf14bd383b8f485b9662fa91e0857586698d4eff76e39a2781aba548d7c4
SshHost       = 9d87179ac9ae1eb624bdc8c973a97c601c79d429c508c1b88059e728c6a7fa1c
Age           = 4ddcaf4dde4bbd9e1cd755ecbdd1ffbb08659551df7860dc00645ff128fcba81
GitSigning    = 6eb3d3ef12a2447f6de281d6f896eba20ad0b0add3bc6fce80499f36b7343842
```

**SSH user keys (two-level HKDF, salt=`"styrene-identity-ssh-user-v1"`, label as info):**
```
github = 3c261af80e084a637fd20e0f7274a4106702894f0d23c47e855f6c9adce20d75
work   = (derive with same method, label="work")
```

**Agent signing keys (two-level HKDF, salt=`"styrene-identity-agent-v1"`, agent name as info):**
```
omegon-primary  = 4dd66edcda091a5e3d15aa3fb8ec32d81e212d94760b61915b1d6f204b0672e2
omegon-cleave-0 = (derive with same method, name="omegon-cleave-0")
```

**TLS certificate keys (two-level HKDF, salt=`"styrene-identity-tls-cert-v1"`, certificate label as info):**
```
auspex/control = bdbce0671a517c65205339d22d04adecc45b588396d8b4762ddedb71cd390ec6
```

These vectors can be reproduced with any HKDF-SHA256 implementation following the derivation rules in Section 2.

## Appendix D: Open Issues

Issues identified during adversarial review that should be resolved before production deployment:

| # | Issue | Severity | Status |
|---|-------|----------|--------|
| 1 | PRK not zeroized | High | **Resolved** — `KeyDeriver` stores raw PRK bytes with `Zeroize` on `Drop` |
| 2 | Argon2 params unspecified | Medium | **Resolved** — pinned to `m=65536, t=3, p=1, Argon2id v0x13` |
| 3 | File format has no magic bytes or version marker | Medium | **Resolved** — `STID` magic + version byte header; legacy 92-byte files still accepted |
| 4 | `SignerChain` (automatic tier fallback) not yet implemented | Medium | **Resolved** — `SignerChain` struct with tier-ordered fallback, status reporting |
| 5 | Empty SSH user labels and agent names silently accepted | Low | **Resolved** — `DeriveError::EmptyLabel` returned |
| 6 | Level-2 HKDF reuses same salt as level-1 | Low | **Resolved** — distinct salts per family (`styrene-identity-agent-v1`, `styrene-identity-ssh-user-v1`) |
| 7 | `DerivedKeys` struct contains 4 of 7 flat purposes; `derive_all()` name is misleading | Low | **Resolved** — `DerivedKeys` now contains all 7 flat purposes |
| 8 | No identity binding mechanism | Medium | Deferred to identity manifest spec |
| 9 | Forward compatibility: no mechanism for v1/v2 key coexistence during transition | Medium | Deferred |
| 10 | PIN/passphrase via env vars | Medium | **Resolved** — `PinProvider` and `PassphraseProvider` traits injected |
| 11 | SSH agent stored private seeds in key map | Medium | **Resolved** — `KeySpec` enum stores only derivation path; seed derived on-demand for signing |
| 12 | File written with race window before permission set | Low | **Resolved** — atomic `OpenOptions` with `mode(0o600)` on Unix |

## Appendix E: Bitwarden Integration (Planned)

The `BitwardenSigner` (Tier C) retrieves the root secret from a Bitwarden vault via the `bw` CLI:

```
1. Check for active Bitwarden session ($BW_SESSION)
2. bw get item <item_id> --session $BW_SESSION
3. Parse the secure note or custom field containing the 32-byte root (hex or base64)
4. Return as RootSecret
```

The Bitwarden vault is unlocked by the user's master password + 2FA (which may itself be the YubiKey). This creates a trust chain: YubiKey → unlocks Bitwarden → provides root secret → feeds HKDF.

On mobile, the Bitwarden app handles vault unlock (including YubiKey NFC tap on iOS/Android), and the root secret is retrieved via the app's credential provider extension.

The root secret stored in Bitwarden MUST be the same 32 bytes as derived by the `YubiKeySigner`. This is achieved by: (1) deriving the root via YubiKey on a trusted machine, (2) storing it in Bitwarden, (3) retrieving it from Bitwarden on other devices. Both access paths produce the same root, enabling the same HKDF tree.
