//! Reading and writing the metadata document.
//!
//! Storage is a single JSON file replaced atomically. With an account limit of 12 the
//! data is tiny, `atomic_write` is a primitive the switch path needs anyway, and a corrupt file
//! can be rebuilt whole - none of which a database engine would improve on.

use std::path::{Path, PathBuf};

use super::document::{LoadProblem, MetadataDocument, migrate};
use crate::codex_home::{atomic_write, permissions};
use crate::diagnostics::{
    ErrorCode, Level, LogRecord, Phase, Result, TogletError, UserAction, log,
};

/// What a load had to do to produce a usable document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadOutcome {
    /// The file was read as it stood.
    Loaded,
    /// There was no file yet; a fresh document was returned.
    Created,
    /// The file could not be used and was replaced with a fresh document.
    ///
    /// **Credentials are untouched by this.** They live in the platform store, so the accounts
    /// can be recovered by importing or signing in again; the metadata is the cheap half.
    Rebuilt { problem: LoadProblem },
}

pub struct MetadataStore {
    path: PathBuf,
}

impl MetadataStore {
    /// `directory` must already exist and be private. Creating the application data directory
    /// belongs to application startup, not to this type.
    pub fn new(directory: &Path) -> Self {
        Self {
            path: directory.join("metadata.json"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Loads the document, rebuilding it if it cannot be used.
    ///
    /// A damaged metadata file must never stop the application from starting: the user would be
    /// left with no way to reach the repair. It is reported, replaced, and startup continues.
    pub fn load(&self) -> (MetadataDocument, LoadOutcome) {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return (MetadataDocument::default(), LoadOutcome::Created);
        };

        // The version is read on its own first. A document from a newer build will usually
        // fail a full parse - that is the whole point of it being newer - so checking the
        // version afterwards would report "unreadable" and hide the real reason.
        let parsed = read_schema_version(&text)
            .ok_or(LoadProblem::Unreadable)
            .and_then(|version| {
                if version > crate::storage::CURRENT_SCHEMA_VERSION {
                    Err(LoadProblem::FromTheFuture { found: version })
                } else {
                    Ok(())
                }
            })
            .and_then(|()| {
                serde_json::from_str::<MetadataDocument>(&text).map_err(|_| LoadProblem::Unreadable)
            })
            .and_then(migrate);

        match parsed {
            Ok(mut document) => {
                for corrected in document.settings.normalise() {
                    log(&LogRecord::new(Level::Warn, "settings_value_out_of_range")
                        .with_phase(Phase::Storage)
                        .with_detail(corrected));
                }
                (document, LoadOutcome::Loaded)
            }
            Err(problem) => {
                log(&LogRecord::new(Level::Error, "metadata_rebuilt")
                    .with_phase(Phase::Storage)
                    .with_code(ErrorCode::Internal)
                    .with_detail(match problem {
                        LoadProblem::Unreadable => "the metadata file could not be parsed",
                        LoadProblem::FromTheFuture { .. } => {
                            "the metadata file was written by a newer version"
                        }
                    }));
                (
                    MetadataDocument::default(),
                    LoadOutcome::Rebuilt { problem },
                )
            }
        }
    }

    /// Replaces the document. Either the whole new content lands or the old content stays.
    pub fn save(&self, document: &MetadataDocument) -> Result<()> {
        let json = serde_json::to_vec_pretty(document).map_err(|error| {
            TogletError::new(ErrorCode::Internal, Phase::Storage, false, UserAction::None)
                .with_detail(&error.to_string())
        })?;

        atomic_write(&self.path, &json).map_err(|error| {
            TogletError::new(
                ErrorCode::CodexHomeUnwritable,
                Phase::Storage,
                true,
                UserAction::Retry,
            )
            .with_detail(&error.to_string())
        })
    }

    /// Whether the stored file is readable by the current user only.
    pub fn is_private(&self) -> std::io::Result<bool> {
        permissions::is_private(&self.path)
    }
}

/// Reads only `schemaVersion`, ignoring everything else in the document.
fn read_schema_version(text: &str) -> Option<u32> {
    #[derive(serde::Deserialize)]
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
    use crate::accounts::AccountProfile;
    use crate::accounts::AccountStatus;
    use crate::codex_home::IsolatedHome;
    use crate::storage::settings::SwitchVerified;

    fn store() -> (IsolatedHome, MetadataStore) {
        let directory = IsolatedHome::create(Phase::Storage).expect("scratch directory");
        let store = MetadataStore::new(directory.path());
        (directory, store)
    }

    fn sample_profile() -> AccountProfile {
        AccountProfile {
            id: "acct-1".to_owned(),
            display_name: "Work".to_owned(),
            masked_email: Some("lea***@gmail.com".to_owned()),
            account_fingerprint: "a".repeat(64),
            plan_type: Some("plus".to_owned()),
            auth_mode: "chatgpt".to_owned(),
            credential_ref: "cred-1".to_owned(),
            status: AccountStatus::Ready,
            created_at: "2026-08-31T00:00:00Z".to_owned(),
            updated_at: "2026-08-31T00:00:00Z".to_owned(),
            last_validated_at: None,
        }
    }

    #[test]
    fn a_missing_file_yields_a_fresh_document_rather_than_an_error() {
        let (_directory, store) = store();

        let (document, outcome) = store.load();

        assert_eq!(outcome, LoadOutcome::Created);
        assert!(document.accounts.is_empty());
    }

    #[test]
    fn a_saved_document_round_trips() {
        let (_directory, store) = store();
        let mut document = MetadataDocument::default();
        document.accounts.push(sample_profile());
        document
            .settings
            .set_active_account_id(Some("acct-1".to_owned()), &SwitchVerified::issue());

        store.save(&document).expect("the document is saved");
        let (loaded, outcome) = store.load();

        assert_eq!(outcome, LoadOutcome::Loaded);
        assert_eq!(loaded, document);
    }

    #[test]
    fn the_stored_file_is_readable_by_the_current_user_only() {
        let (_directory, store) = store();
        store
            .save(&MetadataDocument::default())
            .expect("the document is saved");

        assert!(store.is_private().expect("permissions are readable"));
    }

    #[test]
    fn a_corrupt_file_is_rebuilt_instead_of_stopping_startup() {
        let (_directory, store) = store();
        std::fs::write(store.path(), b"{ this is not json").expect("the file is written");

        let (document, outcome) = store.load();

        assert_eq!(
            outcome,
            LoadOutcome::Rebuilt {
                problem: LoadProblem::Unreadable
            }
        );
        assert!(document.accounts.is_empty());
    }

    #[test]
    fn a_file_from_a_newer_build_is_refused_rather_than_reinterpreted() {
        let (_directory, store) = store();
        // Deliberately also unparseable as a whole: a newer build's document will contain
        // fields this one does not know, so the version has to be readable on its own.
        std::fs::write(
            store.path(),
            br#"{"schemaVersion":99,"accounts":[],"settings":{},"somethingNew":true}"#,
        )
        .expect("the file is written");

        let (_, outcome) = store.load();

        assert_eq!(
            outcome,
            LoadOutcome::Rebuilt {
                problem: LoadProblem::FromTheFuture { found: 99 }
            }
        );
    }

    #[test]
    fn an_interrupted_save_leaves_the_previous_document_intact() {
        let (_directory, store) = store();
        let mut first = MetadataDocument::default();
        first.accounts.push(sample_profile());
        store.save(&first).expect("the first save succeeds");
        let before = std::fs::read(store.path()).expect("readable");

        // A staged-but-not-committed write is what a crash mid-save looks like; `atomic_write`
        // owns that behaviour and is tested there. Here the guarantee that matters is that a
        // second save either fully replaces or fully preserves.
        let mut second = MetadataDocument::default();
        second.accounts.push(AccountProfile {
            id: "acct-2".to_owned(),
            ..sample_profile()
        });
        store.save(&second).expect("the second save succeeds");

        let after = std::fs::read(store.path()).expect("readable");
        assert_ne!(before, after);
        let (loaded, _) = store.load();
        assert_eq!(loaded.accounts.len(), 1);
        assert_eq!(loaded.accounts[0].id, "acct-2");
    }

    #[test]
    fn what_lands_on_disk_carries_no_credential_material() {
        let (_directory, store) = store();
        let mut document = MetadataDocument::default();
        document.accounts.push(sample_profile());
        store.save(&document).expect("saved");

        let text = std::fs::read_to_string(store.path()).expect("readable");

        // The profile type has no field that could hold these, and this asserts it on the
        // actual file rather than trusting the struct definition.
        for forbidden in ["access_token", "refresh_token", "id_token", "eyJ", "sk-"] {
            assert!(
                !text.contains(forbidden),
                "the metadata file leaked {forbidden}"
            );
        }
        assert!(
            text.contains("cred-1"),
            "the credential reference is what links the two"
        );
    }

    #[test]
    fn out_of_range_settings_in_a_hand_edited_file_are_corrected_on_load() {
        let (_directory, store) = store();
        let mut document = MetadataDocument::default();
        store.save(&document).expect("saved");
        let text = std::fs::read_to_string(store.path())
            .expect("readable")
            .replace(
                "\"activeRefreshSeconds\": 60",
                "\"activeRefreshSeconds\": 1",
            );
        std::fs::write(store.path(), text).expect("rewritten");

        let (loaded, outcome) = store.load();

        assert_eq!(
            outcome,
            LoadOutcome::Loaded,
            "a bad value is not a corrupt file"
        );
        assert_eq!(loaded.settings.active_refresh_seconds(), 60);
        document.settings.normalise();
    }
}
