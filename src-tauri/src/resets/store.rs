//! `resets.json`: the switch, the chosen channels, the dedupe markers, and the last good
//! reading with when it was taken. Holds nothing that could reach anywhere; there is no
//! credential entry because the feed needs none.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::feed::ResetStatus;
use super::watch::Markers;
use crate::codex_home::atomic_write;
use crate::diagnostics::{
    ErrorCode, Level, LogRecord, Phase, Result, TogletError, UserAction, log,
};
use crate::storage::{LoadOutcome, LoadProblem, read_schema_version};

const PHASE: Phase = Phase::Resets;

const RESETS_FILE: &str = "resets.json";

/// The version this build writes. Bump it together with a migration step.
pub const RESETS_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetsConfig {
    pub schema_version: u32,
    /// Off by default: off means no request leaves the machine.
    pub enabled: bool,
    /// Notification channels chosen for reset alerts, by id. The channels themselves live in
    /// `notifications.json`; an id whose channel is gone is dropped when read.
    pub channel_ids: Vec<String>,
    pub markers: Markers,
    /// The last good reading, so the panel has something at start-up (marked stale).
    pub cached: Option<ResetStatus>,
    /// Unix seconds of the last good reading; `None` until there has been one.
    pub fetched_at: Option<i64>,
}

impl Default for ResetsConfig {
    fn default() -> Self {
        Self {
            schema_version: RESETS_SCHEMA_VERSION,
            enabled: false,
            channel_ids: Vec::new(),
            markers: Markers::default(),
            cached: None,
            fetched_at: None,
        }
    }
}

impl ResetsConfig {
    /// Switches off. The markers go too: after a week off, the first reading back on must
    /// record silently again rather than announce everything that happened meanwhile. The
    /// cache stays, so the banner is not blank the moment the switch comes back.
    pub fn disable(&mut self) {
        self.enabled = false;
        self.markers = Markers::default();
    }
}

pub struct ResetsStore {
    path: PathBuf,
}

