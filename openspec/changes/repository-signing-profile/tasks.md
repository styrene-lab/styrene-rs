# Repository Signing Identity Profile Tasks

## 1. Corpus Hygiene
<!-- specs: repository-signing-identity -->

- [x] 1.1 Add a test that fails when an ordinary `styrene-identity` test modifies a tracked vector
- [x] 1.2 Move vector generation out of `phase3_advanced` into an explicit maintainer command
- [x] 1.3 Make ordinary tests load and verify the existing derivation vectors read-only
- [x] 1.4 Record generator revision, command, public-test-secret warning, and file digests in corpus provenance

## 2. Identity ID Authority
<!-- specs: repository-signing-identity -->

- [x] 2.1 Add red tests for fixed-width bytes, lowercase text, uppercase rejection, malformed text, and existing identity-hash vectors
- [x] 2.2 Implement the authoritative `IdentityId` type and constant-time public-key derivation check
- [x] 2.3 Add generated parse, display, and derivation round-trip properties
- [x] 2.4 Replace internal untyped identity-hash use where the new domain type is the correct boundary

## 3. Repository Signing Key Family
<!-- specs: repository-signing-identity -->

- [x] 3.1 Add red vectors proving repository keys differ from every existing key family
- [x] 3.2 Implement epoch-indexed repository-signing derivation with explicit epoch input
- [x] 3.3 Add fixed vectors for epochs zero, one, and `u32::MAX`
- [x] 3.4 Add generated root and epoch determinism, separation, and no-wraparound properties
- [x] 3.5 Correct documentation that could conflate deprecated Git commit signing with repository authority

## 4. Binding Profile
<!-- specs: repository-signing-identity -->

- [x] 4.1 Add red golden tests for protected claims, signing frame, signature, outer binding, and binding digest
- [x] 4.2 Implement the closed profile-v1 schema, registered domain, frame, issue, and strict verify APIs
- [x] 4.3 Add an externally supplied repository-key positive vector independent of deterministic derivation
- [x] 4.4 Add typed errors for bounded format, canonical, semantic, identity mismatch, and signature failures
- [x] 4.5 Verify all public key and signature operations use strict Ed25519 verification

## 5. Negative Corpus
<!-- specs: repository-signing-identity -->

- [x] 5.1 Add field mutation vectors for version, Identity ID, Identity key, repository key, epoch, purpose, and suite
- [x] 5.2 Add malformed length, arity, type, truncation, oversize, and trailing-byte vectors
- [x] 5.3 Add non-shortest, indefinite, tagged, floating-point, duplicate, unknown, and reordered-key vectors
- [x] 5.4 Add boundary vectors for protected and outer byte limits
- [x] 5.5 Pin each negative vector to a stable rejection class without treating error prose as a wire contract

## 6. Features And Test Support
<!-- specs: repository-signing-identity -->

- [x] 6.1 Add a minimal `repository-signing` feature with default features disabled in a consumer fixture
- [x] 6.2 Prove the minimal feature excludes vault, file signer, YubiKey, keychain, SSH agent, PKI, daemon, RNS, and transport code
- [x] 6.3 Add bounded deterministic fixture actors behind dev-dependencies or explicit `test-support`
- [x] 6.4 Prove production builds cannot access fixture roots or private test keys

## 7. Downstream Conformance
<!-- specs: repository-signing-identity -->

- [ ] 7.1 Publish immutable positive and negative profile-v1 vectors for independent downstream verification
- [x] 7.2 Add a two-checkout conformance harness that records exact Identity and Git revisions
- [ ] 7.3 Run latest release, previous supported release, and Identity main compatibility lanes
- [x] 7.4 Document the release and compatibility policy before replacing Git's spike contract

## 8. Validation
<!-- specs: repository-signing-identity -->

- [x] 8.1 Run default, minimal repository-signing, property, vector, and negative-corpus tests
- [x] 8.2 Run formatting, warning-denied Clippy, rustdoc warnings, and dependency-policy checks
- [x] 8.3 Verify ordinary tests leave the source tree byte-for-byte unchanged
- [x] 8.4 Compare implementation and every rejection outcome with this delta specification
