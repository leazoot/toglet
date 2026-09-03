//! Reconciling Toglet's stored snapshot with what Codex actually has on disk.
//!
//! Codex refreshes its own tokens, and the user can sign in to a different account without
//! Toglet ever being involved. Both show up the same way - the default `auth.json` changed -
//! and they need opposite treatment: the first is a snapshot to update, the second must never
//! be applied silently.
//!
//! The identity check therefore comes **before** anything is stored. Storing first and checking
//! afterwards would mean a stranger's credentials had already replaced the account's snapshot
//! by the time the mismatch was noticed.

use std::path::Path;

use super::{auth_file, onboarding};
use crate::credentials::{CredentialLock, CredentialRef, SecretStore};
use crate::diagnostics::{ErrorCode, Phase, Result};

/// What Toglet believes is signed in, for the file on disk to be compared against.
#[derive(Debug, Clone, Copy)]
pub struct ActiveAccount<'a> {
    /// Where that account's encrypted snapshot lives.
    pub credentials: &'a CredentialRef,
    /// Its `accountFingerprint`. Irreversible, but still an account identifier: it must not be
    /// logged or returned to the frontend.
    pub fingerprint: &'a str,
}

/// What reconciling found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalChange {
    /// The file matches the stored snapshot byte for byte.
    Unchanged,
    /// The same account, with credentials Codex refreshed. The snapshot now holds them.
    SnapshotUpdated,
    /// Nothing could be concluded, and so nothing was stored. Covers a file that is not JSON -
    /// which is what a half-written one looks like - and one that carries no identity to
    /// compare against, an API-key sign-in among them. Neither may overwrite the snapshot of
    /// an account it might not belong to.
    NotUnderstood,
    /// Codex is signed in as somebody else. **Never applied automatically**: it becomes a
    /// pending item the user resolves by importing it, matching it to a known account, or
    /// ignoring it.
    ExternalLogin {
        /// Irreversible, and still an account identifier - see [`ActiveAccount::fingerprint`].
        fingerprint: String,
        auth_mode: Option<String>,
    },
    /// There is no authentication file: Codex signed out on its own.
    SignedOut,
}

/// Compares the default authentication against the account Toglet thinks is active.
///
/// `active` is `None` when Toglet tracks nobody yet, in which case anything signed in is by
/// definition an external login rather than a refresh.
///
/// Nothing is written unless the identity matches. On any other outcome the stored snapshot is
/// left exactly as it was - including when the file cannot be parsed, which is the state a
/// half-written file presents.
pub fn synchronise(
    lock: &CredentialLock,
    store: &dyn SecretStore,
    active: Option<ActiveAccount<'_>>,
    home: &Path,
    phase: Phase,
) -> Result<ExternalChange> {
    // Checked rather than inferred from a read failure: "signed out" and "could not be read"
    // deserve different answers, and an io error does not distinguish them once it has been
    // mapped to a Toglet error. A file removed between this check and the read below reports a
    // read failure, which is the truthful answer to what happened.
    if !home.join("auth.json").exists() {
        return Ok(ExternalChange::SignedOut);
    }

    let current = onboarding::read_default_credentials(home, phase)?;
    let Some(facts) = auth_file::read(&current) else {
        return Ok(ExternalChange::NotUnderstood);
    };
    let Some(fingerprint) = facts.fingerprint else {
        return Ok(ExternalChange::NotUnderstood);
    };

    let Some(active) = active else {
        return Ok(ExternalChange::ExternalLogin {
            fingerprint,
            auth_mode: facts.auth_mode,
        });
    };
    if fingerprint != active.fingerprint {
        return Ok(ExternalChange::ExternalLogin {
            fingerprint,
            auth_mode: facts.auth_mode,
        });
    }

    // A snapshot that cannot be read is a reason to write one, not to stop: the account is
    // verified and its credentials are right here. If the credential store is genuinely
    // unavailable, the write below says so rather than this read swallowing it.
    if let Ok(stored) = store.load(active.credentials)
        && stored.expose() == current.expose()
    {
        return Ok(ExternalChange::Unchanged);
    }

    // Held across the store call only, so a switch reading the snapshot waits rather than
    // seeing one that is about to be replaced.
    let _guard = lock.acquire();
    store.store(active.credentials, &current)?;
    Ok(ExternalChange::SnapshotUpdated)
}