impl ResetsStore {
    /// `directory` must be the existing, private application data directory.
    pub fn new(directory: &Path) -> Self {
        Self {
            path: directory.join(RESETS_FILE),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Loads the settings; an unusable file is rebuilt with the feature off.
    pub fn load(&self) -> (ResetsConfig, LoadOutcome) {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return (ResetsConfig::default(), LoadOutcome::Created);
        };

        let parsed = read_schema_version(&text)
            .ok_or(LoadProblem::Unreadable)
            .and_then(|version| {
                if version > RESETS_SCHEMA_VERSION {
                    Err(LoadProblem::FromTheFuture { found: version })
                } else {
                    Ok(())
                }
            })
            .and_then(|()| {
                serde_json::from_str::<ResetsConfig>(&text).map_err(|_| LoadProblem::Unreadable)
            });

        match parsed {
            Ok(config) => (config, LoadOutcome::Loaded),
            Err(problem) => {
                log(&LogRecord::new(Level::Error, "resets_settings_rebuilt")
                    .with_phase(PHASE)
                    .with_code(ErrorCode::Internal)
                    .with_detail(match problem {
                        LoadProblem::Unreadable => "the reset settings could not be parsed",
                        LoadProblem::FromTheFuture { .. } => {
                            "the reset settings were written by a newer version"
                        }
                    }));
                (ResetsConfig::default(), LoadOutcome::Rebuilt { problem })
            }
        }
    }

    pub fn save(&self, config: &ResetsConfig) -> Result<()> {
        let json = serde_json::to_vec_pretty(config).map_err(|error| {
            TogletError::new(ErrorCode::Internal, PHASE, false, UserAction::None)
                .with_detail(&error.to_string())
        })?;

        atomic_write(&self.path, &json).map_err(|error| {
            TogletError::new(
                ErrorCode::CodexHomeUnwritable,
                PHASE,
                true,
                UserAction::Retry,
            )
            .with_detail(&error.to_string())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_home::IsolatedHome;
    use crate::resets::feed::{Reset, ResetKind, Stats};

    /// A private scratch directory, removed on drop; never the real data directory.
    fn temp() -> IsolatedHome {
        IsolatedHome::create(Phase::Storage).expect("scratch directory")
    }

    fn reading() -> ResetStatus {
        ResetStatus {
            latest_reset: Some(Reset {
                id: "r1".to_owned(),
                kind: ResetKind::Regular,
                announced_at: 100,
                text: Some("Reset all propagated.".to_owned()),
            }),
            scheduled_reset: None,
            active_watch: None,
            stats: Stats {
                total: 1,
                last_reset_at: Some(100),
                days_since_last: None,
                avg_interval_days: None,
            },
            generated_at: 200,
        }
    }

    #[test]
    fn the_default_is_off_with_nothing_chosen() {
        let config = ResetsConfig::default();
        assert!(!config.enabled);
        assert!(config.channel_ids.is_empty());
        assert!(!config.markers.seen);
        assert!(config.cached.is_none());
    }

    #[test]
    fn a_missing_file_reads_as_the_default() {
        let dir = temp();
        let (config, outcome) = ResetsStore::new(dir.path()).load();
        assert_eq!(config, ResetsConfig::default());
        assert_eq!(outcome, LoadOutcome::Created);
    }

    #[test]
    fn what_is_saved_is_read_back_including_the_cached_reading() {
        let dir = temp();
        let store = ResetsStore::new(dir.path());
        let config = ResetsConfig {
            enabled: true,
            channel_ids: vec!["chan-1".to_owned()],
            markers: Markers {
                seen: true,
                reset_id: Some("r1".to_owned()),
                ..Markers::default()
            },
            cached: Some(reading()),
            fetched_at: Some(300),
            ..ResetsConfig::default()
        };
        store.save(&config).expect("saved");

        let (loaded, outcome) = store.load();
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert_eq!(loaded, config);
    }

    #[test]
    fn a_file_from_a_newer_build_is_refused_and_rebuilt_off() {
        let dir = temp();
        let store = ResetsStore::new(dir.path());
        std::fs::write(
            store.path(),
            format!(
                r#"{{"schemaVersion":{},"enabled":true}}"#,
                RESETS_SCHEMA_VERSION + 1
            ),
        )
        .expect("written");
        let (config, outcome) = store.load();
        assert!(!config.enabled);
        assert!(matches!(
            outcome,
            LoadOutcome::Rebuilt {
                problem: LoadProblem::FromTheFuture { .. }
            }
        ));
    }

    #[test]
    fn a_corrupt_file_is_rebuilt_off_rather_than_guessed_at() {
        let dir = temp();
        let store = ResetsStore::new(dir.path());
        std::fs::write(store.path(), "{not json").expect("written");
        let (config, outcome) = store.load();
        assert_eq!(config, ResetsConfig::default());
        assert!(matches!(
            outcome,
            LoadOutcome::Rebuilt {
                problem: LoadProblem::Unreadable
            }
        ));
    }

    #[test]
    fn switching_off_forgets_the_markers_but_keeps_the_cache() {
        let mut config = ResetsConfig {
            enabled: true,
            markers: Markers {
                seen: true,
                reset_id: Some("r1".to_owned()),
                ..Markers::default()
            },
            cached: Some(reading()),
            ..ResetsConfig::default()
        };
        config.disable();
        assert!(!config.enabled);
        assert_eq!(config.markers, Markers::default());
        assert!(config.cached.is_some());
    }

    #[test]
    fn the_file_holds_no_credential_and_no_address_of_its_own() {
        let dir = temp();
        let store = ResetsStore::new(dir.path());
        let config = ResetsConfig {
            enabled: true,
            channel_ids: vec!["chan-1".to_owned()],
            ..ResetsConfig::default()
        };
        store.save(&config).expect("saved");
        let text = std::fs::read_to_string(store.path()).expect("readable");
        // Only the channel id links to a channel; its address stays in the credential store.
        assert!(text.contains("chan-1"));
        assert!(!text.contains("https://"));
        assert!(!text.contains("@"));
    }
}
