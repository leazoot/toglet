//! The channel list file and the credential entries it links to by id.
//! The file holds nothing that can post anywhere; addresses, keys and passwords stay in the
//! credential store.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::channel::{ChannelConfig, Connection};
use crate::codex_home::atomic_write;
use crate::credentials::{CredentialRef, Secret, SecretStore};
use crate::diagnostics::{
    ErrorCode, Level, LogRecord, Phase, Result, TogletError, UserAction, log,
};
use crate::storage::{LoadOutcome, LoadProblem};

const PHASE: Phase = Phase::Notify;

const CHANNELS_FILE: &str = "notifications.json";

/// The version this build writes. Bump it together with a migration step.
pub const CHANNELS_SCHEMA_VERSION: u32 = 1;

/// Channels are sent to sequentially, so the list is capped to bound the wait.
pub const MAX_CHANNELS: usize = 8;

/// The whole list, as the file holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelBook {
    pub schema_version: u32,
    /// Entries naming an unsupported service are dropped individually rather than failing the
    /// whole file; their credential entries are left behind, inert.
    #[serde(deserialize_with = "known_channels")]
    pub channels: Vec<ChannelConfig>,
}

/// Deserialises the list one entry at a time, keeping the ones this build understands.
fn known_channels<'de, D>(deserializer: D) -> std::result::Result<Vec<ChannelConfig>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Vec::<serde_json::Value>::deserialize(deserializer)?;
    let kept: Vec<ChannelConfig> = raw
        .iter()
        .filter_map(|entry| serde_json::from_value(entry.clone()).ok())
        .collect();
    if kept.len() != raw.len() {
        log(&LogRecord::new(Level::Warn, "notify_channel_dropped")
            .with_phase(PHASE)
            .with_detail(&format!("{} of {}", raw.len() - kept.len(), raw.len())));
    }
    Ok(kept)
}

impl Default for ChannelBook {
    fn default() -> Self {
        Self {
            schema_version: CHANNELS_SCHEMA_VERSION,
            channels: Vec::new(),
        }
    }
}

impl ChannelBook {
    pub fn find(&self, id: &str) -> Option<&ChannelConfig> {
        self.channels.iter().find(|channel| channel.id == id)
    }

    pub fn find_mut(&mut self, id: &str) -> Option<&mut ChannelConfig> {
        self.channels.iter_mut().find(|channel| channel.id == id)
    }

    /// An unused id: seconds, with a suffix when two channels are added within one second.
    pub fn fresh_id(&self, now: i64) -> String {
        let base = format!("chan-{now}");
        if self.find(&base).is_none() {
            return base;
        }
        (1..)
            .map(|suffix| format!("{base}-{suffix}"))
            .find(|candidate| self.find(candidate).is_none())
            .unwrap_or(base)
    }
}

pub struct ChannelStore {
    path: PathBuf,
}

