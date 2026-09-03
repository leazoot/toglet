//! Replacing the default authentication, and putting it back when that goes wrong.
//!
//! This is the only place in the application that writes the default `auth.json`, and it can
//! only be entered with a [`PreflightPassed`] in hand. The sequence is fixed:
//!
//! ```text
//! read who is signed in now → copy it → journal → stage → replace → journal → verify
//!   ├─ the home is signed in as the target → delete journal and backup, issue the token
//!   └─ anything else                       → restore the copy, verify that, report honestly
//! ```
//!
//! `activeAccountId` is not written here. What is returned is a [`SwitchVerified`] token, and
//! the only way to obtain one is to reach the end of this function with the identity
//! confirmed.

use std::path::{Path, PathBuf};

use super::journal::{SwitchJournal, SwitchPhase};
use super::preflight::{ClientVerdict, PreflightPassed};
use super::state::{StepObserver, SwitchProgress, SwitchStep};
use super::verify;
use crate::accounts::AccountIdentity;
use crate::app_server::CodexBinary;
use crate::codex_home::{atomic_write, stage};
use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};
use crate::process::RunningClient;
use crate::storage::SwitchVerified;

/// The points a switch can be made to fail at.
///
/// Failure injection is a constructor argument rather than a build flag or an environment
/// variable: the release binary contains [`NoFaults`], whose method is empty and
/// inlines away, and no code path reads configuration to decide whether to misbehave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchStage {
    /// Before the replacement is written to its temporary file.
    Write,
    /// After it is written, before it replaces the target.
    Replace,
    /// After the replacement is in place, before it is verified.
    Verify,
}

/// Decides whether a stage fails.
pub trait Faults {
    fn before(&self, stage: SwitchStage) -> Result<()>;
}

/// The production implementation: nothing ever fails on purpose.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoFaults;

impl Faults for NoFaults {
    fn before(&self, _stage: SwitchStage) -> Result<()> {
        Ok(())
    }
}

/// A switch that replaced the authentication and confirmed it.
///
/// Carries no credential material - the credentials went into the default home and are not
/// kept here - so a derived `Debug` is safe.
#[derive(Debug)]
pub struct SwitchSucceeded {
    /// The proof that lets `activeAccountId` be written, and nothing else produces it.
    pub verified: SwitchVerified,
    pub progress: SwitchProgress,
    /// What was running when the checks passed, so the caller can offer to reopen it.
    pub clients: Vec<RunningClient>,
    pub verdict: ClientVerdict,
}

/// What happened to the previous authentication after a switch failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RollbackReport {
    /// The switch stopped before anything was replaced.
    NotNeeded,
    /// The previous authentication is back, and a fresh app server confirmed it.
    Restored,
    /// The previous authentication is back, but the confirmation could not be obtained. Said
    /// plainly rather than rounded up to `Restored`.
    RestoredUnverified,
    /// The rollback itself failed.
    Failed {
        /// Where the copy is, so the user can put it back by hand.
        ///
        /// The one absolute path Toglet shows a person. It is carried here, outside the
        /// error's detail, precisely so it is shown and **not** written to a log.
        backup: PathBuf,
    },
}

/// A switch that did not complete.
#[derive(Debug)]
pub struct SwitchFailed {
    pub error: TogletError,
    /// How far it got before stopping - never further than the work that actually happened.
    pub progress: SwitchProgress,
    pub rollback: RollbackReport,
}

/// Everything the replacement needs.
pub struct Switch<'a> {
    pub binary: &'a CodexBinary,
    /// The user's real Codex home.
    pub default_home: &'a Path,
    /// Where the journal lives - the application's own data directory.
    pub journal_directory: &'a Path,
    pub faults: &'a dyn Faults,
    /// Told about each step as it finishes, so the panel can show real progress rather than an
    /// animation. See [`StepObserver`].
    pub observer: &'a dyn StepObserver,
}

