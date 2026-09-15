//! Signing Codex out on explicit request, under the same discipline as a switch.
//! Removes the default `auth.json` (as `codex logout` does), verifies nobody is signed in and
//! rolls back otherwise; the resulting `SwitchVerified` clears `activeAccountId`.

use std::path::Path;

use super::journal::SwitchJournal;
use super::preflight::{self, ClientVerdict, PreflightFailure, SwitchGuard, SwitchLock};
use super::swap::{RollbackReport, back_up, home_error, remove_auth};
use super::verify;
use crate::accounts::AccountIdentity;
use crate::accounts::external_change::ActiveAccount;
use crate::app_server::CodexBinary;
use crate::codex_home::atomic_write;
use crate::credentials::{CredentialLock, SecretStore};
use crate::diagnostics::{ErrorCode, Phase, TogletError, UserAction};
use crate::process::{ClientPresence, ClientProbe, RunningClient};
use crate::storage::SwitchVerified;

/// Everything a sign-out needs. The same things a switch needs, minus a target.
pub struct SignOut<'a> {
    pub lock: &'a SwitchLock,
    pub credential_lock: &'a CredentialLock,
    pub store: &'a dyn SecretStore,
    pub probe: &'a dyn ClientProbe,
    pub binary: &'a CodexBinary,
    /// The user's real Codex home - the one whose `auth.json` goes.
    pub default_home: &'a Path,
    /// Where the journal lives - the application's own data directory.
    pub journal_directory: &'a Path,
    /// Process ids Toglet started itself, which must not block Toglet's own sign-out.
    pub own_processes: &'a [u32],
}

/// Proof the checks passed. Built only by [`SignOut::prepare`], consumed only by
/// [`SignOut::run`].
#[derive(Debug)]
pub struct SignOutPassed<'a> {
    /// Held until the sign-out is over. Dropping this releases it.
    guard: SwitchGuard<'a>,
    /// What was running when the checks passed: the caller closes these before the removal,
    /// for the same reason a switch does.
    pub clients: Vec<RunningClient>,
    pub verdict: ClientVerdict,
}

/// A sign-out that removed the authentication and confirmed the home is signed out.
#[derive(Debug)]
pub struct SignedOut {
    /// The proof that lets `activeAccountId` be cleared, and nothing else produces it.
    pub verified: SwitchVerified,
    pub verdict: ClientVerdict,
}

/// A sign-out that did not complete, and what happened to the authentication afterwards.
#[derive(Debug)]
pub struct SignOutFailed {
    pub error: TogletError,
    pub rollback: RollbackReport,
}

impl<'a> SignOut<'a> {
    /// The pre-checks. A sign-in made outside Toglet stops this, as it stops a switch.
    pub fn prepare(
        &self,
        active: Option<ActiveAccount<'_>>,
    ) -> std::result::Result<SignOutPassed<'a>, PreflightFailure> {
        let guard = preflight::take_lock(self.lock)?;
        let (presence, verdict) = preflight::check_clients(self.probe, self.own_processes)?;
        preflight::check_writable(self.default_home)?;
        preflight::snapshot_current(self.credential_lock, self.store, active, self.default_home)?;

        let clients = match presence {
            ClientPresence::Known(clients) => clients,
            ClientPresence::Unknown => Vec::new(),
        };
        Ok(SignOutPassed {
            guard,
            clients,
            verdict,
        })
    }

    /// Removes the default authentication and confirms the home is signed out.
    ///
    /// The journal never advances past `BackedUp`, so a crash is undone at start-up by restoring
    /// the copy; recovery has no target to complete a sign-out against.
    pub fn run(
        &self,
        passed: SignOutPassed<'_>,
        from_account_id: &str,
        operation_id: &str,
        started_at: &str,
    ) -> std::result::Result<SignedOut, SignOutFailed> {
        // The lock is held, by name, until this function returns.
        let SignOutPassed {
            guard: _guard,
            verdict,
            clients: _,
        } = passed;
        let auth = self.default_home.join("auth.json");

        let previous =
            match verify::read_default_identity(self.binary, self.default_home, Phase::Precheck) {
                Ok(identity) => identity,
                Err(error) => return Err(failed(error, RollbackReport::NotNeeded)),
            };

        let backup = match back_up(&auth, operation_id) {
            Ok(backup) => backup,
            Err(error) => return Err(failed(error, RollbackReport::NotNeeded)),
        };
        let journal = match SwitchJournal::begin(
            self.journal_directory,
            operation_id,
            Some(from_account_id),
            None,
            backup.clone(),
            started_at,
        ) {
            Ok(journal) => journal,
            Err(error) => {
                drop(std::fs::remove_file(&backup));
                return Err(failed(error, RollbackReport::NotNeeded));
            }
        };

        // From here on the authentication may be gone, so every failure rolls back.
        if let Err(error) = remove_auth(&auth) {
            let error = home_error(
                Phase::Write,
                ErrorCode::CodexHomeUnwritable,
                &error.to_string(),
            );
            return Err(self.roll_back(error, journal, previous.as_ref()));
        }

        let actual =
            match verify::read_default_identity(self.binary, self.default_home, Phase::Verify) {
                Ok(identity) => identity,
                Err(error) => return Err(self.roll_back(error, journal, previous.as_ref())),
            };
        if actual.is_some() {
            // The file is gone but the server still names somebody: not a confirmed sign-out.
            let error = TogletError::new(
                ErrorCode::SwitchVerificationMismatch,
                Phase::Verify,
                false,
                UserAction::RestoreFromBackup,
            )
            .with_detail("the default home still reports a signed-in account");
            return Err(self.roll_back(error, journal, previous.as_ref()));
        }

        if let Err(error) = journal.finish(self.journal_directory) {
            return Err(failed(error, RollbackReport::NotNeeded));
        }

        Ok(SignedOut {
            verified: SwitchVerified::issue(),
            verdict,
        })
    }

    /// Puts the copy back and checks that the home is signed in as it was.
    fn roll_back(
        &self,
        error: TogletError,
        journal: SwitchJournal,
        previous: Option<&AccountIdentity>,
    ) -> SignOutFailed {
        let auth = self.default_home.join("auth.json");
        let backup = journal.backup_path.clone();

        let restored = match std::fs::read(&backup) {
            // An empty copy records that nobody was signed in, so restoring means removing.
            Ok(contents) if contents.is_empty() => remove_auth(&auth),
            Ok(contents) => atomic_write(&auth, &contents),
            Err(error) => Err(error),
        };
        if restored.is_err() {
            return failed(error, RollbackReport::Failed { backup });
        }

        let confirmed =
            verify::read_default_identity(self.binary, self.default_home, Phase::Rollback);
        let report = match confirmed {
            Ok(actual) if verify::is_same(actual.as_ref(), previous) => {
                // Back to what it was; a failure to tidy up must not be reported as a failed
                // rollback when the state on disk is already right.
                drop(journal.finish(self.journal_directory));
                RollbackReport::Restored
            }
            Ok(_) => RollbackReport::Failed { backup },
            Err(_) => RollbackReport::RestoredUnverified,
        };
        failed(error, report)
    }
}

fn failed(error: TogletError, rollback: RollbackReport) -> SignOutFailed {
    let error = match rollback {
        // What the user has to do now matters more than what went wrong first.
        RollbackReport::Failed { .. } => TogletError::new(
            ErrorCode::RollbackFailed,
            error.phase(),
            false,
            UserAction::RestoreFromBackup,
        ),
        _ => error,
    };
    SignOutFailed { error, rollback }
}
