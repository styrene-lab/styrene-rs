# mobile-backend-identity - Baseline

### Requirement: Production identity custody fails closed

Selecting a production custody backend must either activate that exact backend
or fail before identity creation. Plaintext storage is permitted only when the
caller explicitly selects the development-only plaintext backend.

#### Scenario: Selected secure backend is unavailable
Given the mobile configuration selects Keychain, Android Keystore, or encrypted-file custody
When the required target capability, feature, authentication, or key material is unavailable
Then boot fails with a typed custody error before creating plaintext identity material
And no fallback identity file is written

#### Scenario: Plaintext backend is selected explicitly
Given a development or test configuration explicitly selects PlaintextFile
When the mobile identity is created
Then custody status identifies development plaintext storage
And no production-secure claim is emitted

### Requirement: Public custody status is authoritative and secret-free

The backend must expose the requested and active custody backend, protection
class, availability, authentication requirement, downgrade state, and typed
failure without exposing private identity material.

#### Scenario: Secure identity is restored
Given a supported secure backend already contains the mobile identity
When the node boots and identity status is queried
Then the public identity is stable and custody status names the active protection
And the projection contains no key bytes, credentials, passphrases, or export capability

#### Scenario: Requested and active custody differ
Given backend configuration and active signer state do not agree
When custody status is constructed
Then the mismatch is reported as a typed downgrade or failure
And the backend does not label custody secure

### Requirement: Editable public identity metadata is durable

Display name, icon, and short name edits must be normalized, persisted outside
private key material, restored before announce construction, and exposed through
typed identity query.

#### Scenario: Display name changes and process restarts
Given the caller updates the display name to a valid normalized value
When the process restarts on the same identity and storage
Then identity query returns the normalized edited value
And the next announce uses that value without changing the identity hash

#### Scenario: Public metadata edit is invalid
Given an edit contains an empty normalized name, a control character, or exceeds a documented bound
When the caller submits the edit
Then the backend rejects it with a typed validation error
And persisted metadata and announce state remain unchanged
