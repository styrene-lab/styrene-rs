# Operator Profile Lifecycle - Delta Spec

## ADDED Requirements

### Requirement: Managed profiles have coherent path authority

Every Quick, Local, or Portable profile derives all durable daemon paths from one
validated profile root and all transient control paths from one host-private
runtime root.

#### Scenario: Managed profile starts
Given a valid managed profile manifest and runtime root
When the backend composes its daemon
Then every configuration, identity, database, page, file, and node path derives from the profile root
And every socket and ownership path derives from the runtime root

#### Scenario: Profile path escapes
Given a profile component resolves outside its validated root
When the backend validates the profile
Then it rejects the profile before daemon startup
And it does not read or write the escaped path

### Requirement: Profile promotion is identity preserving and atomic

A stopped Quick profile can promote to a new Local profile without changing its
identity or publishing partial state.

#### Scenario: Promotion succeeds
Given a stopped Quick profile with committed identity and application state
When the operator promotes it to an unused Local destination
Then the Local profile preserves the public identity and every bounded committed component
And the destination becomes visible only after validation and synchronization succeed

#### Scenario: Promotion fails before commit
Given a stopped source profile
When promotion fails before its atomic commit
Then the source remains usable and unchanged
And no destination profile is published

### Requirement: Snapshots are coherent immutable generations

Snapshots capture one coherent profile generation. A running daemon uses an
authoritative online database backup rather than ordinary live-WAL copying.

#### Scenario: Running profile snapshot
Given a managed profile has an active daemon and SQLite WAL state
When the operator requests a snapshot
Then the backend uses an online database backup and coordinated component checkpoint
And the snapshot records component hashes and one immutable generation

#### Scenario: Snapshot restores as new generation
Given a valid immutable snapshot
When the operator restores it to an unused destination
Then the restored profile preserves the snapshot identity and committed state
And it receives a new profile generation without modifying the snapshot

### Requirement: Identity continuity is verified across custody changes

Recovery enrollment and hardware abandonment claim continuity only after the
restored daemon identity matches the enrolled public fingerprint.

#### Scenario: Enrolled recovery succeeds
Given a profile has an encrypted recovery slot and recorded public fingerprint
When the operator restores after hardware custody is unavailable
Then the backend verifies the recovered identity fingerprint
And it reports continuity only after the fingerprints match

#### Scenario: Recovery cannot prove continuity
Given no usable recovery slot can reproduce the recorded identity
When hardware custody is abandoned
Then the backend reports identity continuity unavailable
And it does not create a replacement identity under the old profile identity

### Requirement: Portable profiles have exclusive fail-closed ownership

Portable operation requires encrypted storage, an exclusive writer lease,
host-private runtime paths, stable hardware selection, and explicit safe removal.

#### Scenario: Portable profile is already owned
Given another live owner holds the portable profile lease
When a frontend requests portable startup
Then the backend rejects the second owner
And it performs no profile mutation

#### Scenario: Portable media disappears
Given a portable daemon is active
When the selected media disappears unexpectedly
Then the backend stops durable writes and reports the interruption
And it does not fall back to a host-global data path

#### Scenario: Safe removal succeeds
Given a portable daemon is active and healthy
When the operator requests safe removal
Then the backend quiesces, checkpoints, synchronizes, releases ownership, and clears keys
And it reports when media can be removed

### Requirement: Frontends use one typed profile lifecycle

The backend exposes typed inventory, creation, promotion, snapshot, restore,
import, export, adoption, progress, ownership, and restart-required outcomes.

#### Scenario: Frontends observe the same profile
Given desktop and TUI clients address the same backend profile
When each client requests profile metadata
Then both receive equivalent ownership, persistence, custody, network-policy, and cleanup fields
And neither client derives profile truth from local mode names

#### Scenario: Connected profile is selected
Given an external daemon owns the selected profile
When a frontend opens a Connected session
Then the frontend does not start an Embedded daemon fallback
And external ownership remains explicit
