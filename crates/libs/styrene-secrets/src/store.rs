//! Encrypted secret store backed by SQLite + ChaCha20Poly1305.
//!
//! Each secret value is individually encrypted with a key derived from
//! a master passphrase via argon2id. The master salt is stored in a
//! `meta` table alongside a verification tag for passphrase checking.
//!
//! Default store location: `~/.styrene/secrets.db`
//!
//! ## Security properties
//!
//! - **Values** are encrypted per-row with ChaCha20Poly1305 (random nonce).
//! - **Key names, timestamps, nonces, and salts** are stored in plaintext.
//!   An attacker with filesystem access can see *what* keys exist but not
//!   their values. Use full-disk encryption (FileVault, LUKS) if key name
//!   confidentiality is required.
//! - **Passphrase verification** uses a per-store random challenge encrypted
//!   with the derived key. The challenge is unique per store instance.
//! - **File permissions** are set to 0o600 (owner-only) on Unix.

use std::path::{Path, PathBuf};

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use rand_core::{OsRng, RngCore};
use rusqlite::{params, Connection};
use secrecy::SecretBox;
use zeroize::Zeroize;

use crate::error::StoreError;
use crate::value::SecretValue;

const SALT_LEN: usize = 32;
const NONCE_LEN: usize = 12;
/// Length of the random verification challenge (per-store unique).
const VERIFY_CHALLENGE_LEN: usize = 32;

/// Hardened Argon2id parameters — same as styrene-identity.
/// m=65536 KiB (64 MiB), t=3 iterations, p=1 parallelism.
fn argon2_instance() -> Argon2<'static> {
    let params = Params::new(65536, 3, 1, Some(32)).expect("valid argon2 params");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

/// Derive a 32-byte encryption key from a passphrase and salt.
fn derive_key(passphrase: &[u8], salt: &[u8]) -> Result<[u8; 32], StoreError> {
    let mut key = [0u8; 32];
    argon2_instance()
        .hash_password_into(passphrase, salt, &mut key)
        .map_err(|e| StoreError::Crypto(format!("argon2id: {e}")))?;
    Ok(key)
}

/// Encrypt plaintext with ChaCha20Poly1305.
fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<(Vec<u8>, [u8; NONCE_LEN]), StoreError> {
    let mut nonce_bytes = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);

    let cipher = ChaCha20Poly1305::new_from_slice(key)
        .map_err(|e| StoreError::Crypto(format!("cipher init: {e}")))?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| StoreError::Crypto(format!("encrypt: {e}")))?;

    Ok((ciphertext, nonce_bytes))
}

/// Decrypt ciphertext with ChaCha20Poly1305.
fn decrypt(key: &[u8; 32], ciphertext: &[u8], nonce_bytes: &[u8]) -> Result<Vec<u8>, StoreError> {
    let cipher = ChaCha20Poly1305::new_from_slice(key)
        .map_err(|e| StoreError::Crypto(format!("cipher init: {e}")))?;
    let nonce = Nonce::from_slice(nonce_bytes);
    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| StoreError::BadPassphrase)
}

/// Set restrictive permissions on a path (Unix only).
#[cfg(unix)]
fn restrict_permissions(path: &Path, mode: u32) -> Result<(), StoreError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    Ok(())
}

/// Encrypted secret store.
///
/// Backed by a SQLite database with per-value ChaCha20Poly1305 encryption.
/// The encryption key is derived from a passphrase via argon2id.
pub struct SecretStore {
    conn: Connection,
    /// Derived encryption key — zeroized on drop.
    enc_key: [u8; 32],
}

impl std::fmt::Debug for SecretStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretStore")
            .field("enc_key", &"[REDACTED]")
            .finish()
    }
}

impl Drop for SecretStore {
    fn drop(&mut self) {
        self.enc_key.zeroize();
    }
}

impl SecretStore {
    /// Open (or create) the default secrets store at `~/.styrene/secrets.db`.
    pub fn open_default(passphrase: &[u8]) -> Result<Self, StoreError> {
        Self::open(default_path()?, passphrase)
    }

