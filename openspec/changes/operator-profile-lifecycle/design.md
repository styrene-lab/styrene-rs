# Operator Profile Lifecycle Design

## Reassessment

The unmerged `origin/operator-profile-lifecycle` branch contains a partial
implementation at `9810575c82c9f04423381e2433c23878d7977eb4`. It is based on an
old runtime and has no pull request. It is not an ancestor of current `main` and
is not an accepted implementation.

That branch is test and design input only. Current runtime architecture is
authoritative. The implementation must begin by porting failing behavior tests,
then apply the smallest compatible production changes on current `main`.

## Profile Model

| Profile | Durable storage | Daemon ownership |
| --- | --- | --- |
| Quick | Temporary managed root | Frontend-owned |
| Local | Persistent managed root | Frontend-owned |
| Portable | Encrypted removable root | Frontend-owned |
| Connected | External daemon authority | Externally owned |

Fixture remains a deterministic test backend and is not an operator profile.
Live is an observed runtime condition, not a profile type.

## Path And Ownership Authority

`styrened` owns profile manifests, path validation, transactions, snapshots,
promotion, and identity continuity. Host-private sockets and ownership state do
not reside on removable media. Managed daemon composition receives explicit
configuration, identity, database, page, file, node, and socket paths. It never
falls back to global Styrene paths.

One exclusive writer lease protects each managed profile. Stale, mismatched, or
unverifiable ownership fails closed and remains observable.

## Transactions

Promotion and restore stage output beside the destination, validate identity and
known components, synchronize durable state, then publish with one atomic rename.
The source remains unchanged. Existing destinations are rejected.

A stopped profile can snapshot directly from coherent files. A running profile
must use authoritative SQLite online backup and coordinated component checkpoints.
Ordinary copying of a live database and WAL is forbidden.

## Custody And Portable Media

The daemon RNS identity is distinct from SDK identity metadata and unrelated
signer roots. Hardware abandonment can claim continuity only through a
pre-enrolled recovery path that verifies the same public fingerprint.

Portable profiles resolve through a selected profile marker and stable hardware
selector, not a persisted mount or device path. Safe removal requires quiesce,
checkpoint, synchronization, lease release, and key clearing. Surprise removal
stops durable writes and does not redirect them to host defaults.

## Frontend Boundary

Typed IPC owns inventory, creation, promotion, snapshot, restore, import, export,
adoption, progress, and restart-required outcomes. Frontends own selection and
confirmation only. Migration removes duplicate Live and Embedded lifecycle logic
after parity tests pass.
