//! Credential storage as user-only files (`0600` in a `0700` directory), used on macOS.
//! Not encryption: the same protection Codex gives its own `auth.json`. The login Keychain is not
//! used because it prompts for the password on every rebuilt, unsigned binary.

use std::path::{Path, PathBuf};

use super::secret::{CredentialRef, Secret};
use super::store::{SecretStore, unavailable};
use crate::codex_home::permissions;
use crate::diagnostics::Result;

const EXTENSION: &str = "credential";

/// Stores credential snapshots as private files under one directory.
pub struct FileSecretStore {
    directory: PathBuf,
}

impl FileSecretStore {
    /// The directory must already exist and be private; the caller owns that decision because
    /// the application data location is settled by `storage`.
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    fn path(&self, reference: &CredentialRef) -> PathBuf {
        // `CredentialRef` is validated to contain no separators, so this cannot escape.
        self.directory
            .join(format!("{}.{EXTENSION}", reference.as_str()))
    }
}

impl SecretStore for FileSecretStore {
    fn store(&self, reference: &CredentialRef, secret: &Secret) -> Result<()> {
        let path = self.path(reference);

        // Replacing means removing first: `write_private_file` refuses to overwrite, which is
        // what keeps it from ever widening an existing file's permissions.
        remove_if_present(&path)?;
        permissions::write_private_file(&path, secret.expose())
            .map_err(|error| unavailable(&error.to_string()))
    }

    fn load(&self, reference: &CredentialRef) -> Result<Secret> {
        std::fs::read(self.path(reference))
            .map(Secret::new)
            .map_err(|error| unavailable(&error.to_string()))
    }

    fn delete(&self, reference: &CredentialRef) -> Result<()> {
        remove_if_present(&self.path(reference))
    }

    fn contains(&self, reference: &CredentialRef) -> Result<bool> {
        Ok(self.path(reference).is_file())
    }
}

fn remove_if_present(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(unavailable(&error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_home::IsolatedHome;
    use crate::diagnostics::{ErrorCode, Phase};

    fn store() -> (IsolatedHome, FileSecretStore) {
        let directory = IsolatedHome::create(Phase::Storage).expect("scratch directory");
        let store = FileSecretStore::new(directory.path().to_path_buf());
        (directory, store)
    }

    #[test]
    fn a_stored_secret_comes_back_unchanged_and_is_gone_after_delete() {
        let (_directory, store) = store();
        let reference = CredentialRef::new("acct-1").expect("valid reference");

        assert!(!store.contains(&reference).expect("checkable"));
        store
            .store(&reference, &Secret::new(b"{\"tokens\":1}".to_vec()))
            .expect("stored");
        assert!(store.contains(&reference).expect("checkable"));
        assert_eq!(
            store.load(&reference).expect("loaded").expose(),
            b"{\"tokens\":1}"
        );

        store.delete(&reference).expect("deleted");
        store.delete(&reference).expect("deleting nothing succeeds");
        assert!(!store.contains(&reference).expect("checkable"));
        assert_eq!(
            store.load(&reference).unwrap_err().code(),
            ErrorCode::CredentialStoreUnavailable
        );
    }

    #[test]
    fn the_file_is_private_from_the_moment_it_exists() {
        let (_directory, store) = store();
        let reference = CredentialRef::new("acct-2").expect("valid reference");

        store
            .store(&reference, &Secret::new(b"x".to_vec()))
            .expect("stored");

        let path = store.path(&reference);
        assert!(permissions::is_private(&path).expect("mode is readable"));
    }

    #[test]
    fn storing_again_replaces_the_content_and_keeps_the_file_private() {
        let (_directory, store) = store();
        let reference = CredentialRef::new("acct-3").expect("valid reference");

        store
            .store(&reference, &Secret::new(b"old".to_vec()))
            .expect("stored");
        store
            .store(&reference, &Secret::new(b"new".to_vec()))
            .expect("replaced");

        assert_eq!(store.load(&reference).expect("loaded").expose(), b"new");
        assert!(permissions::is_private(&store.path(&reference)).expect("mode is readable"));
    }

    #[test]
    fn a_missing_directory_is_reported_as_the_store_being_unavailable_not_as_plaintext() {
        let missing = std::env::temp_dir().join("toglet-no-such-store-dir");
        let store = FileSecretStore::new(missing);
        let reference = CredentialRef::new("acct-4").expect("valid reference");

        let error = store
            .store(&reference, &Secret::new(b"x".to_vec()))
            .unwrap_err();

        assert_eq!(error.code(), ErrorCode::CredentialStoreUnavailable);
    }
}
