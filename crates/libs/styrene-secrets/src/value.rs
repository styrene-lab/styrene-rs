//! Secret value types — thin wrappers around [`secrecy`] crate types.
//!
//! Re-exports `secrecy` primitives and provides a [`SecretValue`] alias
//! for byte-oriented secrets (the common case for API tokens, credentials, etc.).

pub use secrecy::{ExposeSecret, ExposeSecretMut, SecretBox, SecretString};

/// A secret byte buffer that is zeroized when dropped.
///
/// This is the primary type returned by [`crate::resolve()`]. Access the
/// underlying bytes via the [`ExposeSecret`] trait:
///
/// ```
/// use styrene_secrets::value::{SecretValue, ExposeSecret};
///
/// let secret = secret_from_str("hunter2");
/// assert_eq!(secret.expose_secret().as_slice(), b"hunter2");
///
/// # use styrene_secrets::value::secret_from_str;
/// ```
pub type SecretValue = SecretBox<Vec<u8>>;

/// Create a [`SecretValue`] from a string slice.
pub fn secret_from_str(s: &str) -> SecretValue {
    SecretBox::new(Box::new(s.as_bytes().to_vec()))
}

/// Create a [`SecretValue`] from a byte vector.
pub fn secret_from_bytes(bytes: Vec<u8>) -> SecretValue {
    SecretBox::new(Box::new(bytes))
}

/// Extension methods for [`SecretValue`].
pub trait SecretValueExt {
    /// Expose the secret as a UTF-8 string slice.
    fn expose_str(&self) -> Result<&str, std::str::Utf8Error>;
}

impl SecretValueExt for SecretValue {
    fn expose_str(&self) -> Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(self.expose_secret().as_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expose_returns_bytes() {
        let sv = secret_from_bytes(b"hunter2".to_vec());
        assert_eq!(sv.expose_secret().as_slice(), b"hunter2");
    }

    #[test]
    fn from_string_and_expose_str() {
        let sv = secret_from_str("ghp_abc123");
        assert_eq!(sv.expose_str().unwrap(), "ghp_abc123");
    }

    #[test]
    fn expose_str_invalid_utf8() {
        let sv = secret_from_bytes(vec![0xff, 0xfe]);
        assert!(sv.expose_str().is_err());
    }

    #[test]
    fn debug_is_redacted() {
        let sv = secret_from_str("super-secret");
        let dbg = format!("{:?}", sv);
        assert!(!dbg.contains("super-secret"));
    }
}
