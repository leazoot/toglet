//! Putting Codex into file-credential mode, and reporting honestly when it cannot be done.

use std::path::PathBuf;

use super::backup;
use crate::app_server::{AppServerSession, CREDENTIAL_STORE_FILE};
use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialStoreOutcome {
    /// Nothing was written and no backup taken, so repeats do not pile up copies.
    AlreadyEnabled,
    Enabled(EnabledRecord),
}

/// Everything needed to put the configuration back the way it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnabledRecord {
    /// `None` means the key was absent, so restoring removes it rather than writing "".
    pub previous_value: Option<String>,
    /// `None` when there was no configuration file. An absolute path: never sent to the frontend
    /// or logged.
    pub backup: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreOutcome {
    /// Already at its pre-Toglet value; nothing was written and no backup taken.
    AlreadyRestored,
    Restored {
        /// `None` when there was no configuration file.
        backup: Option<PathBuf>,
    },
}

/// Ensures Codex stores credentials in `auth.json`.
///
/// Order matters: refuse a throwaway home (the write would change nothing); stop on
/// organisation-enforced config before copying anything; do nothing if already set; refuse a
/// layer the user does not own; back up, write with `expectedVersion`, then read the value back
/// so a write that silently did not take effect is reported.
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

    // Nothing above has modified anything, so the backup is taken only now.
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
/// `previous: None` means the key did not exist, so restoring removes it. Only the exact value
/// Toglet wrote is undone; a setting someone changed since is left alone. `expectedVersion` is
/// read fresh because a remembered one goes stale on any unrelated edit.
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
