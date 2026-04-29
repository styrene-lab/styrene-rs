# Identity Linkability Model

## The fundamental constraint

All keys derived from a single StyreneIdentity root secret are
**deterministically linked**. This is by design — it's what makes the
system useful for attribution, recovery, and key management. But it
means every key in the HKDF tree is cryptographically traceable to the
same person.

This has two consequences:

1. **Root compromise reveals all identities.** Anyone who obtains the
   root secret can derive every key you've ever used — SSH, git signing,
   RNS mesh addresses, WireGuard tunnels, age encryption, and all agent
   delegation keys. This is the same trade-off as BIP-32 HD wallets.

2. **Correlation + root compromise proves linkage.** If an adversary
   suspects two RNS addresses belong to the same person (via traffic
   analysis, timing, or side channels), and later compromises the root,
   they get cryptographic proof. The HKDF derivation is deterministic —
   there is no deniability.

## What the derivation tree provides

- **Convenience separation.** Different keys for different protocols,
  all recoverable from one backup.
- **Agent accountability.** Omegon's commit signing key is provably
  delegated by your identity.
- **Cross-device consistency.** Same root on any machine produces the
  same keys.

## What the derivation tree does NOT provide

- **Unlinkability.** All derived keys trace to the same root.
- **Deniability.** You cannot plausibly deny that two derived keys
  belong to you if the root is known.
- **Compartmentalization.** Compromise of one derived key does not
  compromise others (the keys are independent), but compromise of the
  root compromises everything.

## When you need unlinkability

Use cases that require cryptographic unlinkability — where two
identities must not be provably the same person — **must use
independent root secrets**:

| Use case | Approach |
|----------|----------|
| Anonymous RNS address | `RootSecret::ephemeral()` — random, never persisted |
| Pseudonymous long-lived identity | Separate `identity.key` file (`--path`) |
| Burner identity for one-time use | `RootSecret::ephemeral()` — dropped after use |
| Whistleblower communication | Separate device, separate identity, separate network |
| Your normal work identity | Primary StyreneIdentity — attribution is the goal |

## Using ephemeral roots

```rust
use styrene_identity::signer::RootSecret;
use styrene_identity::derive::{KeyDeriver, KeyPurpose};

// Generate a root that shares no relationship with any persistent identity
let ephemeral = RootSecret::ephemeral();
let deriver = KeyDeriver::new(ephemeral.as_bytes());

// Derive an anonymous RNS signing key
let rns_seed = deriver.derive(KeyPurpose::Signing);
// ... use it ...

// When ephemeral drops, the root is zeroized. No trace remains.
drop(ephemeral);
```

## Using separate persistent identities

```bash
# Primary identity (attributed to you)
nex identity init

# Pseudonymous identity (separate root, unlinkable)
nex identity init --path ~/.config/styrene/pseudonym.key
```

These two identities share no cryptographic relationship. Compromising
one does not reveal the other. They can coexist on the same machine.

## Anti-patterns

**DO NOT** do these if you need unlinkability:

- Derive "anonymous" keys from your primary root using different
  labels or purposes — they are all linked.
- Use `derive_ssh_user_key("anonymous")` thinking the label provides
  anonymity — it doesn't. The key is still derived from your root.
- Use the same StyreneIdentity on Tor and clearnet, thinking the
  transport provides separation — the identity is the same regardless
  of transport.

## Summary

| Property | Same root | Different roots |
|----------|-----------|----------------|
| Keys recoverable from one backup | Yes | No |
| Keys attributable to one person | Yes | No |
| Keys linkable if root compromised | Yes | No |
| Agent delegation provable | Yes | No |
| Cross-device consistency | Yes | No (unless you copy the file) |
| Unlinkable under traffic analysis | **No** | Yes |
| Deniable under root compromise | **No** | Yes |
