//! The on-disk metadata document and its version handling.
//!
//! **Nothing in here may hold credential material.** The document carries masked addresses,
//! irreversible fingerprints and `credentialRef` keys - a key into the platform credential
//! store, never anything that can be decrypted.

use serde::{Deserialize, Serialize};

use super::settings::AppSettings;
use crate::accounts::AccountProfile;

/// The version this build writes. Bump it together with a migration step.
pub const CURRENT_SCHEMA_VERSION: u32 = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataDocument {
    pub schema_version: u32,
    pub accounts: Vec<AccountProfile>,
    pub settings: AppSettings,
    /// What Toglet changed in Codex's own configuration. Added in schema version 2, because
    /// "stop managing Codex authentication" has to survive a restart to be able to put the
    /// setting back.
    #[serde(default)]
    pub codex_config: CodexConfigState,
}

/// Whether Toglet currently holds a change to Codex's `config.toml`, and what it replaced.
///
/// Deliberately an enum rather than an `Option<Option<String>>`: "Toglet changed nothing" and
/// "Toglet added a key that did not exist" are different states with different restores, and
/// nesting two options is exactly how they get confused.
///
/// Carries no path. A backup's location is an absolute path, which must not be written into the
/// metadata document; putting the setting back needs only the value it used to hold.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum CodexConfigState {
    /// Codex's configuration is as the user left it.
    #[default]
    Untouched,
    /// Toglet set the credential store. `previousValue` is what to put back; absent means the
    /// key did not exist, so restoring removes it rather than writing an empty string.
    #[serde(rename_all = "camelCase")]
    Managed { previous_value: Option<String> },
}

impl Default for MetadataDocument {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            accounts: Vec::new(),
            settings: AppSettings::default(),
            codex_config: CodexConfigState::Untouched,
        }
    }
}

/// Why a document could not be loaded as it stood.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadProblem {
    /// The file could not be parsed at all.
    Unreadable,
    /// Written by a newer build.
    FromTheFuture { found: u32 },
}

/// Brings a parsed document up to [`CURRENT_SCHEMA_VERSION`].
///
/// Versions are stepped explicitly, one at a time. There is deliberately no "guess whether this
/// field exists" compatibility path: implicit tolerance is how a file silently loses data.
///
/// A document from a **newer** build is refused rather than parsed. Reading a structure this
/// build does not understand and writing it back would drop whatever it did not recognise.
pub fn migrate(mut document: MetadataDocument) -> Result<MetadataDocument, LoadProblem> {
    if document.schema_version > CURRENT_SCHEMA_VERSION {
        return Err(LoadProblem::FromTheFuture {
            found: document.schema_version,
        });
    }

    // Version 1 knew nothing about Codex's own configuration, and a build that never wrote the
    // setting has nothing to put back, so `Untouched` is the truthful value rather than a
    // convenient default. Serde supplies it; the step is spelled out because a version that is
    // stepped silently is a version nobody notices going wrong.
    if document.schema_version < 2 {
        document.schema_version = 2;
        document.codex_config = CodexConfigState::Untouched;
    }

    // Version 2 had no language setting, so no user of that build ever chose one. `System` is
    // what "no choice recorded" means, which makes it the truthful value here rather than a
    // fallback - an upgrade must not silently pin someone to English.
    if document.schema_version < 3 {
        document.schema_version = 3;
        document.settings.language = crate::storage::Language::System;
    }

    Ok(document)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_document_is_empty_and_current() {
        let document = MetadataDocument::default();

        assert_eq!(document.schema_version, CURRENT_SCHEMA_VERSION);
        assert!(document.accounts.is_empty());
        assert_eq!(document.settings.active_account_id(), None);
    }

    #[test]
    fn a_document_from_a_newer_build_is_refused_rather_than_parsed() {
        let document = MetadataDocument {
            schema_version: CURRENT_SCHEMA_VERSION + 1,
            ..MetadataDocument::default()
        };

        assert_eq!(
            migrate(document),
            Err(LoadProblem::FromTheFuture {
                found: CURRENT_SCHEMA_VERSION + 1
            })
        );
    }

    #[test]
    fn a_current_document_passes_through_unchanged() {
        let document = MetadataDocument::default();

        assert_eq!(migrate(document.clone()), Ok(document));
    }

    #[test]
    fn the_wire_form_is_camel_case_and_carries_the_version() {
        let json = serde_json::to_string(&MetadataDocument::default()).expect("serialises");

        assert!(json.contains("\"schemaVersion\":3"));
        assert!(json.contains("\"accounts\":[]"));
    }

    /// A document exactly as version 1 wrote it: every settings field of that build, and no
    /// knowledge of Codex's own configuration.
    const VERSION_ONE: &str = r#"{"schemaVersion":1,"accounts":[],"settings":{
        "activeAccountId":null,"dockEdge":"right","displayId":null,"verticalOffset":0,
        "launchAtLogin":false,"alwaysOnTop":true,"avoidFullscreen":true,
        "activeRefreshSeconds":60,"inactiveRefreshSeconds":300,
        "reopenCodexAfterSwitch":true,"theme":"system","reduceMotion":false}}"#;

    /// A document exactly as version 2 wrote it: it knew about Codex's configuration, and nothing
    /// about a language.
    const VERSION_TWO: &str = r#"{"schemaVersion":2,"accounts":[],"codexConfig":{"state":"untouched"},
        "settings":{
        "activeAccountId":null,"dockEdge":"right","displayId":null,"verticalOffset":0,
        "launchAtLogin":false,"alwaysOnTop":true,"avoidFullscreen":true,
        "activeRefreshSeconds":60,"inactiveRefreshSeconds":300,
        "reopenCodexAfterSwitch":true,"theme":"system","reduceMotion":false}}"#;

    #[test]
    fn a_version_one_document_steps_up_and_reports_no_managed_configuration() {
        let parsed: MetadataDocument = serde_json::from_str(VERSION_ONE).expect("parses");

        let migrated = migrate(parsed).expect("a previous version is accepted");

        assert_eq!(migrated.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(
            migrated.codex_config,
            CodexConfigState::Untouched,
            "a build that never wrote the setting has nothing to put back"
        );
    }

    #[test]
    fn a_version_two_document_steps_up_and_follows_the_system_language() {
        let parsed: MetadataDocument = serde_json::from_str(VERSION_TWO).expect("parses");

        let migrated = migrate(parsed).expect("a previous version is accepted");

        assert_eq!(migrated.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(
            migrated.settings.language,
            crate::storage::Language::System,
            "an upgrade must not pin a user to a language they never picked"
        );
    }

    #[test]
    fn a_managed_configuration_survives_a_round_trip() {
        let document = MetadataDocument {
            codex_config: CodexConfigState::Managed {
                previous_value: Some("keyring".to_owned()),
            },
            ..MetadataDocument::default()
        };

        let json = serde_json::to_string(&document).expect("serialises");
        let read_back: MetadataDocument = serde_json::from_str(&json).expect("parses");

        assert_eq!(read_back, document);
    }

    #[test]
    fn an_added_key_is_stored_as_absent_rather_than_as_an_empty_string() {
        let json = serde_json::to_string(&MetadataDocument {
            codex_config: CodexConfigState::Managed {
                previous_value: None,
            },
            ..MetadataDocument::default()
        })
        .expect("serialises");

        assert!(json.contains(r#""state":"managed""#));
        assert!(
            json.contains(r#""previousValue":null"#),
            "absent must not collapse into an empty value: {json}"
        );
    }
}