impl ChannelStore {
    /// `directory` is the application data directory, which already exists and is private.
    pub fn new(directory: &Path) -> Self {
        Self {
            path: directory.join(CHANNELS_FILE),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Loads the list, or an empty one (reported, not repaired) if the file is unusable.
    pub fn load(&self) -> (ChannelBook, LoadOutcome) {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return (ChannelBook::default(), LoadOutcome::Created);
        };

        let parsed = read_schema_version(&text)
            .ok_or(LoadProblem::Unreadable)
            .and_then(|version| {
                if version > CHANNELS_SCHEMA_VERSION {
                    Err(LoadProblem::FromTheFuture { found: version })
                } else {
                    Ok(())
                }
            })
            .and_then(|()| {
                serde_json::from_str::<ChannelBook>(&text).map_err(|_| LoadProblem::Unreadable)
            });

        match parsed {
            Ok(book) => (book, LoadOutcome::Loaded),
            Err(problem) => {
                log(&LogRecord::new(Level::Error, "notify_channels_rebuilt")
                    .with_phase(PHASE)
                    .with_code(ErrorCode::Internal)
                    .with_detail(match problem {
                        LoadProblem::Unreadable => "the channel file could not be parsed",
                        LoadProblem::FromTheFuture { .. } => {
                            "the channel file was written by a newer version"
                        }
                    }));
                (ChannelBook::default(), LoadOutcome::Rebuilt { problem })
            }
        }
    }

    /// Replaces the list. Either the whole new list lands or the old one stays.
    pub fn save(&self, book: &ChannelBook) -> Result<()> {
        let json = serde_json::to_vec_pretty(book).map_err(|error| {
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

/// Credential key for a channel; the `notify-` prefix cannot collide with accounts' `cred-`.
fn reference(id: &str) -> Result<CredentialRef> {
    CredentialRef::new(&format!("notify-{id}"))
}

/// Validates and stores a channel's connection, replacing any previous one.
pub fn store_connection(
    secrets: &dyn SecretStore,
    id: &str,
    connection: &Connection,
) -> Result<()> {
    connection.validate()?;
    let json = serde_json::to_vec(connection).map_err(|error| {
        TogletError::new(ErrorCode::Internal, PHASE, false, UserAction::None)
            .with_detail(&error.to_string())
    })?;
    secrets.store(&reference(id)?, &Secret::new(json))
}

/// A stored connection that no longer parses is refused rather than patched.
pub fn load_connection(secrets: &dyn SecretStore, id: &str) -> Result<Connection> {
    let secret = secrets.load(&reference(id)?)?;
    serde_json::from_slice::<Connection>(secret.expose()).map_err(|_| {
        TogletError::new(
            ErrorCode::Internal,
            PHASE,
            false,
            UserAction::FixNotificationChannel,
        )
        .with_detail("the stored channel details could not be read back")
    })
}

/// Removes a channel's connection. Removing one that is not there succeeds.
pub fn forget_connection(secrets: &dyn SecretStore, id: &str) -> Result<()> {
    secrets.delete(&reference(id)?)
}

/// Reads only `schemaVersion`. A file from a newer build usually fails a full parse, so the
/// version is checked first to report the real reason.
fn read_schema_version(text: &str) -> Option<u32> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct VersionOnly {
        schema_version: u32,
    }
    serde_json::from_str::<VersionOnly>(text)
        .ok()
        .map(|parsed| parsed.schema_version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_home::{IsolatedHome, permissions};
    use crate::credentials::MemorySecretStore;
    use crate::notify::channel::{ChannelKind, Delivery, MailSecurity};

    const NOW: &str = "2026-09-12T10:00:00Z";

    fn scratch() -> IsolatedHome {
        IsolatedHome::create(Phase::Storage).expect("scratch directory")
    }

    fn channel(id: &str) -> ChannelConfig {
        ChannelConfig {
            id: id.to_owned(),
            kind: ChannelKind::Wecom,
            label: "Team".to_owned(),
            enabled: true,
            hint: "qyapi.weixin.qq.com".to_owned(),
            created_at: NOW.to_owned(),
            last_delivery: Some(Delivery {
                at: 1_757_000_000,
                ok: true,
                code: None,
            }),
        }
    }

    fn wecom() -> Connection {
        Connection::Wecom {
            webhook: "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=9f8e-the-token"
                .to_owned(),
        }
    }

    /// A file naming a removed service loses that channel and keeps the rest.
    #[test]
    fn a_channel_for_a_service_this_build_no_longer_has_is_dropped_on_its_own() {
        let home = scratch();
        let store = ChannelStore::new(home.path());
        std::fs::write(
            store.path(),
            r#"{
              "schemaVersion": 1,
              "channels": [
                {"id":"chan-1","kind":"feishu","label":"Group","enabled":true,
                 "hint":"open.feishu.cn","createdAt":"2026-09-12T10:00:00Z","lastDelivery":null},
                {"id":"chan-2","kind":"bark","label":"Phone","enabled":true,
                 "hint":"api.day.app","createdAt":"2026-09-12T10:00:01Z","lastDelivery":null}
              ]
            }"#,
        )
        .expect("written");

        let (book, outcome) = store.load();

        // Loaded, not rebuilt: the file itself was fine, one entry in it was not.
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert_eq!(book.channels.len(), 1);
        assert_eq!(book.channels[0].id, "chan-2");
    }

    #[test]
    fn a_missing_file_yields_an_empty_list_rather_than_an_error() {
        let home = scratch();
        let (book, outcome) = ChannelStore::new(home.path()).load();

        assert_eq!(outcome, LoadOutcome::Created);
        assert!(book.channels.is_empty());
        assert_eq!(book.schema_version, CHANNELS_SCHEMA_VERSION);
    }

    #[test]
    fn a_saved_list_is_read_back_whole() {
        let home = scratch();
        let store = ChannelStore::new(home.path());
        let book = ChannelBook {
            schema_version: CHANNELS_SCHEMA_VERSION,
            channels: vec![channel("chan-1")],
        };
        store.save(&book).expect("saved");

        let (loaded, outcome) = store.load();
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert_eq!(loaded, book);
    }

    // Permissions before content: the file is private from the moment it exists.
    #[test]
    fn the_file_is_private() {
        let home = scratch();
        let store = ChannelStore::new(home.path());
        store.save(&ChannelBook::default()).expect("saved");
        permissions::assert_private(store.path());
    }

    #[test]
    fn a_corrupt_file_costs_the_channel_list_and_nothing_else() {
        let home = scratch();
        let store = ChannelStore::new(home.path());
        std::fs::write(store.path(), b"{ not json").expect("written");

        let (book, outcome) = store.load();
        assert_eq!(
            outcome,
            LoadOutcome::Rebuilt {
                problem: LoadProblem::Unreadable
            }
        );
        assert!(book.channels.is_empty());
    }

    #[test]
    fn a_file_from_a_newer_build_is_refused_rather_than_reinterpreted() {
        let home = scratch();
        let store = ChannelStore::new(home.path());
        std::fs::write(
            store.path(),
            br#"{"schemaVersion":99,"channels":[],"somethingNew":true}"#,
        )
        .expect("written");

        let (_, outcome) = store.load();
        assert_eq!(
            outcome,
            LoadOutcome::Rebuilt {
                problem: LoadProblem::FromTheFuture { found: 99 }
            }
        );
    }

    /// Asserted on the bytes on disk, not on the struct shape.
    #[test]
    fn what_lands_on_disk_carries_no_address_key_or_password() {
        let home = scratch();
        let store = ChannelStore::new(home.path());
        let secrets = MemorySecretStore::default();
        store_connection(&secrets, "chan-1", &wecom()).expect("stored");
        store
            .save(&ChannelBook {
                schema_version: CHANNELS_SCHEMA_VERSION,
                channels: vec![channel("chan-1")],
            })
            .expect("saved");

        let text = std::fs::read_to_string(store.path()).expect("readable");
        for forbidden in ["9f8e-the-token", "open-apis", "https://", "hunter2", "@"] {
            assert!(
                !text.contains(forbidden),
                "the channel file leaked {forbidden}"
            );
        }
        assert!(text.contains("qyapi.weixin.qq.com"), "the host is the hint");
    }

    #[test]
    fn a_connection_round_trips_through_the_credential_store() {
        let secrets = MemorySecretStore::default();
        store_connection(&secrets, "chan-1", &wecom()).expect("stored");

        assert_eq!(
            load_connection(&secrets, "chan-1").expect("read back"),
            wecom()
        );
    }

    #[test]
    fn an_unusable_connection_is_never_stored() {
        let secrets = MemorySecretStore::default();
        let broken = Connection::Webhook {
            url: "http://example.com/hook".to_owned(),
        };

        assert!(store_connection(&secrets, "chan-1", &broken).is_err());
        assert!(load_connection(&secrets, "chan-1").is_err());
    }

    #[test]
    fn forgetting_a_channel_that_was_never_there_succeeds() {
        let secrets = MemorySecretStore::default();
        forget_connection(&secrets, "chan-nothing").expect("removing nothing is not a failure");
    }

    #[test]
    fn a_mailbox_password_is_kept_with_the_connection_and_not_in_the_list() {
        let secrets = MemorySecretStore::default();
        let email = Connection::Email {
            host: "smtp.example.com".to_owned(),
            port: 587,
            security: MailSecurity::StartTls,
            username: "leanne".to_owned(),
            password: "hunter2".to_owned(),
            from: "leanne@example.com".to_owned(),
            to: "team@example.com".to_owned(),
        };
        store_connection(&secrets, "chan-2", &email).expect("stored");

        assert_eq!(
            load_connection(&secrets, "chan-2").expect("read back"),
            email
        );
        // A local part of four characters or fewer is masked whole (the account masking rule).
        assert_eq!(email.hint(), "***@example.com");
    }

    #[test]
    fn a_fresh_id_never_repeats_one_already_in_the_list() {
        let mut book = ChannelBook::default();
        let first = book.fresh_id(1_757_000_000);
        book.channels.push(channel(&first));

        let second = book.fresh_id(1_757_000_000);
        assert_ne!(first, second);
        assert!(book.find(&second).is_none());
    }

    #[test]
    fn a_channel_credential_cannot_collide_with_an_account_credential() {
        let channel = reference("chan-1").expect("valid");
        assert_eq!(channel.as_str(), "notify-chan-1");
        assert!(!channel.as_str().starts_with("cred-"));
    }
}
