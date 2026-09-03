//! Windows credential storage: DPAPI ciphertext in user-only files.
//!
//! DPAPI encrypts but does not store, so the ciphertext goes in a file. That file is created
//! through the shared private-file primitive, so it carries the same user-only DACL as the
//! isolated homes (`codex_home::permissions`).
//!
//! Scope is `CurrentUser`: `CRYPTPROTECT_LOCAL_MACHINE` is never passed, which is what binds
//! the blob to this user's master key. That boundary is other users on the machine, which is
//! exactly the threat model `SECURITY.md` states - an attacker already running code as this
//! user is out of scope, and no user-space design changes that.

use std::ffi::c_void;
use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};

use super::secret::{CredentialRef, Secret};
use super::store::{SecretStore, unavailable};
use crate::codex_home::permissions;
use crate::diagnostics::Result;

/// Application-specific entropy mixed into every blob.
///
/// This is not a key and is not treated as one - it ships inside the binary. Its job is to keep
/// a blob from being decryptable by unrelated software that merely happens to run as the same
/// user, and to make a blob from another product fail closed here.
const ENTROPY: &[u8] = b"toglet.credentials.v1";

const EXTENSION: &str = "dpapi";

/// Stores DPAPI blobs as files under one directory.
pub struct WindowsSecretStore {
    directory: PathBuf,
}

impl WindowsSecretStore {
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

impl SecretStore for WindowsSecretStore {
    fn store(&self, reference: &CredentialRef, secret: &Secret) -> Result<()> {
        let ciphertext = protect(secret.expose())?;
        let path = self.path(reference);

        // Replacing means removing first: `write_private_file` refuses to overwrite, which is
        // what keeps it from ever widening an existing file's permissions.
        remove_if_present(&path)?;
        permissions::write_private_file(&path, &ciphertext)
            .map_err(|error| unavailable(&error.to_string()))
    }

