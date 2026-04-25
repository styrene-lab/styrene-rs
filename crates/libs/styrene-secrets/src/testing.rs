//! Test utilities for extensions that depend on `styrene-secrets`.
//!
//! Provides a [`MockStore`] that can be pre-loaded with secret values,
//! avoiding the need for an encrypted store or environment variables
//! during testing.

use std::collections::HashMap;

use secrecy::SecretBox;

use crate::value::SecretValue;

/// In-memory mock secret store for testing.
///
/// # Example
///
/// ```
/// use styrene_secrets::testing::MockStore;
/// use styrene_secrets::value::ExposeSecret;
///
/// let store = MockStore::new(&[
///     ("forge.github.token", "ghp_test123"),
///     ("forge.forgejo.token", "tok_test456"),
/// ]);
///
/// let token = store.get("forge.github.token").unwrap();
/// assert_eq!(token.expose_secret().as_slice(), b"ghp_test123");
/// assert!(store.get("nonexistent").is_none());
/// ```
pub struct MockStore {
    entries: HashMap<String, Vec<u8>>,
}

impl MockStore {
    /// Create a mock store pre-loaded with the given key-value pairs.
    pub fn new(entries: &[(&str, &str)]) -> Self {
        let entries = entries
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.as_bytes().to_vec()))
            .collect();
        Self { entries }
    }

    /// Look up a secret by key.
    pub fn get(&self, key: &str) -> Option<SecretValue> {
        self.entries
            .get(key)
            .map(|v| SecretBox::new(Box::new(v.clone())))
    }

    /// List all keys in the mock store.
    pub fn list(&self) -> Vec<String> {
        let mut keys: Vec<_> = self.entries.keys().cloned().collect();
        keys.sort();
        keys
    }

    /// Whether the mock store contains the given key.
    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    #[test]
    fn mock_store_get() {
        let store = MockStore::new(&[("forge.github.token", "ghp_abc")]);
        let val = store.get("forge.github.token").unwrap();
        assert_eq!(val.expose_secret().as_slice(), b"ghp_abc");
    }

    #[test]
    fn mock_store_missing_key() {
        let store = MockStore::new(&[]);
        assert!(store.get("nope").is_none());
    }

    #[test]
    fn mock_store_list() {
        let store = MockStore::new(&[("b.key", "v"), ("a.key", "v")]);
        assert_eq!(store.list(), vec!["a.key", "b.key"]);
    }

    #[test]
    fn mock_store_contains() {
        let store = MockStore::new(&[("exists", "yes")]);
        assert!(store.contains("exists"));
        assert!(!store.contains("nope"));
    }
}
