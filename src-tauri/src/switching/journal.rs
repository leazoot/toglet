//! The record of a switch in flight, so a crash in the middle can be undone.
//!
//! **No credential material, at the type level**: the fields are identifiers,
//! a phase and the backup's location. A test opens the written file and asserts the same thing
//! about its bytes, because a struct that cannot hold a token today can grow a field tomorrow.
//!
//! Only one journal exists at a time. A switch that finished and verified deletes it along with
//! the backup; anything found on the next start-up means a switch did not finish.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::codex_home::atomic_write;
use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};

/// The journal's file name inside the application data directory.
pub const JOURNAL_FILE: &str = "switch-journal.json";

/// How far the switch had got when the journal was last written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SwitchPhase {
    /// The previous authentication has been copied. Nothing has been replaced yet.
    BackedUp,
    /// The replacement is in place but has not been verified.
    Replaced,
}

/// A switch that has started and not yet finished.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchJournal {
    pub operation_id: String,
    pub from_account_id: Option<String>,
    /// `None` for a sign-out: the authentication is being removed, not replaced, and there is
    /// no account for recovery to confirm against - it restores the copy instead.
    pub to_account_id: Option<String>,
    pub phase: SwitchPhase,
    /// Where the previous authentication was copied to.
    ///
    /// An absolute path, which is allowed here and nowhere else: this is a runtime file in the
    /// application's own directory, not the metadata document, and recovery has nothing to
    /// restore from without it. It never reaches a log or the frontend.
    pub backup_path: PathBuf,
    pub started_at: String,
}

impl SwitchJournal {
    /// Writes the journal that says a switch has begun.
    pub fn begin(
        directory: &Path,
        operation_id: &str,
        from_account_id: Option<&str>,
        to_account_id: Option<&str>,
        backup_path: PathBuf,
        started_at: &str,
    ) -> Result<Self> {
        let journal = Self {
            operation_id: operation_id.to_owned(),
            from_account_id: from_account_id.map(str::to_owned),
            to_account_id: to_account_id.map(str::to_owned),
            phase: SwitchPhase::BackedUp,
            backup_path,
            started_at: started_at.to_owned(),
        };
        journal.save(directory)?;
        Ok(journal)
    }

    /// Records that the switch reached a later phase.
    pub fn advance(&mut self, directory: &Path, phase: SwitchPhase) -> Result<()> {
        self.phase = phase;
        self.save(directory)
    }

    /// Removes the journal and the backup it points at.
    ///
    /// Called only after verification succeeded. Deleting either one earlier would throw away
    /// the only thing a rollback has to work with.
    pub fn finish(self, directory: &Path) -> Result<()> {
        // Best effort, and in this order: without the journal, a leftover backup is inert, but
        // a leftover journal pointing at a deleted backup would send recovery looking for a
        // file that is not there.
        remove(&directory.join(JOURNAL_FILE), Phase::Storage)?;
        drop(std::fs::remove_file(&self.backup_path));
        Ok(())
    }

    /// Reads the journal, if a switch was interrupted.
    ///
    /// A file that cannot be parsed is reported as a failure rather than as "no switch was in
    /// flight": the difference decides whether the previous authentication gets restored.
    pub fn load(directory: &Path) -> Result<Option<Self>> {
        let path = directory.join(JOURNAL_FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(storage_error(
                    ErrorCode::Internal,
                    UserAction::None,
                    &error.to_string(),
                ));
            }
        };

        serde_json::from_str(&text).map(Some).map_err(|error| {
            storage_error(
                ErrorCode::Internal,
                UserAction::RestoreFromBackup,
                &error.to_string(),
            )
        })
    }

    fn save(&self, directory: &Path) -> Result<()> {
        let body = serde_json::to_vec(self).map_err(|error| {
            storage_error(ErrorCode::Internal, UserAction::None, &error.to_string())
        })?;
        atomic_write(&directory.join(JOURNAL_FILE), &body).map_err(|error| {
            storage_error(
                ErrorCode::CodexHomeUnwritable,
                UserAction::FixPermissions,
                &error.to_string(),
            )
        })
    }
}

/// What recovery should do about a journal found at start-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryPlan {
    /// The replacement never happened, or may not have. Put the backup back and check it took.
    RestoreBackup,
    /// The replacement landed but was never verified. Check who the default home is signed in
    /// as now, and either finish or roll back on that evidence.
    ReVerify,
}

