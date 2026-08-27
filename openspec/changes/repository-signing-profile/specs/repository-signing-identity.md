# Repository Signing Identity - Delta Spec

## ADDED Requirements

### Requirement: Canonical identity identifiers have one authoritative representation

Styrene Identity defines `IdentityId` as the first 16 bytes of SHA-256 over the established
Ed25519 Identity public key. Its binary representation is exactly 16 bytes. Its canonical
text representation is exactly 32 lowercase hexadecimal characters.

#### Scenario: Independent identity identifier derivation
Given two implementations receive the same valid Identity public key.
When each derives the canonical `IdentityId`.
Then both produce the same 16 bytes and lowercase hexadecimal text.

#### Scenario: Non-canonical identity identifier text
Given identifier text is uppercase, has the wrong length, or contains a non-hexadecimal character.
When a canonical `IdentityId` parser receives the text.
Then parsing fails rather than normalizing the input.

### Requirement: Repository signing uses a dedicated epoch-indexed key family

Styrene Identity derives repository-signing Ed25519 keys from a dedicated parameterized
family. The family is distinct from the Identity authority key, ordinary Git commit
signing, every transport key, SSH keys, agent keys, and certificate keys. The derivation
accepts an unsigned 32-bit epoch and does not derive by incrementing the epoch internally.

#### Scenario: Repository key family separation
Given one root secret and one repository-signing epoch.
When all supported key purposes are derived.
Then the repository-signing seed differs from every other derived seed.
And the repository-signing public key differs from every other Ed25519 public key.

#### Scenario: Repository signing epoch changes
Given one root secret and two different repository-signing epochs.
When a key is derived at each epoch.
Then the seeds and public keys are different.
And deriving either epoch again returns the same bytes.

#### Scenario: Maximum repository signing epoch
Given repository-signing epoch `u32::MAX`.
When the key is derived and represented in a binding.
Then derivation and canonical encoding succeed without wraparound.

### Requirement: Repository signer bindings use a closed canonical profile

A repository signer binding commits to its profile version, canonical Identity ID,
Identity public key, repository-signing public key, key epoch, fixed purpose, and signature
suite. The established Identity authority signs the canonical protected claims through the
registered profile signing frame. The profile accepts no caller-selected domain or purpose.

#### Scenario: Valid repository signer binding
Given an Identity authority and a repository-signing public key at one epoch.
When the authority issues a profile-v1 binding.
Then strict verification attributes the key and epoch to the canonical Identity ID.
And the binding contains no repository, transport, route, or daemon context.

#### Scenario: Binding field is substituted
Given a valid binding.
When its Identity ID, Identity key, repository key, epoch, purpose, suite, or version is changed.
Then canonical or signature verification fails.

#### Scenario: Wrong key family is bound as repository authority
Given a valid transport, SSH, agent, certificate, or ordinary Git commit-signing key.
When it is presented without a valid repository signer binding.
Then it is not accepted as repository-signing authority.

### Requirement: Binding verification is strict and bounded

A verifier enforces byte limits and decodes exactly one closed-schema value. It rejects
forbidden CBOR forms and validates all semantics. It then re-encodes, compares the bytes,
constructs the registered signing frame, and verifies the Ed25519 signature.

#### Scenario: Non-canonical binding encoding
Given a binding uses a forbidden or non-canonical CBOR form.
When strict binding verification runs.
Then verification fails before repository authority is returned.

#### Scenario: Binding exceeds its profile limit
Given protected claims or outer binding bytes exceed the profile-v1 limit.
When strict binding verification runs.
Then verification fails before unbounded allocation or signature verification.

#### Scenario: Invalid signature shape
Given a binding contains a truncated or oversized public key or signature.
When strict binding verification runs.
Then verification fails with a stable format or signature rejection class.

### Requirement: Binding epochs identify keys but do not define consumer policy

A cryptographically valid binding remains evidence that an Identity authority assigned a
repository key at the stated epoch. The binding profile does not decide which epoch is
current, revoke older bindings, or authorize repository state. Consumers must select the
binding and epoch under their own accepted state policy.

#### Scenario: Historical binding verification
Given valid bindings exist at epochs zero and one.
When each binding is verified without repository policy.
Then both bindings verify as Identity-issued evidence.
And neither result claims to be the current repository binding.

#### Scenario: Repository operation names a different epoch
Given a repository operation names epoch one and the verifier receives an epoch-zero binding.
When the consumer applies its repository policy.
Then the consumer rejects the operation because the selected binding epoch does not match.

### Requirement: Repository signing conformance vectors are immutable and read-only

Before profile v1 is released, Styrene Identity publishes positive and negative vectors.
They cover derivation, identifiers, claims, frames, signatures, bindings, digests, mutation,
canonical rejection, and boundaries. Ordinary tests never regenerate committed vectors.

#### Scenario: Ordinary corpus validation
Given the committed repository-signing corpus.
When the normal Identity test suite runs.
Then every vector is verified without modifying a tracked file.

#### Scenario: Released vector correction
Given profile-v1 vectors have been released.
When a correction would change canonical bytes or rejection behavior.
Then a new profile version and explicit compatibility rule are required.

#### Scenario: Downstream independent verification
Given a downstream repository copies released immutable vectors with provenance.
When it verifies them without invoking the Identity vector generator.
Then shared implementation errors can be detected at the package boundary.

### Requirement: Repository signing is available without operational signer backends

The production profile is available through a minimal feature. It does not enable file
signers, vaults, hardware signers, PKI, daemon, RNS, or transport code. Deterministic private
fixture material is available only through dev-dependencies or explicit test support.

#### Scenario: Minimal consumer build
Given a consumer disables default features and enables repository signing.
When the consumer builds and verifies a binding.
Then no operational signer backend or network subsystem is enabled.

#### Scenario: Production build without test support
Given a production consumer enables repository signing.
When it inspects the public production API.
Then deterministic fixture roots and private test keys are not available.