    /// Open (or create) a secrets store at the given path.
    pub fn open(path: impl AsRef<Path>, passphrase: &[u8]) -> Result<Self, StoreError> {
        let path = path.as_ref();

        // Create parent directory with restrictive permissions.
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
            #[cfg(unix)]
            restrict_permissions(parent, 0o700)?;
        }

        let conn = Connection::open(path)?;

        // Set restrictive permissions on the database file.
        #[cfg(unix)]
        restrict_permissions(path, 0o600)?;

        // Enable WAL mode and busy timeout for safe concurrent access.
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
            PRAGMA busy_timeout = 5000;",
        )?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta (
                key   TEXT PRIMARY KEY,
                value BLOB NOT NULL
            );
            CREATE TABLE IF NOT EXISTS secrets (
                key        TEXT PRIMARY KEY,
                value      BLOB NOT NULL,
                nonce      BLOB NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );",
        )?;

        // Derive encryption key from passphrase.
        let salt = load_or_create_salt(&conn)?;
        let enc_key = derive_key(passphrase, &salt)?;

        // Verify passphrase against stored verification tag.
        verify_or_init_passphrase(&conn, &enc_key)?;

        Ok(Self { conn, enc_key })
    }

    /// Open a store using a passphrase from the OS keychain.
    ///
    /// On first use, generates a random passphrase and stores it in the
    /// keychain. On subsequent uses, retrieves the stored passphrase.
    /// This provides encrypted-at-rest secrets with zero user interaction.
    #[cfg(feature = "keychain")]
    pub fn open_with_keychain(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let mut passphrase = keychain_passphrase()?;
        let result = Self::open(path, passphrase.as_bytes());
        passphrase.zeroize();
        result
    }

    /// Open the default store using the OS keychain for the passphrase.
    #[cfg(feature = "keychain")]
    pub fn open_default_keychain() -> Result<Self, StoreError> {
        Self::open_with_keychain(default_path()?)
    }

    /// Retrieve a secret by key. Returns `None` if the key doesn't exist.
    pub fn get(&self, key: &str) -> Result<Option<SecretValue>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT value, nonce FROM secrets WHERE key = ?1")?;

        let result = stmt.query_row(params![key], |row| {
            let value: Vec<u8> = row.get(0)?;
            let nonce: Vec<u8> = row.get(1)?;
            Ok((value, nonce))
        });

        match result {
            Ok((ciphertext, nonce)) => {
                let plaintext = decrypt(&self.enc_key, &ciphertext, &nonce)?;
                // Move directly into SecretBox — no clone.
                Ok(Some(SecretBox::new(Box::new(plaintext))))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StoreError::Db(e)),
        }
    }

    /// Store a secret. Overwrites any existing value for the key.
    pub fn set(&self, key: &str, value: &[u8]) -> Result<(), StoreError> {
        let (ciphertext, nonce) = encrypt(&self.enc_key, value)?;

        self.conn.execute(
            "INSERT INTO secrets (key, value, nonce, created_at, updated_at)
             VALUES (?1, ?2, ?3, datetime('now'), datetime('now'))
             ON CONFLICT(key) DO UPDATE SET
                value = excluded.value,
                nonce = excluded.nonce,
                updated_at = datetime('now')",
            params![key, ciphertext, nonce.as_slice()],
        )?;

        Ok(())
    }

    /// List all stored secret keys (not values).
    pub fn list(&self) -> Result<Vec<String>, StoreError> {
        let mut stmt = self.conn.prepare("SELECT key FROM secrets ORDER BY key")?;
        let keys = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(keys)
    }

    /// Delete a secret. Returns `true` if the key existed.
    pub fn delete(&self, key: &str) -> Result<bool, StoreError> {
        let count = self
            .conn
            .execute("DELETE FROM secrets WHERE key = ?1", params![key])?;
        Ok(count > 0)
    }
}

/// Default store path: `~/.styrene/secrets.db`.
///
/// Returns an error if `HOME` is not set or empty, rather than falling
/// back to the current directory (which could be world-writable).
pub fn default_path() -> Result<PathBuf, StoreError> {
    let home = std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| {
            StoreError::Crypto(
                "HOME environment variable not set — cannot determine secrets store path".into(),
            )
        })?;
    Ok(home.join(".styrene").join("secrets.db"))
}

