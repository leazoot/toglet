//! Timestamped copies of the user's `config.toml`, taken before Toglet changes it.

use std::path::{Path, PathBuf};

use crate::codex_home::atomic_write;
use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};

/// Distinct from other tools' backups (such as `config.toml.agentport-bak-*`), so a cleanup can
/// never delete another program's copy.
const BACKUP_PREFIX: &str = "toglet-backup";

/// Bounded so a directory that somehow rejects every name fails instead of looping.
const NAME_ATTEMPTS: u32 = 64;

/// Copies `config` beside itself under a timestamped name.
///
/// Returns `None` when there is no file: an empty backup would later "restore" the configuration
/// to nothing.
pub fn back_up(config: &Path, unix_seconds: i64, phase: Phase) -> Result<Option<PathBuf>> {
    if !config.exists() {
        return Ok(None);
    }

    let contents = std::fs::read(config).map_err(|error| unreadable(phase, &error.to_string()))?;
    let directory = config
        .parent()
        .ok_or_else(|| unreadable(phase, "the configuration file has no directory"))?;
    let file_name = config
        .file_name()
        .ok_or_else(|| unreadable(phase, "the configuration file has no name"))?
        .to_string_lossy()
        .into_owned();

    for attempt in 0..NAME_ATTEMPTS {
        let name = match attempt {
            0 => format!("{file_name}.{BACKUP_PREFIX}-{unix_seconds}"),
            // Two backups in the same second: the suffix keeps the earlier one from being overwritten.
            _ => format!("{file_name}.{BACKUP_PREFIX}-{unix_seconds}-{attempt}"),
        };
        let candidate = directory.join(name);
        if candidate.exists() {
            continue;
        }

        // The atomic write lands the backup complete and readable only by the user.
        atomic_write(&candidate, &contents)
            .map_err(|error| unreadable(phase, &error.to_string()))?;
        return Ok(Some(candidate));
    }

    Err(unreadable(
        phase,
        "no unused name for a configuration backup",
    ))
}

/// Whether `name` is a backup Toglet created.
pub fn is_toglet_backup(name: &str) -> bool {
    name.contains(BACKUP_PREFIX)
}

fn unreadable(phase: Phase, detail: &str) -> TogletError {
    TogletError::new(
        ErrorCode::CodexHomeUnwritable,
        phase,
        true,
        UserAction::FixConfigManually,
    )
    .with_detail(detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_home::{IsolatedHome, is_private};

    const ORIGINAL: &[u8] = b"# a comment\nmodel = \"gpt-5.6-sol\"\n";

    fn scratch() -> (IsolatedHome, PathBuf) {
        let home = IsolatedHome::create(Phase::Backup).expect("scratch home");
        let config = home.path().join("config.toml");
        std::fs::write(&config, ORIGINAL).expect("the configuration is written");
        (home, config)
    }

    #[test]
    fn a_backup_is_a_byte_for_byte_copy_beside_the_original() {
        let (home, config) = scratch();

        let backup = back_up(&config, 1_788_164_992, Phase::Backup)
            .expect("the backup succeeds")
            .expect("there was a file to copy");

        assert_eq!(std::fs::read(&backup).expect("readable"), ORIGINAL);
        assert_eq!(backup.parent(), Some(home.path()));
        assert_eq!(
            std::fs::read(&config).expect("readable"),
            ORIGINAL,
            "taking a backup must not modify the original"
        );
    }

    #[test]
    fn a_backup_carries_the_timestamp_and_a_toglet_prefix() {
        let (_home, config) = scratch();

        let backup = back_up(&config, 1_788_164_992, Phase::Backup)
            .expect("the backup succeeds")
            .expect("there was a file to copy");

        let name = backup
            .file_name()
            .expect("named")
            .to_string_lossy()
            .into_owned();
        assert_eq!(name, "config.toml.toglet-backup-1788164992");
        assert!(
            is_toglet_backup(&name),
            "a Toglet backup must be distinguishable from another tool's"
        );
    }

    #[test]
    fn a_backup_is_readable_only_by_its_owner() {
        let (_home, config) = scratch();

        let backup = back_up(&config, 1_788_164_992, Phase::Backup)
            .expect("the backup succeeds")
            .expect("there was a file to copy");

        assert!(
            is_private(&backup).expect("the permissions are readable"),
            "a copy of the configuration must not be world readable"
        );
    }

    #[test]
    fn a_second_backup_in_the_same_second_does_not_overwrite_the_first() {
        let (_home, config) = scratch();
        let first = back_up(&config, 1_788_164_992, Phase::Backup)
            .expect("the first backup succeeds")
            .expect("there was a file to copy");

        std::fs::write(&config, b"changed\n").expect("the configuration changes");
        let second = back_up(&config, 1_788_164_992, Phase::Backup)
            .expect("the second backup succeeds")
            .expect("there was a file to copy");

        assert_ne!(first, second);
        assert_eq!(
            std::fs::read(&first).expect("readable"),
            ORIGINAL,
            "the earlier state must survive a second backup in the same second"
        );
        assert_eq!(std::fs::read(&second).expect("readable"), b"changed\n");
    }

    #[test]
    fn a_missing_configuration_yields_no_backup_rather_than_an_empty_one() {
        let home = IsolatedHome::create(Phase::Backup).expect("scratch home");

        let backup = back_up(&home.path().join("absent.toml"), 1, Phase::Backup)
            .expect("a missing file is not a failure");

        assert!(
            backup.is_none(),
            "an empty backup would later restore the configuration to nothing"
        );
    }

    #[test]
    fn another_tools_backup_is_not_claimed_as_ours() {
        assert!(!is_toglet_backup("config.toml.agentport-bak-1788164992"));
    }
}
