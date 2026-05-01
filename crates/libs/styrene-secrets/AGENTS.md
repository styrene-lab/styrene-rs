# styrene-secrets

Application secret resolution for Styrene extensions. Checks project-local store, user-global store, and environment variables in priority order. Published to crates.io as v0.1.1.

## Module Map

| File | Purpose |
|------|---------|
| `src/lib.rs` | `resolve(key)`, `resolve_with_source()`, `resolve_or_env()` -- cascading resolution logic. Project store walks up from cwd (stops at HOME, rejects symlinks). Env var fallback warns on stderr. |
| `src/store.rs` | `SecretStore` -- encrypted SQLite + ChaCha20Poly1305 per-value encryption. argon2id key derivation. Keychain integration for zero-interaction passphrase. WAL mode + busy_timeout for concurrency. |
| `src/error.rs` | `ResolveError` (NotFound with actionable message, Store), `StoreError` (Db, Crypto, BadPassphrase, Io) |
| `src/value.rs` | `SecretValue` type alias (`SecretBox<Vec<u8>>`), `secret_from_str()`, `secret_from_bytes()`, `SecretValueExt::expose_str()` |
| `src/testing.rs` | `MockStore` -- in-memory HashMap for extension tests, no encryption needed |

## Key Types

- **`resolve(key) -> Result<SecretValue, ResolveError>`** -- primary API, checks all sources
- **`resolve_or_env(key, env_var)`** -- adds a fallback conventional env var (e.g. `GITHUB_TOKEN`)
- **`resolve_with_source(key) -> Result<ResolvedSecret, ResolveError>`** -- also returns `SecretSource`
- **`SecretValue`** -- `SecretBox<Vec<u8>>`, zeroized on drop, redacted Debug
- **`SecretSource`** -- enum: `ProjectStore(PathBuf)`, `UserStore`, `EnvVar(String)`, `FallbackEnvVar(String)`
- **`SecretStore`** -- encrypted SQLite store: `open()`, `get()`, `set()`, `list()`, `delete()`
- **`MockStore`** -- test double: `new(&[("key", "value")])`, `get()`, `list()`, `contains()`
- **`ResolveError::NotFound`** -- includes actionable message with env var name and CLI command

## Feature Flags

| Feature | Default | Enables |
|---------|---------|---------|
| (none) | yes | Env var resolution only + `MockStore` |
| `file-store` | no | `SecretStore` (rusqlite, argon2, chacha20poly1305, rand_core) |
| `keychain` | no | `file-store` + OS keychain passphrase management (keyring crate) |

## Test Commands

```bash
# Default features (env var only + MockStore)
cargo test -p styrene-secrets

# With encrypted store
cargo test -p styrene-secrets --features file-store

# Full stack including keychain (may prompt on macOS)
cargo test -p styrene-secrets --features keychain
```

## Resolution Order

1. Project-local `.styrene/secrets.db` (walk up from cwd, stop at HOME)
2. User-global `~/.styrene/secrets.db`
3. `STYRENE_SECRET_{KEY}` env var (warns on stderr)
4. `ResolveError::NotFound` with actionable instructions

`resolve_or_env()` adds step 3.5: fallback conventional env var (e.g. `GITHUB_TOKEN`).

## Gotchas

- **Env var key mapping**: `forge.github.token` becomes `STYRENE_SECRET_FORGE_GITHUB_TOKEN`. Dots and dashes become underscores, uppercased.
- **Stderr warnings**: Env var resolution emits a yellow warning nudging toward the encrypted store. Suppress with `STYRENE_SECRETS_QUIET=1`.
- **Broken store falls through**: If a project or user store fails to open (wrong passphrase, corruption), it warns on stderr and continues to the next source rather than hard-erroring. This prevents a locked project store from blocking env var fallback.
- **Symlink rejection**: `find_project_store()` rejects symlinked `.styrene/` directories to prevent traversal attacks.
- **Keychain migration safety**: If a store exists on disk but the keychain has no passphrase, `open_with_keychain()` refuses to generate a new passphrase (would create a second store). Requires explicit `keychain-migrate`.
- **Key names stored in plaintext**: The SQLite store encrypts values but key names, timestamps, nonces, and salts are visible. Use full-disk encryption if key name confidentiality matters.
- **Argon2id params match styrene-identity**: m=64MiB, t=3, p=1. Same hardened params.
- **rusqlite version range**: Accepts both 0.31 and 0.32 (`>=0.31, <0.33`) for downstream compatibility.
- **thiserror version range**: Accepts both 1.x and 2.x (`>=1, <3`) for downstream compatibility.
- **File permissions**: Store and parent directory are set to 0o600/0o700 on Unix.
- **Store path is `~/.styrene/secrets.db`**: Not `~/.config/styrene/` -- different from the identity file location.

## Current Status

- Published to crates.io as v0.1.1
- Core resolution (env var, file store, keychain) is complete and tested
- `MockStore` available for extension testing without encryption
- No CLI binary yet -- `styrene-secrets set` referenced in error messages is planned