impl RecoveryPlan {
    /// Chooses the plan from how far the switch got.
    ///
    /// `BackedUp` restores rather than re-verifying, because the replacement may have been
    /// interrupted half-way through its own sequence; restoring a known-good copy and checking
    /// it is the shorter path to a state the user can trust.
    pub fn for_phase(phase: SwitchPhase) -> Self {
        match phase {
            SwitchPhase::BackedUp => Self::RestoreBackup,
            SwitchPhase::Replaced => Self::ReVerify,
        }
    }
}

fn remove(path: &Path, phase: Phase) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(TogletError::new(
            ErrorCode::CodexHomeUnwritable,
            phase,
            true,
            UserAction::FixPermissions,
        )
        .with_detail(&error.to_string())),
    }
}

fn storage_error(code: ErrorCode, action: UserAction, detail: &str) -> TogletError {
    TogletError::new(code, Phase::Storage, false, action).with_detail(detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_home::IsolatedHome;

    const STARTED_AT: &str = "2026-09-01T00:00:00Z";

    fn scratch() -> IsolatedHome {
        IsolatedHome::create(Phase::Storage).expect("scratch directory")
    }

    fn begin(directory: &Path) -> SwitchJournal {
        SwitchJournal::begin(
            directory,
            "op-1",
            Some("from-account"),
            Some("to-account"),
            directory.join("auth.json.backup"),
            STARTED_AT,
        )
        .expect("the journal is written")
    }

    #[test]
    fn a_journal_survives_a_restart() {
        let home = scratch();
        let written = begin(home.path());

        let loaded = SwitchJournal::load(home.path())
            .expect("readable")
            .expect("a switch was in flight");

        assert_eq!(loaded, written);
        assert_eq!(loaded.phase, SwitchPhase::BackedUp);
    }

    #[test]
    fn no_journal_means_no_switch_was_in_flight() {
        let home = scratch();

        assert_eq!(SwitchJournal::load(home.path()).expect("readable"), None);
    }

    #[test]
    fn a_journal_that_cannot_be_parsed_is_a_failure_rather_than_an_all_clear() {
        let home = scratch();
        std::fs::write(home.path().join(JOURNAL_FILE), b"{ not json").expect("written");

        let error = SwitchJournal::load(home.path())
            .expect_err("a damaged journal must not be read as 'nothing happened'");

        assert_eq!(error.action(), UserAction::RestoreFromBackup);
    }

    #[test]
    fn what_lands_on_disk_carries_no_credential_material() {
        // The structural guarantee is that the type has no such field. This checks the bytes,
        // because a future field would pass the type check and fail here.
        let home = scratch();
        begin(home.path());

        let written = std::fs::read_to_string(home.path().join(JOURNAL_FILE)).expect("readable");

        for forbidden in [
            "access_token",
            "refresh_token",
            "id_token",
            "auth_mode",
            "OPENAI_API_KEY",
            "eyJ",
        ] {
            assert!(
                !written.contains(forbidden),
                "the journal carried `{forbidden}`: {written}"
            );
        }
    }

    #[test]
    fn advancing_the_phase_is_visible_after_a_restart() {
        let home = scratch();
        let mut journal = begin(home.path());

        journal
            .advance(home.path(), SwitchPhase::Replaced)
            .expect("the phase is recorded");

        let loaded = SwitchJournal::load(home.path())
            .expect("readable")
            .expect("still in flight");
        assert_eq!(loaded.phase, SwitchPhase::Replaced);
    }

    #[test]
    fn finishing_removes_both_the_journal_and_the_backup() {
        let home = scratch();
        let backup = home.path().join("auth.json.backup");
        std::fs::write(&backup, b"previous").expect("written");
        let journal = SwitchJournal::begin(
            home.path(),
            "op-1",
            None,
            Some("to-account"),
            backup.clone(),
            STARTED_AT,
        )
        .expect("written");

        journal.finish(home.path()).expect("cleanup succeeds");

        assert_eq!(SwitchJournal::load(home.path()).expect("readable"), None);
        assert!(!backup.exists(), "the temporary backup must not be kept");
    }

    #[test]
    fn an_interrupted_backup_phase_restores_rather_than_trusting_what_is_there() {
        assert_eq!(
            RecoveryPlan::for_phase(SwitchPhase::BackedUp),
            RecoveryPlan::RestoreBackup
        );
    }

    #[test]
    fn an_unverified_replacement_is_checked_before_anything_is_concluded() {
        assert_eq!(
            RecoveryPlan::for_phase(SwitchPhase::Replaced),
            RecoveryPlan::ReVerify
        );
    }
}
