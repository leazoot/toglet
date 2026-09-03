//! Decrypting one account's credentials into a throwaway Codex home.
//!
//! This is the only path by which a stored credential becomes readable by Codex:
//!
//! 1. a restricted temporary directory, private from the moment it exists;
//! 2. the decrypted `auth.json`, written with its permissions already applied;
//! 3. the app server runs against it;
//! 4. the directory is removed.
//!
//! Step 4 is a `Drop` guard inherited from [`IsolatedHome`], so it also runs on an early return
//! and on a panic. Deletion is still best effort - it cannot promise erasure from the
//! underlying storage - which is why step 1 matters more than step 4.
//!
//! The plaintext exists only as a [`Secret`] local to `open`, and `Secret` clears its buffer on
//! drop. Nothing plaintext is stored in the session: the struct holds a directory and nothing
//! else.

use super::secret::CredentialRef;
use super::store::SecretStore;
use crate::codex_home::IsolatedHome;
use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};

/// The file name Codex reads credentials from.
const AUTH_FILE: &str = "auth.json";

/// A Codex home holding one account's decrypted credentials.
pub struct CredentialSession {
    home: IsolatedHome,
}

impl CredentialSession {
    /// Decrypts `reference` into a fresh isolated home.
    ///
    /// If anything after the directory exists fails, the guard removes it on the way out; the
    /// caller never has to unwind this by hand.
    pub fn open(store: &dyn SecretStore, reference: &CredentialRef, phase: Phase) -> Result<Self> {
        let home = IsolatedHome::create(phase)?;
        let secret = store.load(reference)?;

        // `write_private_file` applies the permissions as part of creating the file, so the
        // decrypted credentials are never briefly readable by anyone else.
        crate::codex_home::permissions::write_private_file(
            &home.path().join(AUTH_FILE),
            secret.expose(),
        )
        .map_err(|error| unwritable(phase, &error.to_string()))?;

        // `secret` drops here and clears its buffer. From this point the plaintext exists only
        // in the file, which the guard removes.
        Ok(Self { home })
    }

    /// The home to pass as `CODEX_HOME`.
    pub fn home(&self) -> &IsolatedHome {
        &self.home
    }

    /// Hands the home to something that takes ownership of it, such as an app server client.
    /// The cleanup guard travels with it.
    pub fn into_home(self) -> IsolatedHome {
        self.home
    }
}

fn unwritable(phase: Phase, detail: &str) -> TogletError {
    TogletError::new(
        ErrorCode::CodexHomeUnwritable,
        phase,
        true,
        UserAction::Retry,
    )
    .with_detail(detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_home::permissions;
    use crate::credentials::{MemorySecretStore, Secret};

    const AUTH: &[u8] =
        br#"{"auth_mode":"chatgpt","tokens":{"access_token":"eyJhbGciOiJIUzI1NiJ9.x"}}"#;

    fn store_with_entry() -> (MemorySecretStore, CredentialRef) {
        let store = MemorySecretStore::new();
        let reference = CredentialRef::new("acct-1").expect("valid reference");
        store
            .store(&reference, &Secret::new(AUTH.to_vec()))
            .expect("the credential is stored");
        (store, reference)
    }

    #[test]
    fn the_decrypted_auth_file_is_written_privately_and_matches_the_stored_bytes() {
        let (store, reference) = store_with_entry();

        let session =
            CredentialSession::open(&store, &reference, Phase::ReadQuota).expect("session opens");

        let path = session.home().path().join(AUTH_FILE);
        permissions::assert_private(&path);
        assert_eq!(std::fs::read(&path).expect("readable"), AUTH);
    }

    #[test]
    fn the_home_holds_only_the_config_and_the_auth_file() {
        let (store, reference) = store_with_entry();
        let session =
            CredentialSession::open(&store, &reference, Phase::ReadQuota).expect("session opens");

        let mut entries: Vec<String> = std::fs::read_dir(session.home().path())
            .expect("home is readable")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        entries.sort();

        assert_eq!(
            entries,
            vec!["auth.json".to_owned(), "config.toml".to_owned()]
        );
    }

    #[test]
    fn dropping_the_session_removes_the_decrypted_credentials() {
        let (store, reference) = store_with_entry();
        let session =
            CredentialSession::open(&store, &reference, Phase::ReadQuota).expect("session opens");
        let path = session.home().path().to_path_buf();

        drop(session);

        assert!(!path.exists());
    }

    #[test]
    fn a_panic_while_the_credentials_are_on_disk_still_removes_them() {
        let (store, reference) = store_with_entry();
        let observed = std::sync::Arc::new(std::sync::Mutex::new(std::path::PathBuf::new()));
        let recorder = std::sync::Arc::clone(&observed);

        let result = std::panic::catch_unwind(move || {
            let session = CredentialSession::open(&store, &reference, Phase::ReadQuota)
                .expect("session opens");
            *recorder.lock().expect("lock") = session.home().path().to_path_buf();
            panic!("simulated failure while the credentials are decrypted");
        });

        assert!(result.is_err());
        let path = observed.lock().expect("lock").clone();
        assert!(!path.as_os_str().is_empty());
        assert!(!path.exists(), "decrypted credentials survived a panic");
    }

    #[test]
    fn a_store_failure_names_no_path_and_leaves_no_credentials() {
        let store = MemorySecretStore::unavailable();
        let reference = CredentialRef::new("acct-1").expect("valid reference");

        // Matched rather than `expect_err`, which would need `Debug` on the session - and a
        // `Debug` on a type that owns decrypted credentials is a hazard worth not having.
        let error = match CredentialSession::open(&store, &reference, Phase::ReadQuota) {
            Ok(_) => panic!("an unavailable store must fail"),
            Err(error) => error,
        };

        assert_eq!(error.code(), ErrorCode::CredentialStoreUnavailable);
        let detail = error.detail().unwrap_or_default();
        // The isolated home was created before the store was consulted, so its path is exactly
        // the kind of value that could leak into this error.
        for marker in [":\\", "/", "toglet-", "Temp"] {
            assert!(
                !detail.contains(marker),
                "the error named {marker:?}: {detail}"
            );
        }
        // The home itself is cleaned up by the guard on the way out; that path is covered by
        // `codex_home::isolated`'s early-return test and by the whole-suite check that no
        // `toglet-*` directory survives a run.
    }

    #[test]
    fn the_home_survives_being_handed_over() {
        let (store, reference) = store_with_entry();
        let session =
            CredentialSession::open(&store, &reference, Phase::ReadQuota).expect("session opens");

        let home = session.into_home();

        assert!(home.path().join(AUTH_FILE).is_file());
        let path = home.path().to_path_buf();
        drop(home);
        assert!(!path.exists(), "the guard must travel with the home");
    }
}
