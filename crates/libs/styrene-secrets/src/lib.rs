//! Application secret resolution for Styrene extensions.
//!
//! Provides a simple `resolve(key)` API that checks multiple sources
//! in priority order:
//!
//! 1. **Project-local store** — `.styrene/secrets.db` (walks up from cwd).
//!    Requires the `file-store` feature.
//! 2. **User-global store** — `~/.styrene/secrets.db`.
//!    Requires the `file-store` feature.
//! 3. **Environment variable** — `STYRENE_SECRET_{KEY}` (key uppercased,
//!    dots and dashes become underscores). Always available. Emits a
//!    warning on stderr when used, nudging toward the encrypted store.
//!
//! The encrypted store is unlocked via OS keychain (with `keychain` feature)
//! or the `STYRENE_SECRETS_PASSPHRASE` env var. Keychain is preferred —
//! it provides zero-interaction encrypted secrets.
//!
//! Secret values are [`secrecy::SecretBox`]-wrapped, zeroized on drop,
//! and redacted in Debug output.
//!
//! # Feature flags
//!
//! | Feature | What it enables |
//! |---|---|
//! | (default) | Env var resolution only, `MockStore` for testing |
//! | `file-store` | Encrypted SQLite store, manual passphrase |
//! | `keychain` | `file-store` + OS keychain manages the passphrase |
//!
//! # Extension usage
//!
//! ```toml
//! [features]
//! omegon-secrets = ["dep:styrene-secrets"]
//!
//! [dependencies]
//! styrene-secrets = { version = "0.1", optional = true, features = ["keychain"] }
//! ```
//!
//! # Testing
//!
//! Use [`testing::MockStore`] in extension tests:
//!
//! ```
//! use styrene_secrets::testing::MockStore;
//! use styrene_secrets::value::ExposeSecret;
//!
//! let store = MockStore::new(&[("forge.github.token", "ghp_test")]);
//! let token = store.get("forge.github.token").unwrap();
//! assert_eq!(token.expose_secret().as_slice(), b"ghp_test");
//! ```

#![forbid(unsafe_code)]

pub mod error;
#[cfg(feature = "file-store")]
pub mod store;
pub mod testing;
pub mod value;

pub use error::ResolveError;
pub use secrecy::{ExposeSecret, SecretBox, SecretString};
pub use value::{secret_from_bytes, secret_from_str, SecretValue, SecretValueExt};

#[cfg(feature = "file-store")]
pub use error::StoreError;
#[cfg(feature = "file-store")]
pub use store::SecretStore;

/// Where a resolved secret came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretSource {
    /// Project-local `.styrene/secrets.db` (includes the path).
    ProjectStore(std::path::PathBuf),
    /// User-global `~/.styrene/secrets.db`.
    UserStore,
    /// `STYRENE_SECRET_{KEY}` environment variable.
    EnvVar(String),
    /// Fallback conventional environment variable (e.g. `GITHUB_TOKEN`).
    FallbackEnvVar(String),
}

impl std::fmt::Display for SecretSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ProjectStore(p) => write!(f, "project store ({})", p.display()),
            Self::UserStore => write!(f, "user store (~/.styrene/secrets.db)"),
            Self::EnvVar(var) => write!(f, "env var {var}"),
            Self::FallbackEnvVar(var) => write!(f, "fallback env var {var}"),
        }
    }
}

/// A resolved secret with its source.
pub struct ResolvedSecret {
    /// The secret value.
    pub value: SecretValue,
    /// Where the secret was found.
    pub source: SecretSource,
}

/// Resolve a secret by key, checking sources in priority order.
///
/// Resolution order:
/// 1. Project-local store (`.styrene/secrets.db`, walking up from cwd)
/// 2. User-global store (`~/.styrene/secrets.db`)
/// 3. Environment variable `STYRENE_SECRET_{KEY}` (**warns on stderr**)
/// 4. Returns [`ResolveError::NotFound`] with actionable instructions
///
/// # Example
///
/// ```no_run
/// # use styrene_secrets::{resolve, ExposeSecret};
/// let token = resolve("forge.github.token").unwrap();
/// println!("token length: {}", token.expose_secret().len());
/// ```
pub fn resolve(key: &str) -> Result<SecretValue, ResolveError> {
    let resolved = resolve_with_source(key)?;
    Ok(resolved.value)
}

/// Like [`resolve`], but also returns where the secret was found.
///
/// Useful for diagnostics and migration tooling.
pub fn resolve_with_source(key: &str) -> Result<ResolvedSecret, ResolveError> {
    // 1. Project-local store (walk up from cwd).
    #[cfg(feature = "file-store")]
    {
        if let Some(result) = resolve_from_project_store(key)? {
            return Ok(result);
        }
    }

    // 2. User-global store.
    #[cfg(feature = "file-store")]
    {
        if let Some(result) = resolve_from_user_store(key)? {
            return Ok(result);
        }
    }

    // 3. Environment variable (warn — nudge toward store).
    let env_key = to_env_key(key);
    if let Ok(val) = std::env::var(&env_key) {
        if !is_quiet() {
            eprintln!(
                "\x1b[33mwarning:\x1b[0m secret '{}' resolved from env var {} — \
                 consider moving to the encrypted store: styrene-secrets set {}",
                key, env_key, key
            );
        }
        return Ok(ResolvedSecret {
            value: value::secret_from_str(&val),
            source: SecretSource::EnvVar(env_key),
        });
    }

    // 4. Not found.
    Err(ResolveError::NotFound {
        key: key.to_string(),
        env_key,
    })
}

