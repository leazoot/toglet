//! Replacing the default authentication, and restoring it when that fails.
//! Entered only with a `PreflightPassed`. It never writes `activeAccountId`; it returns
//! `SwitchVerified` only once the target identity is confirmed.

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
/// Injected as a constructor argument, never a build flag or env var; release uses `NoFaults`.
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

/// A switch that replaced the authentication and confirmed it. Holds no credentials, so a
/// derived `Debug` is safe.
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
/// Use `as_str` for the interface and logs; `{:?}` would print the backup path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RollbackReport {
    /// The switch stopped before anything was replaced.
    NotNeeded,
    /// The previous authentication is back, and a fresh app server confirmed it.
    Restored,
    /// The previous authentication is back, but the confirmation could not be obtained.
    RestoredUnverified,
    /// The rollback itself failed.
    Failed {
        /// Where the copy is, so the user can restore it by hand. Shown to the user, never logged.
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

impl RollbackReport {
    /// The stable wire form. Carries no path.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotNeeded => "not_needed",
            Self::Restored => "restored",
            Self::RestoredUnverified => "restored_unverified",
            Self::Failed { .. } => "failed",
        }
    }
}

/// Everything the replacement needs.
pub struct Switch<'a> {
    pub binary: &'a CodexBinary,
    /// The user's real Codex home.
    pub default_home: &'a Path,
    /// Where the journal lives - the application's own data directory.
    pub journal_directory: &'a Path,
    pub faults: &'a dyn Faults,
    /// Told as each step really finishes, so the panel shows real progress.
    pub observer: &'a dyn StepObserver,
}

impl Switch<'_> {
    /// Runs the replacement. `operation_id` and `started_at` come from the caller, so no clock
    /// is read here.
    pub fn run(
        &self,
        passed: PreflightPassed<'_>,
        from_account_id: Option<&str>,
        to_account_id: &str,
        operation_id: &str,
        started_at: &str,
    ) -> std::result::Result<SwitchSucceeded, SwitchFailed> {
        let mut progress = SwitchProgress::new();
        // Holding a `PreflightPassed` means the pre-checks are done.
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
                // Back to what it was. A cleanup failure must not turn a successful rollback into a
                // reported failure.
                drop(journal.finish(self.journal_directory));
                RollbackReport::Restored
            }
            Ok(_) => RollbackReport::Failed { backup },
            Err(_) => RollbackReport::RestoredUnverified,
        };

        failed(error, progress, report)
    }
}

/// Copies the current authentication beside itself; shared with the sign-out.
/// With no `auth.json` the backup is empty, and rollback restores that by removing the file.
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
            // Nothing was signed in: an empty backup tells rollback to leave the home empty.
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
        // The user now has to restore their credentials by hand, which matters more than the cause.
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