/// The result of the last synchronisation before the app exits.
///
/// Deliberately not a `Result`: a failed final synchronisation must not stop the user quitting,
/// and a type with no error variant to propagate cannot be turned into a reason to stay open by
/// a later `?`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitSync {
    Done(ExternalChange),
    /// It failed. The code is kept so the failure can be reported honestly next time the app
    /// starts, rather than being discarded.
    Failed(ErrorCode),
}

/// Runs [`synchronise`] one last time, absorbing failure.
pub fn synchronise_before_exit(
    lock: &CredentialLock,
    store: &dyn SecretStore,
    active: Option<ActiveAccount<'_>>,
    home: &Path,
    phase: Phase,
) -> ExitSync {
    match synchronise(lock, store, active, home, phase) {
        Ok(change) => ExitSync::Done(change),
        Err(error) => ExitSync::Failed(error.code()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::fingerprint;
    use crate::codex_home::IsolatedHome;
    use crate::credentials::{MemorySecretStore, Secret};

    const ACCOUNT_ID: &str = "8f14e45f-ceea-467a-9f3a-1c2d3e4f5a6b";
    const OTHER_ACCOUNT_ID: &str = "1a2b3c4d-0000-4000-8000-abcdefabcdef";
    const PHASE: Phase = Phase::Storage;

    fn auth_json(account_id: &str, refresh_token: &str) -> Vec<u8> {
        format!(
            r#"{{"auth_mode":"chatgpt","tokens":{{"access_token":"eyJhbGciOiJIUzI1NiJ9.aaaa","account_id":"{account_id}","refresh_token":"{refresh_token}"}}}}"#
        )
        .into_bytes()
    }

    struct Fixture {
        home: IsolatedHome,
        store: MemorySecretStore,
        reference: CredentialRef,
        lock: CredentialLock,
        fingerprint: String,
    }

    impl Fixture {
        /// A home signed in as `ACCOUNT_ID`, with a snapshot holding the same bytes.
        fn new() -> Self {
            let home = IsolatedHome::create(PHASE).expect("scratch home");
            let store = MemorySecretStore::new();
            let reference = CredentialRef::new("acct-1").expect("valid reference");
            let snapshot = auth_json(ACCOUNT_ID, "rt-original");
            std::fs::write(home.path().join("auth.json"), &snapshot).expect("written");
            store
                .store(&reference, &Secret::new(snapshot))
                .expect("stored");

            Self {
                home,
                store,
                reference,
                lock: CredentialLock::new(),
                fingerprint: fingerprint::from_account_id(ACCOUNT_ID),
            }
        }

        fn active(&self) -> Option<ActiveAccount<'_>> {
            Some(ActiveAccount {
                credentials: &self.reference,
                fingerprint: &self.fingerprint,
            })
        }

        fn run(&self, active: Option<ActiveAccount<'_>>) -> Result<ExternalChange> {
            synchronise(&self.lock, &self.store, active, self.home.path(), PHASE)
        }

        fn snapshot(&self) -> Vec<u8> {
            self.store
                .load(&self.reference)
                .expect("readable")
                .expose()
                .to_vec()
        }

        fn write_auth(&self, contents: &[u8]) {
            std::fs::write(self.home.path().join("auth.json"), contents).expect("written");
        }
    }

    #[test]
    fn credentials_identical_to_the_snapshot_are_left_alone() {
        let fixture = Fixture::new();

        assert_eq!(
            fixture.run(fixture.active()).expect("succeeds"),
            ExternalChange::Unchanged
        );
    }

    #[test]
    fn a_refresh_of_the_same_account_updates_the_snapshot() {
        let fixture = Fixture::new();
        let refreshed = auth_json(ACCOUNT_ID, "rt-rotated");
        fixture.write_auth(&refreshed);

        assert_eq!(
            fixture.run(fixture.active()).expect("succeeds"),
            ExternalChange::SnapshotUpdated
        );
        assert_eq!(fixture.snapshot(), refreshed);
    }

    #[test]
    fn a_different_account_is_reported_and_never_stored() {
        // This is the case that must not be silently applied.
        let fixture = Fixture::new();
        let before = fixture.snapshot();
        fixture.write_auth(&auth_json(OTHER_ACCOUNT_ID, "rt-stranger"));

        let change = fixture.run(fixture.active()).expect("succeeds");

        assert!(matches!(change, ExternalChange::ExternalLogin { .. }));
        assert_eq!(
            fixture.snapshot(),
            before,
            "another account's credentials must not replace this account's snapshot"
        );
    }

    #[test]
    fn a_sign_in_with_no_account_tracked_yet_is_an_external_login() {
        let fixture = Fixture::new();

        let change = fixture.run(None).expect("succeeds");

        assert_eq!(
            change,
            ExternalChange::ExternalLogin {
                fingerprint: fingerprint::from_account_id(ACCOUNT_ID),
                auth_mode: Some("chatgpt".to_owned()),
            }
        );
    }

    #[test]
    fn a_half_written_file_concludes_nothing_and_stores_nothing() {
        // The watcher is what avoids this, but the parse is the second guard.
        let fixture = Fixture::new();
        let before = fixture.snapshot();
        fixture.write_auth(br#"{"auth_mode":"chatgpt","tok"#);

        assert_eq!(
            fixture.run(fixture.active()).expect("succeeds"),
            ExternalChange::NotUnderstood
        );
        assert_eq!(fixture.snapshot(), before);
    }

    #[test]
    fn a_file_without_an_identity_to_compare_is_not_stored_over_the_snapshot() {
        let fixture = Fixture::new();
        let before = fixture.snapshot();
        fixture.write_auth(br#"{"OPENAI_API_KEY":"sk-x","auth_mode":"apikey"}"#);

        assert_eq!(
            fixture.run(fixture.active()).expect("succeeds"),
            ExternalChange::NotUnderstood
        );
        assert_eq!(fixture.snapshot(), before);
    }

    #[test]
    fn a_removed_authentication_file_reads_as_signed_out_rather_than_as_a_failure() {
        let fixture = Fixture::new();
        std::fs::remove_file(fixture.home.path().join("auth.json")).expect("removed");

        assert_eq!(
            fixture.run(fixture.active()).expect("succeeds"),
            ExternalChange::SignedOut
        );
        assert!(
            !fixture.snapshot().is_empty(),
            "signing out of Codex must not delete the credentials Toglet holds"
        );
    }

    #[test]
    fn what_is_reported_carries_no_token_material() {
        let fixture = Fixture::new();
        fixture.write_auth(&auth_json(OTHER_ACCOUNT_ID, "rt-stranger"));

        let rendered = format!("{:?}", fixture.run(fixture.active()).expect("succeeds"));

        for secret in ["rt-stranger", "eyJ", OTHER_ACCOUNT_ID, "access_token"] {
            assert!(!rendered.contains(secret), "the outcome carried {secret}");
        }
    }

    #[test]
    fn a_failed_final_synchronisation_is_a_value_rather_than_an_error() {
        // Whatever happens here, the user gets to quit. A directory where the file should be
        // makes the read fail without needing an unavailable credential store.
        let fixture = Fixture::new();
        std::fs::remove_file(fixture.home.path().join("auth.json")).expect("removed");
        std::fs::create_dir(fixture.home.path().join("auth.json")).expect("created");

        let outcome = synchronise_before_exit(
            &fixture.lock,
            &fixture.store,
            fixture.active(),
            fixture.home.path(),
            PHASE,
        );

        assert!(
            matches!(outcome, ExitSync::Failed(_)),
            "the failure must be reported, not thrown: {outcome:?}"
        );
    }

    #[test]
    fn a_successful_final_synchronisation_reports_what_it_did() {
        let fixture = Fixture::new();
        fixture.write_auth(&auth_json(ACCOUNT_ID, "rt-rotated"));

        let outcome = synchronise_before_exit(
            &fixture.lock,
            &fixture.store,
            fixture.active(),
            fixture.home.path(),
            PHASE,
        );

        assert_eq!(outcome, ExitSync::Done(ExternalChange::SnapshotUpdated));
    }
}
