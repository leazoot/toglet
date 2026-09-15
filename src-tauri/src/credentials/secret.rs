//! The plaintext type and the key that names a stored credential.

use std::fmt;

use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};

/// Plaintext credential material.
///
/// `Debug` prints only the length; there is no `Display` or `Serialize`; `Drop` zeroes the buffer
/// (best effort: reallocations and SSDs may keep copies, so permissions remain the defence).
pub struct Secret(Vec<u8>);

impl Secret {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    pub fn expose(&self) -> &[u8] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        // Best-effort scrub, not a guarantee of erasure.
        self.0.fill(0);
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Secret({} bytes, redacted)", self.0.len())
    }
}

/// The key a credential is stored under. Validated because it becomes a file name: a separator
/// or `..` would escape the store's directory.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CredentialRef(String);

/// Long enough to be unique, short enough to stay a valid file name on both platforms.
const MAX_REF_LEN: usize = 64;

impl CredentialRef {
    /// Accepts lowercase ASCII letters, digits and hyphens only. Toglet generates the value, so
    /// anything else is a bug.
    pub fn new(value: &str) -> Result<Self> {
        let valid = !value.is_empty()
            && value.len() <= MAX_REF_LEN
            && value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');

        if valid {
            Ok(Self(value.to_owned()))
        } else {
            Err(
                TogletError::new(ErrorCode::Internal, Phase::Storage, false, UserAction::None)
                    .with_detail("credential reference is not in the allowed form"),
            )
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_shows_a_length_and_never_the_content() {
        let secret = Secret::new(b"eyJhbGciOiJIUzI1NiJ9.payload".to_vec());

        let rendered = format!("{secret:?}");

        assert!(!rendered.contains("eyJ"));
        assert_eq!(rendered, "Secret(28 bytes, redacted)");
    }

    #[test]
    fn a_reference_rejects_anything_that_could_escape_the_store_directory() {
        for hostile in ["..", "a/b", r"a\b", "a:b", "", "A", "a b", "a.b"] {
            assert!(
                CredentialRef::new(hostile).is_err(),
                "{hostile:?} must be rejected"
            );
        }
        assert!(CredentialRef::new(&"a".repeat(MAX_REF_LEN + 1)).is_err());
    }

    #[test]
    fn a_generated_reference_is_accepted() {
        let reference = CredentialRef::new("acct-4f2b9c1e").expect("a generated id is valid");

        assert_eq!(reference.as_str(), "acct-4f2b9c1e");
    }
}
