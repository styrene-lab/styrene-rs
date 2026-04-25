# Security Model — styrene-rbac

## Authorization Architecture

- **Cumulative role hierarchy**: BLOCKED (0) < NONE (1) < PEER (10) < MONITOR (20) < OPERATOR (30) < ADMIN (40)
- **Deny-first evaluation**: blocked list checked before roster, before default role
- **Orthogonal grants**: `vpn.handshake` and `relay.reject` sit outside the hierarchy and require explicit per-identity assignment, even for ADMIN
- **Defense-in-depth on grants**: invalid capability strings are filtered at construction time (`with_grants()`), at insertion time (`add_entry()`), *and* at evaluation time (`has_capability()` checks `is_valid_capability()`)

## Input Validation

- **Identity hashes**: Must be exactly 32 lowercase hex characters (128 bits, matching RNS address hash format). Rejected by `add_entry()` if invalid.
- **Blocked prefixes**: Must be at least 8 hex characters (4 bytes). Shorter prefixes would block unacceptably large swaths of the identity space (a 2-char prefix blocks 1/256 of all identities).
- **Grants**: Filtered against `ALL_CAPABILITIES` allowlist. Unknown capability strings are silently dropped.

## Deserialization Safety

When loading `RbacPolicy` from config files via serde, callers **must** call `policy.normalize()` after deserialization. The serde path populates fields directly without validation. `normalize()` enforces:
- Identity hash format validation (32 hex chars)
- Case normalization (lowercase)
- Blocked prefix minimum length
- Grant filtering against known capabilities
- Removal of invalid entries

Without `normalize()`, a malicious or misconfigured config file can:
- Insert short blocked prefixes (mass DoS)
- Insert non-normalized identity hashes (silent policy bypass)
- Insert arbitrary grant strings (mitigated by evaluation-time checks)

## Accepted Risks

### A1. Linear-scan policy evaluation

`resolve_role()`, `has_capability()`, and `allow_list()` perform O(n) scans of the roster and blocked list. For mesh networks with small rosters (dozens to hundreds of entries), this is negligible. If roster sizes grow to thousands, consider migrating to `HashMap` for roster lookups and a trie for blocked prefix matching.

### A2. Default role is Peer (fail-open)

`RbacPolicy::default()` assigns `Role::Peer` to unknown identities, granting basic mesh capabilities (chat, relay, aether query/report). This matches the open-mesh philosophy of Reticulum. Restrictive deployments should explicitly set `default_role: none` in config.

### A3. 128-bit identity space

RNS identity hashes are truncated SHA-256 (16 bytes). The birthday bound for collisions is 2^64, which is far above any realistic mesh size. This is a protocol-level choice in Reticulum, validated as sufficient for the operational domain.

### A4. Capability strings are not namespaced

Capabilities follow a `domain.action` convention (e.g., `chat.send`, `aether.delegate`) but there is no structural enforcement of the namespace. The `ALL_CAPABILITIES` allowlist serves as the canonical registry. New capabilities must be added to both the constants and the allowlist.