    fn load(&self, reference: &CredentialRef) -> Result<Secret> {
        let ciphertext =
            std::fs::read(self.path(reference)).map_err(|error| unavailable(&error.to_string()))?;
        unprotect(&ciphertext)
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

fn protect(plaintext: &[u8]) -> Result<Vec<u8>> {
    with_blob(plaintext, ENTROPY, |input, entropy, output| {
        // SAFETY: both input blobs point at live slices; `output` is a zeroed blob the API
        // fills with a LocalAlloc'd buffer.
        unsafe {
            CryptProtectData(
                input,
                std::ptr::null(),
                entropy,
                std::ptr::null(),
                std::ptr::null(),
                // No UI may appear: this runs on a refresh timer, not in front of a user.
                CRYPTPROTECT_UI_FORBIDDEN,
                output,
            )
        }
    })
    .map_err(|()| unavailable("the credential store refused to encrypt"))
}

fn unprotect(ciphertext: &[u8]) -> Result<Secret> {
    let plaintext = with_blob(ciphertext, ENTROPY, |input, entropy, output| {
        // SAFETY: as above.
        unsafe {
            CryptUnprotectData(
                input,
                std::ptr::null_mut(),
                entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                output,
            )
        }
    })
    .map_err(|()| unavailable("the credential store refused to decrypt"))?;

    Ok(Secret::new(plaintext))
}

/// Runs one DPAPI call and copies the result out of the OS-allocated buffer.
///
/// The output buffer is `LocalAlloc`'d by the API and freed here on every path, including the
/// failure path, so a rejected decryption cannot leak it.
fn with_blob(
    data: &[u8],
    entropy: &[u8],
    call: impl FnOnce(
        *const CRYPT_INTEGER_BLOB,
        *const CRYPT_INTEGER_BLOB,
        *mut CRYPT_INTEGER_BLOB,
    ) -> i32,
) -> std::result::Result<Vec<u8>, ()> {
    // The input blobs only borrow `data` and `entropy`; the API does not take ownership of
    // either, so there is nothing to release for them.
    let input = blob(data)?;
    let entropy = blob(entropy)?;
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };

    let ok = call(&raw const input, &raw const entropy, &raw mut output);

    if ok == 0 || output.pbData.is_null() {
        return Err(());
    }

    // SAFETY: on success the API guarantees `pbData` points at `cbData` readable bytes.
    let copied =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
    // SAFETY: the buffer came from the API's LocalAlloc and is freed exactly once.
    unsafe { LocalFree(output.pbData.cast::<c_void>()) };
    Ok(copied)
}

fn blob(data: &[u8]) -> std::result::Result<CRYPT_INTEGER_BLOB, ()> {
    Ok(CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(data.len()).map_err(|_| ())?,
        // The API does not write through this pointer for input blobs.
        pbData: data.as_ptr().cast_mut(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_home::IsolatedHome;
    use crate::diagnostics::{ErrorCode, Phase};

    const TOKEN: &[u8] = b"eyJhbGciOiJIUzI1NiJ9.a-refresh-token-that-must-never-be-readable";

    fn store() -> (IsolatedHome, WindowsSecretStore) {
        // A private directory with automatic cleanup is exactly what a store needs in a test.
        let directory = IsolatedHome::create(Phase::Storage).expect("scratch directory");
        let store = WindowsSecretStore::new(directory.path().to_path_buf());
        (directory, store)
    }

    #[test]
    fn a_stored_secret_round_trips() {
        let (_directory, store) = store();
        let reference = CredentialRef::new("acct-1").expect("valid reference");

        store
            .store(&reference, &Secret::new(TOKEN.to_vec()))
            .expect("the secret is stored");
        let loaded = store.load(&reference).expect("the secret is readable");

        assert_eq!(loaded.expose(), TOKEN);
    }

    #[test]
    fn what_lands_on_disk_contains_no_plaintext_and_is_user_only() {
        let (directory, store) = store();
        let reference = CredentialRef::new("acct-2").expect("valid reference");
        store
            .store(&reference, &Secret::new(TOKEN.to_vec()))
            .expect("the secret is stored");

        let path = directory.path().join("acct-2.dpapi");
        let on_disk = std::fs::read(&path).expect("the blob is readable");

        assert!(
            !on_disk.windows(TOKEN.len()).any(|window| window == TOKEN),
            "the plaintext token appeared verbatim in the stored blob"
        );
        assert!(
            on_disk.len() > TOKEN.len(),
            "a blob shorter than the input is suspicious"
        );
        permissions::assert_private(&path);
    }

    #[test]
    fn a_blob_protected_with_different_entropy_cannot_be_read_back() {
        let (_directory, _store) = store();
        // Encrypt with entropy this build does not use, then try to open it the normal way.
        let foreign = with_blob(
            TOKEN,
            b"some.other.product",
            |input, entropy, output| unsafe {
                CryptProtectData(
                    input,
                    std::ptr::null(),
                    entropy,
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    output,
                )
            },
        )
        .expect("the foreign blob is produced");

        let error = unprotect(&foreign).expect_err("a foreign blob must not decrypt");

        assert_eq!(error.code(), ErrorCode::CredentialStoreUnavailable);
    }

    #[test]
    fn corrupted_ciphertext_reports_a_stable_code_rather_than_returning_garbage() {
        let (directory, store) = store();
        let reference = CredentialRef::new("acct-3").expect("valid reference");
        store
            .store(&reference, &Secret::new(TOKEN.to_vec()))
            .expect("the secret is stored");
        let path = directory.path().join("acct-3.dpapi");
        let mut blob = std::fs::read(&path).expect("the blob is readable");
        blob[20] ^= 0xFF;
        std::fs::write(&path, &blob).expect("the blob is rewritten");

        let error = store
            .load(&reference)
            .expect_err("tampering must be detected");

        assert_eq!(error.code(), ErrorCode::CredentialStoreUnavailable);
        assert!(error.retryable());
    }

    #[test]
    fn a_missing_entry_reports_the_store_error_and_never_an_empty_secret() {
        let (_directory, store) = store();
        let reference = CredentialRef::new("absent").expect("valid reference");

        assert!(
            !store
                .contains(&reference)
                .expect("containment is checkable")
        );
        assert_eq!(
            store.load(&reference).expect_err("nothing to load").code(),
            ErrorCode::CredentialStoreUnavailable
        );
    }

    #[test]
    fn storing_twice_replaces_rather_than_failing_or_widening_permissions() {
        let (directory, store) = store();
        let reference = CredentialRef::new("acct-4").expect("valid reference");

        store
            .store(&reference, &Secret::new(b"first".to_vec()))
            .expect("first write");
        store
            .store(&reference, &Secret::new(b"second".to_vec()))
            .expect("second write replaces the first");

        assert_eq!(
            store.load(&reference).expect("readable").expose(),
            b"second"
        );
        permissions::assert_private(&directory.path().join("acct-4.dpapi"));
    }

    #[test]
    fn deleting_removes_the_file_and_deleting_again_still_succeeds() {
        let (_directory, store) = store();
        let reference = CredentialRef::new("acct-5").expect("valid reference");
        store
            .store(&reference, &Secret::new(TOKEN.to_vec()))
            .expect("the secret is stored");

        store.delete(&reference).expect("the entry is removed");

        assert!(
            !store
                .contains(&reference)
                .expect("containment is checkable")
        );
        store.delete(&reference).expect("removing nothing succeeds");
    }
}