/// Resolve a secret by key, with a fallback conventional env var.
///
/// Resolution order:
/// 1. Project-local store
/// 2. User-global store
/// 3. `STYRENE_SECRET_{KEY}` environment variable (warns)
/// 4. The specified `env_var` fallback (warns)
/// 5. [`ResolveError::NotFound`]
pub fn resolve_or_env(key: &str, env_var: &str) -> Result<SecretValue, ResolveError> {
    let resolved = resolve_or_env_with_source(key, env_var)?;
    Ok(resolved.value)
}

/// Like [`resolve_or_env`], but also returns the source.
pub fn resolve_or_env_with_source(
    key: &str,
    env_var: &str,
) -> Result<ResolvedSecret, ResolveError> {
    match resolve_with_source(key) {
        Ok(r) => return Ok(r),
        Err(ResolveError::NotFound { .. }) => {}
        #[cfg(feature = "file-store")]
        Err(e) => return Err(e),
    }

    // Try the fallback env var.
    if let Ok(val) = std::env::var(env_var) {
        if !is_quiet() {
            eprintln!(
                "\x1b[33mwarning:\x1b[0m secret '{}' resolved from env var {} — \
                 consider moving to the encrypted store: styrene-secrets set {}",
                key, env_var, key
            );
        }
        return Ok(ResolvedSecret {
            value: value::secret_from_str(&val),
            source: SecretSource::FallbackEnvVar(env_var.to_string()),
        });
    }

    let env_key = to_env_key(key);
    Err(ResolveError::NotFound {
        key: key.to_string(),
        env_key,
    })
}

// ---------------------------------------------------------------------------
// Store resolution helpers
// ---------------------------------------------------------------------------

/// Walk up from cwd looking for `.styrene/secrets.db`.
///
/// **Security boundaries:**
/// - Stops at HOME directory (never walks above it)
/// - Rejects symlinked `.styrene/` directories (prevents traversal attacks)
/// - Does not follow symlinks for the `secrets.db` file itself
#[cfg(feature = "file-store")]
fn find_project_store() -> Option<std::path::PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let home = std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .map(std::path::PathBuf::from)?;

    let mut dir = cwd.as_path();
    loop {
        let styrene_dir = dir.join(".styrene");

        // Reject symlinked .styrene directories.
        if let Ok(meta) = std::fs::symlink_metadata(&styrene_dir) {
            if meta.file_type().is_symlink() {
                if !is_quiet() {
                    eprintln!(
                        "\x1b[33mwarning:\x1b[0m .styrene/ at {} is a symlink — skipping",
                        dir.display()
                    );
                }
            } else {
                let candidate = styrene_dir.join("secrets.db");
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }

        // Stop at HOME — never walk above it.
        if dir == home {
            return None;
        }

        dir = dir.parent()?;
    }
}

/// Try to resolve from a project-local store.
///
/// If the project store can't be opened (wrong passphrase, corruption),
/// warns on stderr and falls through to the next source rather than
/// hard-erroring. This prevents a corrupt or locked project store from
/// blocking access to the user-global store.
#[cfg(feature = "file-store")]
fn resolve_from_project_store(key: &str) -> Result<Option<ResolvedSecret>, ResolveError> {
    let path = match find_project_store() {
        Some(p) => p,
        None => return Ok(None),
    };

    match open_store_best_effort(&path) {
        Some(Ok(s)) => match s.get(key) {
            Ok(Some(val)) => Ok(Some(ResolvedSecret {
                value: val,
                source: SecretSource::ProjectStore(path),
            })),
            Ok(None) => Ok(None),
            Err(e) => Err(ResolveError::Store {
                key: key.to_string(),
                source: e,
            }),
        },
        Some(Err(e)) => {
            // Warn and fall through — don't let a broken project store
            // block access to the user store or env vars.
            if !is_quiet() {
                eprintln!(
                    "\x1b[33mwarning:\x1b[0m project store at {} could not be opened: {} \
                     — falling back to next source",
                    path.display(),
                    e
                );
            }
            Ok(None)
        }
        None => Ok(None),
    }
}

