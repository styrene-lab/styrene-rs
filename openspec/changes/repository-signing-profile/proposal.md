# Repository signing identity profile

## Intent

Make Styrene Identity the sole authority for canonical identity identifiers,
repository-signing key derivation, and repository signer bindings. Publish an immutable
positive and negative conformance corpus before `styrene-git` consumes the profile.

## Scope

Included:

- A fixed-width canonical `IdentityId` derived from the established Identity public key.
- An epoch-indexed repository-signing key family distinct from identity, Git commit,
  transport, SSH, agent, and certificate keys.
- A versioned canonical-CBOR binding signed with the established Identity authority.
- Strict decode, validate, re-encode, compare, frame, and signature verification.
- Immutable derivation, binding, framing, signature, digest, and rejection vectors.
- A minimal production feature and deterministic test-support surface.

Excluded:

- Repository identifiers, delegates, thresholds, refs, objects, and governance history.
- Selection of the current accepted binding or epoch for a repository operation.
- Identity-root replacement, binding revocation, custody policy, and lifecycle evaluation.
- Git commit signing, transport authentication, vault policy, and hardware signer policy.
- Daemon, RNS, IPC, carrier, route, and network types.

## Success criteria

- Repository-signing keys are bytewise distinct from every existing key family for fixed
  vectors and generated root secrets.
- Independent implementations produce the same `IdentityId`, binding bytes, signing
  frame, signature, and binding digest for every positive vector.
- Every negative vector fails with its specified stable rejection class.
- Ordinary tests verify committed vectors without modifying the source tree.
- `styrene-git` can consume the released profile with default features disabled and no
  dependency on signer storage, hardware, daemon, or transport code.
