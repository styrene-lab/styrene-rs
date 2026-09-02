# Operator Profile Lifecycle Tasks

## 1. Profile roots and ownership
<!-- specs: operator-profile-lifecycle -->

- [x] Port failing Quick and Local root, path derivation, private-permission, path-escape, and no-global-fallback tests from the old candidate onto current `main`
- [x] Implement versioned manifests, coherent durable paths, host-private runtime paths, and explicit managed-daemon composition
- [x] Add failing lease tests for active owner, stale owner, process mismatch, and idempotent release before implementing exclusive ownership

## 2. Promotion and snapshots
<!-- specs: operator-profile-lifecycle -->

- [x] Add failing promotion tests for identity continuity, complete bounded state, destination collision, and pre-commit cleanup
- [x] Implement stopped-profile Quick-to-Local promotion through a validated staged sibling and atomic commit
- [x] Add failing stopped and running snapshot tests for hashes, immutable generations, live-owner coordination, and restore-as-new-generation
- [x] Implement coherent snapshots with authoritative SQLite online backup for running profiles

## 3. Identity custody
<!-- specs: operator-profile-lifecycle -->

- [x] Define and test the daemon RNS identity custody boundary without conflating SDK metadata or unrelated signer roots
- [x] Add failing recovery enrollment, fingerprint match, fingerprint mismatch, unavailable hardware, and abandonment tests
- [x] Implement encrypted recovery slots and explicit fail-closed hardware-abandonment outcomes

## 4. Portable operation
<!-- specs: operator-profile-lifecycle -->

- [x] Add failing tests for encryption, filesystem capability, stable selector, mount change, safe removal, and surprise removal
- [x] Implement encrypted Portable roots, exclusive writer leases, stable selectors, quiesce, checkpoint, synchronization, and key clearing
- [ ] Add signed macOS and Linux launcher packaging checks without autorun or persisted device paths

## 5. IPC and frontend migration
<!-- specs: operator-profile-lifecycle -->

- [x] Add failing typed IPC contract tests for inventory, create, promote, snapshot, restore, import, export, adoption, progress, and restart-required outcomes
- [x] Implement the backend operations and authorization before changing frontend mode selection
- [ ] Migrate desktop and TUI onto Quick, Local, Portable, and Connected profiles with cross-frontend parity tests
- [ ] Remove duplicate Live and Embedded lifecycle logic only after failure and ownership semantics pass

## 6. Verification
<!-- specs: operator-profile-lifecycle -->

- [ ] Run focused profile, daemon, IPC, TUI, and desktop tests plus formatting and warning-denied Clippy
- [ ] Validate OpenSpec, clean packaging, unsupported-filesystem disclosures, and residual custody or host-leakage limits
