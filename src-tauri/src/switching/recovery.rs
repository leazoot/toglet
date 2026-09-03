//! Finishing what an interrupted switch started.
//!
//! A journal on disk at start-up means the process died between replacing the authentication
//! and confirming it. Two states are possible and they need different answers:
//!
//! * the copy was taken but the replacement may not have completed → put the copy back;
//! * the replacement landed but was never verified → verify it now, and only then finish.
//!
//! The second case is the one that must not be guessed. `activeAccountId` still may not be
//! written on anything weaker than a fresh `account/read` agreeing with the target, so
//! recovery either produces the same [`SwitchVerified`] token the switch itself would have, or
//! it rolls back.

use std::path::{Path, PathBuf};

use super::journal::{RecoveryPlan, SwitchJournal};
use super::verify;
use crate::accounts::AccountIdentity;
use crate::app_server::CodexBinary;
use crate::codex_home::atomic_write;
use crate::diagnostics::{Phase, Result};
use crate::storage::SwitchVerified;

/// What recovery did about a journal found at start-up.
#[derive(Debug)]
pub enum RecoveryOutcome {
    /// No switch was in flight.
    NothingToDo,
    /// The interrupted switch was undone and the previous authentication is back.
    RolledBack,
    /// The replacement had landed and a fresh reading confirms it, so the switch is finished
    /// now rather than discarded.
    Completed {
        verified: SwitchVerified,
        to_account_id: String,
    },
    /// The copy could not be put back.
    Failed {
        /// Shown to the user so they can restore it by hand; never logged.
        backup: PathBuf,
    },
}

/// Reads the journal, if any, and brings the authentication back to a state the user can trust.
///
/// `expected_target` is the identity the interrupted switch was aiming at. It is passed in
/// rather than derived here because the metadata document stores only masked addresses - the
/// full one comes from verifying the target's stored credentials, which is the caller's job and
/// already exists as `accounts::onboarding::verify`. Passing `None` means the target could not
/// be established, and recovery then rolls back rather than completing on an assumption.
pub fn recover(
    binary: &CodexBinary,
    default_home: &Path,
    journal_directory: &Path,
    expected_target: Option<&AccountIdentity>,
) -> Result<RecoveryOutcome> {
    let Some(journal) = SwitchJournal::load(journal_directory)? else {
        return Ok(RecoveryOutcome::NothingToDo);
    };

    // A sign-out journal names no target and never advances past `BackedUp`, so it never gets
    // here: an interrupted sign-out is restored, never completed on a reading.
    if RecoveryPlan::for_phase(journal.phase) == RecoveryPlan::ReVerify
        && let Some(expected) = expected_target
        && let Some(to_account_id) = journal.to_account_id.clone()
    {
        let actual = verify::read_default_identity(binary, default_home, Phase::Verify)?;
        if verify::is_target(actual.as_ref(), expected) {
            journal.finish(journal_directory)?;
            return Ok(RecoveryOutcome::Completed {
                verified: SwitchVerified::issue(),
                to_account_id,
            });
        }
    }

    let auth = default_home.join("auth.json");
    let backup = journal.backup_path.clone();
    let restored = match std::fs::read(&backup) {
        // An empty copy records that nobody was signed in, so restoring means removing the file.
        Ok(contents) if contents.is_empty() => remove_if_present(&auth),
        Ok(contents) => atomic_write(&auth, &contents),
        Err(error) => Err(error),
    };
    if restored.is_err() {
        return Ok(RecoveryOutcome::Failed { backup });
    }

    journal.finish(journal_directory)?;
    Ok(RecoveryOutcome::RolledBack)
}

fn remove_if_present(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}