/// Load the master salt from the meta table, or create one if this is a new store.
///
/// Uses `INSERT OR IGNORE` to avoid race conditions when multiple
/// processes try to create the store simultaneously.
fn load_or_create_salt(conn: &Connection) -> Result<Vec<u8>, StoreError> {
    // Try to read existing salt first.
    let existing: Option<Vec<u8>> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'salt'",
            [],
            |row| row.get(0),
        )
        .ok();

    if let Some(salt) = existing {
        return Ok(salt);
    }

    // No salt yet — generate one and try atomic insert.
    let mut salt = vec![0u8; SALT_LEN];
    OsRng.fill_bytes(&mut salt);

    let rows = conn.execute(
        "INSERT OR IGNORE INTO meta (key, value) VALUES ('salt', ?1)",
        params![salt],
    )?;

    if rows == 1 {
        // We created the salt.
        return Ok(salt);
    }

    // Another process beat us — read theirs.
    salt.zeroize();
    conn.query_row(
        "SELECT value FROM meta WHERE key = 'salt'",
        [],
        |row| row.get(0),
    )
    .map_err(StoreError::Db)
}

/// Verify the passphrase against a stored verification ciphertext,
/// or create the verification entry if this is a new store.
///
/// On first use, generates a random challenge and encrypts it. On
/// subsequent opens, decrypts the stored challenge — failure means
/// wrong passphrase.
fn verify_or_init_passphrase(conn: &Connection, enc_key: &[u8; 32]) -> Result<(), StoreError> {
    let existing: Option<(Vec<u8>, Vec<u8>)> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'verify'",
            [],
            |row| {
                let blob: Vec<u8> = row.get(0)?;
                Ok(blob)
            },
        )
        .ok()
        .and_then(|blob| {
            // verify blob format: [nonce:12][ciphertext:...]
            if blob.len() <= NONCE_LEN {
                return None;
            }
            let nonce = blob[..NONCE_LEN].to_vec();
            let ct = blob[NONCE_LEN..].to_vec();
            Some((ct, nonce))
        });

    match existing {
        Some((ciphertext, nonce)) => {
            // Try to decrypt — if it fails, wrong passphrase.
            decrypt(enc_key, &ciphertext, &nonce)?;
            Ok(())
        }
        None => {
            // New store — generate random challenge, encrypt and store it.
            let mut challenge = [0u8; VERIFY_CHALLENGE_LEN];
            OsRng.fill_bytes(&mut challenge);

            let (ciphertext, nonce) = encrypt(enc_key, &challenge)?;
            challenge.zeroize();

            let mut blob = Vec::with_capacity(NONCE_LEN + ciphertext.len());
            blob.extend_from_slice(&nonce);
            blob.extend_from_slice(&ciphertext);
            conn.execute(
                "INSERT INTO meta (key, value) VALUES ('verify', ?1)",
                params![blob],
            )?;
            Ok(())
        }
    }
}

/// Keychain service name for the master passphrase.
#[cfg(feature = "keychain")]
const KEYCHAIN_SERVICE: &str = "styrene-secrets";
/// Keychain user name for the master passphrase.
#[cfg(feature = "keychain")]
const KEYCHAIN_USER: &str = "master-passphrase";

