//! Storing credentials Codex refreshed while Toglet was reading a quota.
//!
//! The app server refreshes tokens on its own. If that happens inside a throwaway home, the
//! refreshed credentials are in that directory and about to be deleted with it - and the stored
//! snapshot is now out of date. Where refresh tokens rotate, the stored snapshot would be dead.
//!
//! Detection is a byte comparison, not a guess: the file either changed or it did not.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use super::secret::{CredentialRef, Secret};
use super::store::SecretStore;
use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};

/// Serialises credential writes against a switch reading them.
///
/// **Decided here: a refresh write-back finishes; a switch that starts during one
/// waits.** The reasoning is asymmetric risk. Discarding a refreshed token is not merely
/// inconvenient - if the provider rotates refresh tokens, the snapshot Toglet still holds has
/// already been invalidated, and the account becomes unusable until the user signs in again.
/// Waiting, by contrast, costs a switch the few milliseconds an encrypt-and-write takes.
///
/// `switching` must take this lock around "read the snapshot, replace the default
/// authentication, verify" so a write-back cannot land between the read and the verify.
///
/// Whether Codex actually rotates refresh tokens is **not established** - it has not been
/// observed. The decision deliberately takes the safe side of that uncertainty.
#[derive(Debug, Default)]
pub struct CredentialLock {
    inner: Mutex<()>,
}

impl CredentialLock {
    pub fn new() -> Self {
        Self::default()
    }

    /// Blocks until no other credential write or switch holds the lock.
    pub fn acquire(&self) -> MutexGuard<'_, ()> {
        // A poisoned lock means some other thread panicked while holding it. The data it guards
        // is `()`, so there is nothing to be corrupted - recovering beats refusing to switch
        // for the rest of the process's life.
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// What a write-back did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteBack {
    /// The credentials were untouched by the app server.
    Unchanged,
    /// They were refreshed and the new snapshot is stored.
    Stored,
}