impl Switch<'_> {
    /// Runs the replacement.
    ///
    /// `operation_id` and `started_at` are supplied by the caller rather than read from a clock
    /// here, so a journal written during a test is reproducible.
    pub fn run(
        &self,
        passed: PreflightPassed<'_>,
        from_account_id: Option<&str>,
        to_account_id: &str,
        operation_id: &str,
        started_at: &str,
    ) -> std::result::Result<SwitchSucceeded, SwitchFailed> {
        let mut progress = SwitchProgress::new();
        // The pre-checks are the first step, and they are already done - that is what holding a
        // `PreflightPassed` means.
        if let Err(error) = progress.complete(SwitchStep::Check, Phase::Precheck) {
            return Err(failed(error, progress, RollbackReport::NotNeeded));
        }
        self.observer.completed(SwitchStep::Check);

        let target_identity = passed.target.identity().clone();
        let auth = self.default_home.join("auth.json");

        // Read before anything is touched: this is what a rollback has to restore *to*, and
        // asking afterwards would be asking about the state the switch created.
        let previous_identity =
            match verify::read_default_identity(self.binary, self.default_home, Phase::Precheck) {
                Ok(identity) => identity,
                Err(error) => return Err(failed(error, progress, RollbackReport::NotNeeded)),
            };

        let backup = match back_up(&auth, operation_id) {
            Ok(backup) => backup,
            Err(error) => return Err(failed(error, progress, RollbackReport::NotNeeded)),
        };

        let mut journal = match SwitchJournal::begin(
            self.journal_directory,
            operation_id,
            from_account_id,
            Some(to_account_id),
            backup.clone(),
            started_at,
        ) {
            Ok(journal) => journal,
            Err(error) => {
                drop(std::fs::remove_file(&backup));
                return Err(failed(error, progress, RollbackReport::NotNeeded));
            }
        };

        // From here on the previous authentication may be gone, so every failure rolls back.
        let replaced = self.replace(&auth, passed.target.secret().expose());
        if let Err(error) = replaced {
            return Err(self.roll_back(error, progress, journal, previous_identity.as_ref()));
        }

        if let Err(error) = journal.advance(self.journal_directory, SwitchPhase::Replaced) {
            return Err(self.roll_back(error, progress, journal, previous_identity.as_ref()));
        }
        if let Err(error) = progress.complete(SwitchStep::Switch, Phase::Write) {
            return Err(self.roll_back(error, progress, journal, previous_identity.as_ref()));
        }
        self.observer.completed(SwitchStep::Switch);

        if let Err(error) = self.faults.before(SwitchStage::Verify) {
            return Err(self.roll_back(error, progress, journal, previous_identity.as_ref()));
        }

        let actual =
            match verify::read_default_identity(self.binary, self.default_home, Phase::Verify) {
                Ok(identity) => identity,
                Err(error) => {
                    return Err(self.roll_back(
                        error,
                        progress,
                        journal,
                        previous_identity.as_ref(),
                    ));
                }
            };
        if !verify::is_target(actual.as_ref(), &target_identity) {
            let error = verify::mismatch(Phase::Verify);
            return Err(self.roll_back(error, progress, journal, previous_identity.as_ref()));
        }

        if let Err(error) = progress.complete(SwitchStep::Verify, Phase::Verify) {
            return Err(self.roll_back(error, progress, journal, previous_identity.as_ref()));
        }
        self.observer.completed(SwitchStep::Verify);

        // Verified, so the journal and the copy have done their job.
        if let Err(error) = journal.finish(self.journal_directory) {
            return Err(failed(error, progress, RollbackReport::NotNeeded));
        }
        if let Err(error) = progress.complete(SwitchStep::Ready, Phase::Verify) {
            return Err(failed(error, progress, RollbackReport::NotNeeded));
        }
        self.observer.completed(SwitchStep::Ready);

        Ok(SwitchSucceeded {
            verified: SwitchVerified::issue(),
            progress,
            clients: passed.clients,
            verdict: passed.verdict,
        })
    }

    fn replace(&self, auth: &Path, contents: &[u8]) -> Result<()> {
        self.faults.before(SwitchStage::Write)?;
        let staged = stage(auth, contents).map_err(|error| {
            home_error(
                Phase::Write,
                ErrorCode::CodexHomeUnwritable,
                &error.to_string(),
            )
        })?;

        self.faults.before(SwitchStage::Replace)?;
        staged.commit().map_err(|error| {
            home_error(
                Phase::Write,
                ErrorCode::CodexHomeUnwritable,
                &error.to_string(),
            )
        })
    }

    /// Puts the copy back and checks that it took.
    fn roll_back(
        &self,
        error: TogletError,
        progress: SwitchProgress,
        journal: SwitchJournal,
        previous: Option<&AccountIdentity>,
    ) -> SwitchFailed {
        let auth = self.default_home.join("auth.json");
        let backup = journal.backup_path.clone();

        let restored = match std::fs::read(&backup) {
            // An empty backup means nothing was signed in before, so restoring means removing
            // the file rather than writing zero bytes into it.
            Ok(contents) if contents.is_empty() => remove_auth(&auth),
            Ok(contents) => atomic_write(&auth, &contents),
            Err(error) => Err(error),
        };
        if restored.is_err() {
            return failed(error, progress, RollbackReport::Failed { backup });
        }

        let confirmed =
            verify::read_default_identity(self.binary, self.default_home, Phase::Rollback);
        let report = match confirmed {
            Ok(actual) if verify::is_same(actual.as_ref(), previous) => {
                // The home is back to what it was, so the journal and the copy are no longer
                // needed. A failure to clean up must not turn a successful rollback into a
                // reported failure - the state on disk is already correct.
                drop(journal.finish(self.journal_directory));
                RollbackReport::Restored
            }
            Ok(_) => RollbackReport::Failed { backup },
            Err(_) => RollbackReport::RestoredUnverified,
        };

        failed(error, progress, report)
    }
}

