# RNS Security Hardening - Delta Spec

## ADDED Requirements

### Requirement: Cached Fernet tags are verified in constant time

Cached Fernet authentication must use a constant-time MAC verification primitive and must not compare tags with data-dependent early exit.
Canonical token tests must load `tests/interop/fixtures/rns/index-v2.json` through
`styrene_interop_runner::rns_fixtures`, select the `rns-1.5.1` authority ID,
and must not copy vectors or define another canonical RNS index, schema, or fixture root.

#### Scenario: Tampered cached token is rejected
Given a valid token produced by a cached Fernet key
When any authentication-tag byte is changed and the token is verified
Then verification returns the authentication failure outcome
And no plaintext decryption is attempted

### Requirement: Private key and ratchet persistence is private and atomic

Raw key and private ratchet files must be written through exclusive unpredictable temporary files, atomically replaced without following predictable temporary symlinks, and excluded from debug output; supported platforms must apply owner-private permissions and preserve the previous complete file when replacement fails.

#### Scenario: Existing temporary symlink cannot redirect a secret write
Given an attacker-controlled symlink exists at the legacy predictable temporary path
When a key or private ratchet is persisted
Then the symlink target remains unchanged
And the complete secret is stored only at the intended destination with owner-private permissions where supported

#### Scenario: Secret replacement fails before publication
Given a complete secret file already exists
When writing or atomically replacing its new temporary sibling fails
Then the previous complete secret remains readable
And abandoned temporary secret material is removed

### Requirement: Key fallback distinguishes availability from invalid data

A fallback key manager must consult its secondary only for a missing primary read or an explicitly classified primary availability failure; primary argument, decode, integrity, and unclassified failures must surface unchanged.

#### Scenario: Corrupt primary write is not redirected
Given the primary key backend reports an integrity or decode failure
When a caller stores key material through the fallback manager
Then the original primary error is returned
And the secondary backend does not contain the key

### Requirement: Receipt correlation recovers from mutex poisoning

Receipt correlation must recover its independent-entry map after mutex poisoning, clear the accepted poison state, and continue track, lookup, resolve, and prune operations.

#### Scenario: Receipt map is used after a panic
Given a thread panicked while holding the receipt correlation mutex
When a caller tracks and resolves a receipt after the panic
Then the correlation operation succeeds using the recovered map
And subsequent direct lock acquisition no longer reports poison
