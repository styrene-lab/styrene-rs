# Repository Signing Compatibility

This policy applies to the `styrene-repository-signing-v1` profile and its
consumers. Git commit-signing compatibility is outside this policy.

## Profile Stability

Released canonical bytes, signing frames, digests, and rejection classes are
immutable. A correction that changes any of these values requires a new profile
version and an explicit compatibility rule.

Error text is not a compatibility interface. Consumers can depend only on the
published `RepositorySignerBindingErrorClass` value for each negative vector.

## Release Requirements

A profile-bearing `styrene-identity` release must include:

- Positive and negative conformance corpora.
- Corpus provenance with the exact generator revision, commands, and SHA-256
  digests.
- Passing default, minimal-feature, corpus, Clippy, rustdoc, formatting, and
  dependency-policy gates.
- A clean immutable source revision that reproduces every committed vector.

Consumers must use a released package version or an immutable Git revision.
Committed sibling path dependencies are not supported.

## Consumer Support

`styrene-git` supports the latest and immediately previous profile-bearing
stable `styrene-identity` releases. A release that predates the
`repository-signing` feature does not count as a profile-bearing release.

The latest and previous release lanes are required before a `styrene-git`
release. The Identity-main lane is an early-warning lane. It becomes required
before the corresponding `styrene-identity` release.

Each lane records both resolved repository revisions, the Identity package
version, corpus digests, Rust version, exact commands, and property-test seed.

The previous supported release can be deprecated only after advance
documentation and a passing downstream migration. Until two profile-bearing
stable releases exist, the previous-release lane is unavailable rather than
silently substituting a release without the profile.

## Migration

Consumers must verify copied vectors independently. They must not invoke an
Identity vector generator during normal tests.

`styrene-git` can replace its spike Identity ID and signer-binding code only
after the first profile-bearing release and exact-revision conformance pass. If
spike binding bytes were persisted, migration requires an explicit legacy
reader. Consumers must not reinterpret spike bytes as profile-v1 bytes.
