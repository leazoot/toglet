//! An in-memory credential store for tests.
//!
//! Integration tests live outside the crate, so this cannot be `#[cfg(test)]` - it has to be a
//! real public type. Two things keep that safe:
//!
//! * It **never touches the filesystem**. Nothing it holds survives the process, so it cannot
//!   become an accidental plaintext-on-disk fallback - the one failure mode that is ruled out
//!   absolutely.
//! * The application never constructs it. Only the platform stores are wired in, and the
//!   release build is checked for development entry points.

use std::collections::BTreeMap;
use std::sync::Mutex;

use super::secret::{CredentialRef, Secret};
use super::store::{SecretStore, unavailable};
use crate::diagnostics::Result;

#[derive(Default)]
pub struct MemorySecretStore {
    entries: Mutex<BTreeMap<CredentialRef, Vec<u8>>>,
    /// When set, every operation fails. Lets a test drive the "credential store unavailable"
    /// path without locking a real Keychain.
    unavailable: bool,
}

impl MemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// A store that refuses every operation, for exercising the unavailable path.
    pub fn unavailable() -> Self {
        Self {
            unavailable: true,
            ..Self::default()
        }
    }

    fn entries(&self) -> Result<std::sync::MutexGuard<'_, BTreeMap<CredentialRef, Vec<u8>>>> {
        if self.unavailable {
            return Err(unavailable(
                "the credential store is simulated as unavailable",
            ));
        }
        Ok(self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()))
    }
}

impl SecretStore for MemorySecretStore {
    fn store(&self, reference: &CredentialRef, secret: &Secret) -> Result<()> {
        self.entries()?
            .insert(reference.clone(), secret.expose().to_vec());
        Ok(())
    }

    fn load(&self, reference: &CredentialRef) -> Result<Secret> {
        self.entries()?
            .get(reference)
            .map(|bytes| Secret::new(bytes.clone()))
            .ok_or_else(|| unavailable("no such credential entry"))
    }

    fn delete(&self, reference: &CredentialRef) -> Result<()> {
        self.entries()?.remove(reference);
        Ok(())
    }

    fn contains(&self, reference: &CredentialRef) -> Result<bool> {
        Ok(self.entries()?.contains_key(reference))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::ErrorCode;

    #[test]
    fn it_behaves_like_the_platform_store_for_the_paths_tests_rely_on() {
        let store = MemorySecretStore::new();
        let reference = CredentialRef::new("acct-1").expect("valid reference");

        assert!(
            !store
                .contains(&reference)
                .expect("containment is checkable")
        );
        store
            .store(&reference, &Secret::new(b"token".to_vec()))
            .expect("stored");
        assert_eq!(store.load(&reference).expect("loaded").expose(), b"token");
        store.delete(&reference).expect("deleted");
        store.delete(&reference).expect("deleting nothing succeeds");
        assert!(store.load(&reference).is_err());
    }

    #[test]
    fn the_unavailable_store_fails_every_operation_with_the_stable_code() {
        let store = MemorySecretStore::unavailable();
        let reference = CredentialRef::new("acct-1").expect("valid reference");

        for error in [
            store
                .store(&reference, &Secret::new(b"x".to_vec()))
                .unwrap_err(),
            store.load(&reference).unwrap_err(),
            store.delete(&reference).unwrap_err(),
            store.contains(&reference).unwrap_err(),
        ] {
            assert_eq!(error.code(), ErrorCode::CredentialStoreUnavailable);
        }
    }
}