/// Try to resolve from the user-global store.
///
/// Like `resolve_from_project_store`, warns and falls through on store
/// open errors — a locked or misconfigured user store should not prevent
/// env var fallback.
#[cfg(feature = "file-store")]
fn resolve_from_user_store(key: &str) -> Result<Option<ResolvedSecret>, ResolveError> {
    let store_path = match store::default_path() {
        Ok(p) => p,
        Err(_) => return Ok(None),
    };

    match open_store_best_effort(&store_path) {
        Some(Ok(s)) => match s.get(key) {
            Ok(Some(val)) => Ok(Some(ResolvedSecret {
                value: val,
                source: SecretSource::UserStore,
            })),
            Ok(None) => Ok(None),
            Err(e) => Err(ResolveError::Store {
                key: key.to_string(),
                source: e,
            }),
        },
        Some(Err(e)) => {
            if !is_quiet() {
                eprintln!(
                    "\x1b[33mwarning:\x1b[0m user store could not be opened: {} \
                     — falling back to environment variables",
                    e
                );
            }
            Ok(None)
        }
        None => Ok(None),
    }
}

/// Try to open a store using the best available passphrase source.
///
/// Returns `None` if no passphrase source is available (no keychain,
/// no env var). Returns `Some(Err)` if a passphrase source exists but
/// the store can't be opened (wrong passphrase, corruption, etc.).
#[cfg(feature = "file-store")]
fn open_store_best_effort(
    path: &std::path::Path,
) -> Option<Result<SecretStore, crate::error::StoreError>> {
    // Prefer keychain if available.
    #[cfg(feature = "keychain")]
    {
        return Some(SecretStore::open_with_keychain(path));
    }

    // Fall back to env var passphrase.
    #[allow(unreachable_code)]
    {
        if let Ok(passphrase) = std::env::var("STYRENE_SECRETS_PASSPHRASE") {
            return Some(SecretStore::open(path, passphrase.as_bytes()));
        }
        None
    }
}

/// Check if warnings are suppressed via `STYRENE_SECRETS_QUIET=1`.
///
/// Useful in CI, daemons, and production environments where stderr
/// warnings about env var resolution would be noise.
fn is_quiet() -> bool {
    std::env::var("STYRENE_SECRETS_QUIET")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Convert a dotted secret key to the environment variable name.
///
/// `forge.github.token` → `STYRENE_SECRET_FORGE_GITHUB_TOKEN`
fn to_env_key(key: &str) -> String {
    let mut env_key = String::with_capacity("STYRENE_SECRET_".len() + key.len());
    env_key.push_str("STYRENE_SECRET_");
    for ch in key.chars() {
        match ch {
            '.' | '-' => env_key.push('_'),
            c => env_key.push(c.to_ascii_uppercase()),
        }
    }
    env_key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_env_key_dots_and_dashes() {
        assert_eq!(
            to_env_key("forge.github.token"),
            "STYRENE_SECRET_FORGE_GITHUB_TOKEN"
        );
        assert_eq!(
            to_env_key("some-service.api-key"),
            "STYRENE_SECRET_SOME_SERVICE_API_KEY"
        );
    }

    #[test]
    fn resolve_from_env_with_warning() {
        let key = "test.resolve.env.only";
        let env_key = to_env_key(key);
        std::env::set_var(&env_key, "test-value-42");

        let resolved = resolve_with_source(key).unwrap();
        assert_eq!(resolved.value.expose_secret().as_slice(), b"test-value-42");
        assert_eq!(resolved.source, SecretSource::EnvVar(env_key.clone()));

        std::env::remove_var(&env_key);
    }

    #[test]
    fn resolve_not_found_has_actionable_message() {
        let err = resolve("test.unlikely.key.xyzzy").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("STYRENE_SECRET_"), "error should mention env var: {msg}");
        assert!(msg.contains("styrene-secrets set"), "error should mention CLI: {msg}");
    }

    #[test]
    fn resolve_or_env_fallback_with_source() {
        let key = "test.resolve.fallback.src";
        let fallback = "TEST_RESOLVE_FALLBACK_SRC_TOKEN";

        // Neither set — should fail.
        std::env::remove_var(&to_env_key(key));
        std::env::remove_var(fallback);
        assert!(resolve_or_env(key, fallback).is_err());

        // Set fallback — should succeed via fallback.
        std::env::set_var(fallback, "fallback-value");
        let resolved = resolve_or_env_with_source(key, fallback).unwrap();
        assert_eq!(resolved.value.expose_secret().as_slice(), b"fallback-value");
        assert_eq!(
            resolved.source,
            SecretSource::FallbackEnvVar(fallback.to_string())
        );

        // Set primary env var — should take precedence.
        let env_key = to_env_key(key);
        std::env::set_var(&env_key, "primary-value");
        let resolved = resolve_or_env_with_source(key, fallback).unwrap();
        assert_eq!(resolved.value.expose_secret().as_slice(), b"primary-value");
        assert_eq!(resolved.source, SecretSource::EnvVar(env_key.clone()));

        std::env::remove_var(&env_key);
        std::env::remove_var(fallback);
    }

    #[test]
    fn secret_source_display() {
        assert_eq!(
            SecretSource::UserStore.to_string(),
            "user store (~/.styrene/secrets.db)"
        );
        assert_eq!(
            SecretSource::EnvVar("STYRENE_SECRET_FOO".into()).to_string(),
            "env var STYRENE_SECRET_FOO"
        );
        assert_eq!(
            SecretSource::FallbackEnvVar("GITHUB_TOKEN".into()).to_string(),
            "fallback env var GITHUB_TOKEN"
        );
    }
}