/// Compares the credentials in `home` against `original` and stores them if they changed.
///
/// On failure the stored snapshot is left exactly as it was. That is the important half: a
/// write-back that cannot complete must not destroy the credentials that still work.
pub fn write_back_if_refreshed(
    lock: &CredentialLock,
    store: &dyn SecretStore,
    reference: &CredentialRef,
    home: &Path,
    original: &Secret,
    phase: Phase,
) -> Result<WriteBack> {
    let current = std::fs::read(home.join("auth.json")).map_err(|error| {
        TogletError::new(ErrorCode::AuthFileConflict, phase, true, UserAction::Retry)
            .with_detail(&error.to_string())
    })?;

    if current == original.expose() {
        return Ok(WriteBack::Unchanged);
    }

    let refreshed = Secret::new(current);
    // Held across the store call only. A switch that arrives now waits for it rather than
    // reading a snapshot that is about to be replaced.
    let _guard = lock.acquire();
    store.store(reference, &refreshed)?;
    Ok(WriteBack::Stored)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_home::IsolatedHome;
    use crate::credentials::MemorySecretStore;

    const ORIGINAL: &[u8] = br#"{"auth_mode":"chatgpt","tokens":{"refresh_token":"rt-original"}}"#;
    const REFRESHED: &[u8] = br#"{"auth_mode":"chatgpt","tokens":{"refresh_token":"rt-rotated"}}"#;

    fn setup() -> (
        IsolatedHome,
        MemorySecretStore,
        CredentialRef,
        CredentialLock,
    ) {
        let home = IsolatedHome::create(Phase::ReadQuota).expect("scratch home");
        let store = MemorySecretStore::new();
        let reference = CredentialRef::new("acct-1").expect("valid reference");
        store
            .store(&reference, &Secret::new(ORIGINAL.to_vec()))
            .expect("the original is stored");
        (home, store, reference, CredentialLock::new())
    }

    #[test]
    fn credentials_the_app_server_did_not_touch_are_left_alone() {
        let (home, store, reference, lock) = setup();
        std::fs::write(home.path().join("auth.json"), ORIGINAL).expect("written");

        let outcome = write_back_if_refreshed(
            &lock,
            &store,
            &reference,
            home.path(),
            &Secret::new(ORIGINAL.to_vec()),
            Phase::ReadQuota,
        )
        .expect("comparison succeeds");

        assert_eq!(outcome, WriteBack::Unchanged);
        assert_eq!(store.load(&reference).expect("stored").expose(), ORIGINAL);
    }

    #[test]
    fn refreshed_credentials_replace_the_stored_snapshot() {
        let (home, store, reference, lock) = setup();
        std::fs::write(home.path().join("auth.json"), REFRESHED).expect("written");

        let outcome = write_back_if_refreshed(
            &lock,
            &store,
            &reference,
            home.path(),
            &Secret::new(ORIGINAL.to_vec()),
            Phase::ReadQuota,
        )
        .expect("write-back succeeds");

        assert_eq!(outcome, WriteBack::Stored);
        assert_eq!(
            store.load(&reference).expect("stored").expose(),
            REFRESHED,
            "a rotated refresh token must not be lost with the temporary home"
        );
    }

    #[test]
    fn a_single_changed_byte_is_detected() {
        let (home, store, reference, lock) = setup();
        let mut nearly = ORIGINAL.to_vec();
        let last = nearly.len() - 2;
        nearly[last] = b'X';
        std::fs::write(home.path().join("auth.json"), &nearly).expect("written");

        let outcome = write_back_if_refreshed(
            &lock,
            &store,
            &reference,
            home.path(),
            &Secret::new(ORIGINAL.to_vec()),
            Phase::ReadQuota,
        )
        .expect("write-back succeeds");

        assert_eq!(outcome, WriteBack::Stored, "comparison must be exact");
    }

    #[test]
    fn a_failed_write_back_leaves_the_old_snapshot_intact() {
        let home = IsolatedHome::create(Phase::ReadQuota).expect("scratch home");
        let working = MemorySecretStore::new();
        let reference = CredentialRef::new("acct-1").expect("valid reference");
        working
            .store(&reference, &Secret::new(ORIGINAL.to_vec()))
            .expect("the original is stored");
        std::fs::write(home.path().join("auth.json"), REFRESHED).expect("written");

        // The store refuses everything, as a locked keychain would.
        let broken = MemorySecretStore::unavailable();
        let error = write_back_if_refreshed(
            &lock_of(),
            &broken,
            &reference,
            home.path(),
            &Secret::new(ORIGINAL.to_vec()),
            Phase::ReadQuota,
        )
        .expect_err("an unavailable store must fail");

        assert_eq!(error.code(), ErrorCode::CredentialStoreUnavailable);
        assert_eq!(
            working.load(&reference).expect("stored").expose(),
            ORIGINAL,
            "a failed write-back must not destroy credentials that still work"
        );
    }

    #[test]
    fn a_missing_credential_file_reports_a_conflict_rather_than_storing_nothing() {
        let (home, store, reference, lock) = setup();

        let error = write_back_if_refreshed(
            &lock,
            &store,
            &reference,
            home.path(),
            &Secret::new(ORIGINAL.to_vec()),
            Phase::ReadQuota,
        )
        .expect_err("there is no auth.json to compare");

        assert_eq!(error.code(), ErrorCode::AuthFileConflict);
        assert_eq!(
            store.load(&reference).expect("stored").expose(),
            ORIGINAL,
            "the stored snapshot must survive a comparison that could not run"
        );
    }

    #[test]
    fn the_lock_serialises_two_holders() {
        let lock = std::sync::Arc::new(CredentialLock::new());
        let order = std::sync::Arc::new(Mutex::new(Vec::new()));

        let held = lock.acquire();
        order.lock().expect("lock").push("first-acquired");

        let waiter = {
            let lock = std::sync::Arc::clone(&lock);
            let order = std::sync::Arc::clone(&order);
            std::thread::spawn(move || {
                let _guard = lock.acquire();
                order.lock().expect("lock").push("second-acquired");
            })
        };

        // The waiter cannot have got in while the guard is held.
        std::thread::sleep(std::time::Duration::from_millis(50));
        order.lock().expect("lock").push("first-released");
        drop(held);
        waiter.join().expect("the waiter finishes");

        assert_eq!(
            *order.lock().expect("lock"),
            vec!["first-acquired", "first-released", "second-acquired"]
        );
    }

    fn lock_of() -> CredentialLock {
        CredentialLock::new()
    }
}