/// Copies the current authentication beside itself. Shared with the sign-out.
///
/// A home with no `auth.json` still gets a backup file - an empty one would be wrong, so
/// the absence is recorded by there being nothing to copy, and the rollback restores that
/// same absence by removing the file.
pub(super) fn back_up(auth: &Path, operation_id: &str) -> Result<PathBuf> {
    let backup = auth.with_file_name(format!("auth.json.toglet-switch-{operation_id}"));
    match std::fs::read(auth) {
        Ok(contents) => {
            atomic_write(&backup, &contents).map_err(|error| {
                home_error(
                    Phase::Backup,
                    ErrorCode::CodexHomeUnwritable,
                    &error.to_string(),
                )
            })?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // Nothing was signed in. The marker file records that, so a rollback knows to
            // leave the home empty rather than to look for content that never existed.
            atomic_write(&backup, b"").map_err(|error| {
                home_error(
                    Phase::Backup,
                    ErrorCode::CodexHomeUnwritable,
                    &error.to_string(),
                )
            })?;
        }
        Err(error) => {
            return Err(home_error(
                Phase::Backup,
                ErrorCode::AuthFileConflict,
                &error.to_string(),
            ));
        }
    }
    Ok(backup)
}

pub(super) fn remove_auth(auth: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(auth) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn failed(error: TogletError, progress: SwitchProgress, rollback: RollbackReport) -> SwitchFailed {
    let error = match rollback {
        // The original failure matters less than the fact that the user now has to put
        // their own credentials back, so that is what the error says.
        RollbackReport::Failed { .. } => TogletError::new(
            ErrorCode::RollbackFailed,
            error.phase(),
            false,
            UserAction::RestoreFromBackup,
        ),
        _ => error,
    };
    SwitchFailed {
        error,
        progress,
        rollback,
    }
}

pub(super) fn home_error(phase: Phase, code: ErrorCode, detail: &str) -> TogletError {
    TogletError::new(code, phase, true, UserAction::Retry).with_detail(detail)
}
