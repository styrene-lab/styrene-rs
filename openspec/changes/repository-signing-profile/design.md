# Repository Signing Identity Profile Design

## Ownership

`styrene-identity` owns principals, key-family separation, binding attribution, canonical
binding bytes, and cryptographic verification. It does not own repository governance.

`styrene-git` consumes verified `IdentityId` and repository binding values. It owns
repository identifiers, delegate policy, transition history, current-epoch selection,
publisher namespaces, and signed repository state.

This direction prevents a dependency cycle. `styrene-identity` must not depend on any
`styrene-git` crate, including through test support.

## Identity ID

Profile v1 retains the existing identity algorithm:

```text
IdentityId = SHA-256(identity_ed25519_public_key)[0..16]
```

The Rust type stores `[u8; 16]`. Canonical text is lowercase hexadecimal with no prefix.
Canonical CBOR embeds the ID as a 16-byte byte string. Parsers reject uppercase text rather
than treating presentation normalization as canonical parsing.

## Repository Key Derivation

The software derivation path uses a parameterized family:

```text
root_prk = HKDF-Extract("styrene-identity-v1", root_secret)
master = HKDF-Expand(root_prk, "styrene-repository-signing-master-v1", 32)
epoch_prk = HKDF-Extract("styrene-identity-repository-signing-v1", master)
seed = HKDF-Expand(
  epoch_prk,
  "styrene-repository-signing-epoch-v1\0" || u32be(epoch),
  32,
)
```

The result is an Ed25519 seed. Epoch zero is valid. `u32::MAX` is valid. APIs accept an
epoch value and never infer the next epoch by arithmetic.

The binding format also accepts a repository public key produced by an external signer.
Deterministic derivation is not a custody requirement.

Ordinary Git commit signing continues to use the established Identity signing behavior.
The deprecated `GitSigning` alias is not repository authority.

## Binding Profile V1

The registered signing domain is:

```text
styrene-repository-signer-binding-v1
```

The signing frame is the accepted common profile frame:

```text
u16be(domain_length) || domain_ascii ||
u16be(record_profile_version) ||
u32be(canonical_cbor_length) || canonical_protected_cbor
```

The protected claims are a canonical CBOR map:

| Key | Field | Type | Rule |
| --- | --- | --- | --- |
| `0` | profile version | unsigned integer | Exactly `1`. |
| `1` | Identity ID | byte string | Exactly 16 bytes. |
| `2` | Identity public key | byte string | Exactly 32 bytes and hashes to key `1`. |
| `3` | repository public key | byte string | Exactly 32 valid Ed25519 bytes. |
| `4` | key epoch | unsigned integer | Range `0..=u32::MAX`. |
| `5` | purpose | text string | Exactly `styrene-repository-signing-v1`. |
| `6` | signature suite | unsigned integer | `0` means Ed25519. |

No field is nullable. Unknown and duplicate keys are invalid. Keys occur in increasing
numeric order. Protected claims are limited to 256 encoded bytes.

The outer binding is a canonical CBOR map:

| Key | Field | Type | Rule |
| --- | --- | --- | --- |
| `0` | protected claims | byte string | Exact canonical protected bytes. |
| `1` | signature | byte string | Exactly 64 Ed25519 bytes. |

The outer binding is limited to 384 encoded bytes. The established Identity Ed25519 key
signs the protected signing frame. The binding digest is:

```text
SHA-256("styrene-repository-signer-binding-id-v1\0" || canonical_outer_cbor)
```

The binding carries no repository identifier. One valid binding can authorize operations
in multiple repositories when each repository independently permits the Identity.

## Verification Order

Verification follows the common canonical profile without shortcuts:

1. Enforce the outer 384-byte limit.
2. Decode one outer map and reject trailing bytes.
3. Enforce the closed outer schema and canonical CBOR forms.
4. Enforce the protected 256-byte limit.
5. Decode and validate the protected closed schema.
6. Derive `IdentityId` from the Identity key and compare it in constant time.
7. Re-encode protected and outer values and compare both byte-for-byte.
8. Construct the registered frame.
9. Verify the Ed25519 signature strictly.
10. Return a verified binding value with no claim about consumer authorization.

Errors are typed into bounded format, canonical, semantic, identity mismatch, and signature
classes. Error text is not a wire contract.

## Epoch Policy Boundary

Identity verification proves assignment at an epoch. It does not make an epoch current.
Historical verification can therefore retain old bindings.

Repository state selects its accepted binding under prior-state policy. A repository
operation names an epoch and must be checked against the selected binding. Higher epochs do
not automatically authorize repository state merely because they are cryptographically
valid.

Revocation, identity-root replacement, and authoritative lifecycle heads remain outside
profile v1. A later lifecycle profile can restrict which historically valid bindings are
currently acceptable without changing their original signatures.

## Corpus Layout

The released corpus lives under a versioned, read-only path such as:

```text
crates/libs/styrene-identity/tests/vectors/repository-signing-v1/
  manifest.toml
  positive.json
  negative.json
```

The manifest records the profile version, generation revision, generator command, file
digests, public test-root warning, and supported verifier implementations.

An explicit maintainer command generates candidate vectors outside ordinary tests. A
separate check compares candidate output with committed files. No normal test writes into
the source tree.

Positive vectors include epochs zero, one, and `u32::MAX`, plus an externally supplied
repository key. Negative vectors cover every field mutation, forbidden CBOR form, arity,
length, type, canonical ordering, and size boundary required by the common profile.

## Features And Test Support

The `repository-signing` feature enables only public key, Identity ID, canonical binding,
derivation, framing, and verification code. It uses no operational signer backend.

A `test-support` feature can provide deterministic fixture actors only if the surface stays
small. It must not expose repository documents, delegate policy, refs, objects, transports,
or mutable global state. If fixture support grows, it moves to a separate test-support crate.

## Cross-Workspace Consumption

Committed `styrene-git` manifests use a released `styrene-identity` version or immutable Git
revision. They do not use a sibling path dependency.

Every Git pull request verifies copied immutable vectors independently. A scheduled or
manually dispatched external gate checks out exact revisions of both repositories and runs:

- latest Identity release against Git main.
- previous supported Identity release against Git main.
- Identity main against Git main.

The gate records both revisions and failing property-test seeds. It does not make either
repository an ordinary workspace member of the other.

## Migration

Before Git consumes profile v1, its private `StyreneIdentity` and `SignerBinding`
implementations are temporary spike code. Consumption removes those implementations and
adapts repository errors to the Identity profile's typed failures.

Repository IDs remain stable because profile v1 retains the same 16-byte Identity ID
algorithm. Signer-binding bytes can change because no released repository state depends on
the spike format. If that assumption becomes false before migration, persisted bindings
must receive an explicit legacy reader rather than silent reinterpretation.