/// Get or create the master passphrase in the OS keychain.
///
/// Returns a `String` that the caller must zeroize after use.
///
/// **Safety:** If the keychain is empty but a store already exists on disk,
/// this means the store was created with a different passphrase (e.g. via
/// `STYRENE_SECRETS_PASSPHRASE`). In that case, we refuse to generate a
/// new passphrase — the user must explicitly migrate with
/// `styrene-secrets keychain-migrate`.
#[cfg(feature = "keychain")]
fn keychain_passphrase() -> Result<String, StoreError> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_USER)
        .map_err(|e| StoreError::Crypto(format!("keychain entry: {e}")))?;

    match entry.get_password() {
        Ok(pass) => Ok(pass),
        Err(keyring::Error::NoEntry) => {
            // Keychain is empty. Before generating a new passphrase, check
            // whether a store already exists — if so, it was created with a
            // different passphrase and we must not silently replace it.
            if let Ok(path) = default_path() {
                if path.exists() {
                    return Err(StoreError::Crypto(
                        "secrets store exists but keychain has no passphrase — \
                         the store was likely created with STYRENE_SECRETS_PASSPHRASE. \
                         Run 'styrene-secrets keychain-migrate' to re-key the store, \
                         or set STYRENE_SECRETS_PASSPHRASE to use the existing passphrase."
                            .into(),
                    ));
                }
            }

            // No existing store — safe to generate a new passphrase.
            let mut bytes = [0u8; 32];
            OsRng.fill_bytes(&mut bytes);
            let pass = hex::encode(bytes);
            bytes.zeroize();
            entry
                .set_password(&pass)
                .map_err(|e| StoreError::Crypto(format!("keychain set: {e}")))?;
            Ok(pass)
        }
        Err(e) => Err(StoreError::Crypto(format!("keychain get: {e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    fn temp_store(passphrase: &[u8]) -> (SecretStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.db");
        let store = SecretStore::open(&path, passphrase).unwrap();
        (store, dir)
    }

    #[test]
    fn set_and_get_roundtrip() {
        let (store, _dir) = temp_store(b"test-pass");
        store.set("forge.github.token", b"ghp_abc123").unwrap();

        let val = store.get("forge.github.token").unwrap().unwrap();
        assert_eq!(val.expose_secret().as_slice(), b"ghp_abc123");
    }

    #[test]
    fn get_missing_key_returns_none() {
        let (store, _dir) = temp_store(b"test-pass");
        assert!(store.get("nonexistent").unwrap().is_none());
    }

    #[test]
    fn set_overwrites_existing() {
        let (store, _dir) = temp_store(b"test-pass");
        store.set("key", b"v1").unwrap();
        store.set("key", b"v2").unwrap();

        let val = store.get("key").unwrap().unwrap();
        assert_eq!(val.expose_secret().as_slice(), b"v2");
    }

    #[test]
    fn list_returns_sorted_keys() {
        let (store, _dir) = temp_store(b"test-pass");
        store.set("c.key", b"v").unwrap();
        store.set("a.key", b"v").unwrap();
        store.set("b.key", b"v").unwrap();

        assert_eq!(store.list().unwrap(), vec!["a.key", "b.key", "c.key"]);
    }

    #[test]
    fn delete_existing_key() {
        let (store, _dir) = temp_store(b"test-pass");
        store.set("key", b"v").unwrap();
        assert!(store.delete("key").unwrap());
        assert!(store.get("key").unwrap().is_none());
    }

    #[test]
    fn delete_missing_key_returns_false() {
        let (store, _dir) = temp_store(b"test-pass");
        assert!(!store.delete("nope").unwrap());
    }

    #[test]
    fn reopen_with_same_passphrase() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.db");

        {
            let store = SecretStore::open(&path, b"pass").unwrap();
            store.set("key", b"value").unwrap();
        }

        let store = SecretStore::open(&path, b"pass").unwrap();
        let val = store.get("key").unwrap().unwrap();
        assert_eq!(val.expose_secret().as_slice(), b"value");
    }

    #[test]
    fn reopen_with_wrong_passphrase_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.db");

        {
            let _store = SecretStore::open(&path, b"correct").unwrap();
        }

        let err = SecretStore::open(&path, b"wrong").unwrap_err();
        assert!(
            matches!(err, StoreError::BadPassphrase),
            "expected BadPassphrase, got: {err}"
        );
    }

    #[test]
    fn empty_store_list_is_empty() {
        let (store, _dir) = temp_store(b"test-pass");
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn binary_secret_values() {
        let (store, _dir) = temp_store(b"test-pass");
        let binary = vec![0x00, 0xff, 0xfe, 0x01, 0x80];
        store.set("binary.secret", &binary).unwrap();

        let val = store.get("binary.secret").unwrap().unwrap();
        assert_eq!(val.expose_secret().as_slice(), &binary);
    }

    #[cfg(unix)]
    #[test]
    fn database_file_has_restricted_permissions() {
        let (_store, dir) = temp_store(b"test-pass");
        let path = dir.path().join("secrets.db");

        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::metadata(&path).unwrap().permissions();
        assert_eq!(perms.mode() & 0o777, 0o600);
    }
}
