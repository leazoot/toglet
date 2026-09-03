//! Putting Codex into file-credential mode, and reporting honestly when it cannot be done.

use std::path::PathBuf;

use super::backup;
use crate::app_server::{AppServerSession, CREDENTIAL_STORE_FILE};
use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};

/// What managing the setting did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialStoreOutcome {
    /// Codex was already in file mode. Nothing was written and no backup was taken - repeating
    /// the operation must not pile up copies of the user's configuration.
    AlreadyEnabled,
    /// The setting was changed.
    Enabled(EnabledRecord),
}

/// Everything needed to put the configuration back the way it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnabledRecord {
    /// The value before the change. `None` means the key was absent, and restoring means
    /// removing it rather than writing an empty string.
    pub previous_value: Option<String>,
    /// The pre-change copy. `None` when there was no configuration file yet.
    ///
    /// An absolute path: it stays inside the Rust layer and is never returned to the frontend
    /// or written to a log.
    pub backup: Option<PathBuf>,
}

/// What restoring did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreOutcome {
    /// The setting already held its pre-Toglet value. Nothing was written and no backup was
    /// taken, so "stop managing" twice costs nothing.
    AlreadyRestored,
    /// The setting was put back.
    Restored {
        /// The pre-restore copy, `None` when there was no configuration file.
        backup: Option<PathBuf>,
    },
}

/// Ensures Codex stores credentials in `auth.json`.
///
/// The order matters and is the point of this function:
///
/// 1. refuse to run against a throwaway home, where a write would succeed and change nothing;
/// 2. stop on an organisation-enforced configuration **before** reading or copying anything;
/// 3. do nothing at all when the setting is already right, so repeats are free;
/// 4. refuse to write through a layer that is not the user's own;
/// 5. back up, then write with `expectedVersion` so a concurrent edit is refused by the
///    process that owns the file;
/// 6. read the value back, because a write that reported success but did not take effect is
///    exactly the kind of false success this project refuses to report.
///
/// `unix_seconds` names the backup and is injected rather than read here so the name is
/// reproducible in tests.
pub fn enable_file_credential_store(
    session: &mut AppServerSession,
    unix_seconds: i64,
) -> Result<CredentialStoreOutcome> {
    let phase = session.phase();

    if !session.home_is_default() {
        return Err(
            TogletError::new(ErrorCode::Internal, phase, false, UserAction::None)
                .with_detail("the credential store setting was managed against a throwaway home"),
        );
    }

    if session.organisation_requirements_present()? {
        return Err(readonly(
            phase,
            "an organisation-enforced configuration is present",
        ));
    }

    let current = session.read_credential_store_setting()?;
    if current.is_file_mode() {
        return Ok(CredentialStoreOutcome::AlreadyEnabled);
    }
    if current.is_externally_managed() {
        return Err(readonly(
            phase,
            "the credential store setting comes from a layer Toglet does not own",
        ));
    }

    // Nothing above this line has modified anything, which is why the backup belongs here
    // rather than at the top.
    let config = session.home_path().join("config.toml");
    let backup = backup::back_up(&config, unix_seconds, phase)?;

    let written = session
        .write_credential_store_setting(CREDENTIAL_STORE_FILE, current.version.as_deref())?;

    if written.overridden {
        return Err(readonly(
            phase,
            "the value was written but a higher-priority layer overrides it",
        ));
    }

    let confirmed = session.read_credential_store_setting()?;
    if !confirmed.is_file_mode() {
        return Err(
            TogletError::new(ErrorCode::ConfigConflict, phase, true, UserAction::Retry)
                .with_detail("the setting did not take effect after a write reported success"),
        );
    }

    Ok(CredentialStoreOutcome::Enabled(EnabledRecord {
        previous_value: current.value,
        backup,
    }))
}

/// Puts the credential-store setting back to what it was before Toglet changed it.
///
/// `previous` is the value recorded when managing started: `None` means the key did not exist,
/// so restoring removes it. Callers that never changed anything must not call this - there is
/// no "nothing to restore" outcome here, because a configuration Toglet never touched is not
/// this function's business.
///
/// The refusal in the middle is the point of the whole function. Toglet only undoes the exact
/// value it wrote; if the setting now says anything else, somebody changed it after Toglet did,
/// and writing "the old value" over their choice would be wrong.
///
/// The `expectedVersion` presented to the server is read fresh rather than remembered from when
/// managing started. A remembered token would go stale the moment the user edited any unrelated
/// key, and a restore that can never run again is worse than one that runs over a file it has
/// just re-read - the value check above is what protects the setting itself.
pub fn restore_credential_store(
    session: &mut AppServerSession,
    previous: Option<&str>,
    unix_seconds: i64,
) -> Result<RestoreOutcome> {
    let phase = session.phase();

    if !session.home_is_default() {
        return Err(
            TogletError::new(ErrorCode::Internal, phase, false, UserAction::None)
                .with_detail("the credential store setting was restored against a throwaway home"),
        );
    }

    if session.organisation_requirements_present()? {
        return Err(readonly(
            phase,
            "an organisation-enforced configuration is present",
        ));
    }

    let current = session.read_credential_store_setting()?;
    if current.value.as_deref() == previous {
        return Ok(RestoreOutcome::AlreadyRestored);
    }
    if !current.is_file_mode() {
        return Err(TogletError::new(
            ErrorCode::ConfigConflict,
            phase,
            false,
            UserAction::FixConfigManually,
        )
        .with_detail("the setting was changed after Toglet set it, so it was left alone"));
    }
    if current.is_externally_managed() {
        return Err(readonly(
            phase,
            "the credential store setting comes from a layer Toglet does not own",
        ));
    }

    let config = session.home_path().join("config.toml");
    let backup = backup::back_up(&config, unix_seconds, phase)?;

    let version = current.version.as_deref();
    let written = match previous {
        Some(value) => session.write_credential_store_setting(value, version)?,
        None => session.remove_credential_store_setting(version)?,
    };

    if written.overridden {
        return Err(readonly(
            phase,
            "the value was restored but a higher-priority layer overrides it",
        ));
    }

    let confirmed = session.read_credential_store_setting()?;
    if confirmed.value.as_deref() != previous {
        return Err(
            TogletError::new(ErrorCode::ConfigConflict, phase, true, UserAction::Retry)
                .with_detail("the setting did not take effect after a restore reported success"),
        );
    }

    Ok(RestoreOutcome::Restored { backup })
}

fn readonly(phase: Phase, detail: &str) -> TogletError {
    TogletError::new(
        ErrorCode::ConfigLayerReadonly,
        phase,
        false,
        UserAction::FixConfigManually,
    )
    .with_detail(detail)
}
