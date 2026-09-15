//! The on-disk metadata document and its version handling.
//!
//! Holds no credential material: only masked addresses, irreversible fingerprints and
//! `credentialRef` keys into the platform credential store.

use serde::{Deserialize, Serialize};

use super::settings::AppSettings;
use crate::accounts::AccountProfile;

/// The version this build writes. Bump it together with a migration step.
pub const CURRENT_SCHEMA_VERSION: u32 = 4;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataDocument {
    pub schema_version: u32,
    pub accounts: Vec<AccountProfile>,
    pub settings: AppSettings,
    /// What Toglet changed in Codex's configuration, kept so it can be restored after a restart.
    #[serde(default)]
    pub codex_config: CodexConfigState,
}

/// Whether Toglet currently holds a change to Codex's `config.toml`, and what it replaced.
///
/// Carries no backup path: absolute paths must not be written into the metadata document.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum CodexConfigState {
    #[default]
    Untouched,
    /// Toglet set the credential store. `None` means the key did not exist, so restoring
    /// removes it rather than writing an empty string.
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

/// Brings a parsed document up to [`CURRENT_SCHEMA_VERSION`], one explicit step per version.
///
/// A document from a newer build is refused: writing it back would drop unknown fields.
pub fn migrate(mut document: MetadataDocument) -> Result<MetadataDocument, LoadProblem> {
    if document.schema_version > CURRENT_SCHEMA_VERSION {
        return Err(LoadProblem::FromTheFuture {
            found: document.schema_version,
        });
    }

    // Version 1 never wrote Codex's configuration, so there is nothing to put back.
    if document.schema_version < 2 {
        document.schema_version = 2;
        document.codex_config = CodexConfigState::Untouched;
    }

    // Version 2 had no language setting; an upgrade must not pin someone to English.
    if document.schema_version < 3 {
        document.schema_version = 3;
        document.settings.language = crate::storage::Language::System;
    }

    // Version 3 always showed the bar and could not hide it.
    if document.schema_version < 4 {
        document.schema_version = 4;
        document.settings.dock_shape = crate::storage::DockShape::Bar;
        document.settings.dock_hidden = false;
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

        assert!(json.contains("\"schemaVersion\":4"));
        assert!(json.contains("\"accounts\":[]"));
    }

    /// A document as version 1 wrote it.
    const VERSION_ONE: &str = r#"{"schemaVersion":1,"accounts":[],"settings":{
        "activeAccountId":null,"dockEdge":"right","displayId":null,"verticalOffset":0,
        "launchAtLogin":false,"alwaysOnTop":true,"avoidFullscreen":true,
        "activeRefreshSeconds":60,"inactiveRefreshSeconds":300,
        "reopenCodexAfterSwitch":true,"theme":"system","reduceMotion":false}}"#;

    /// A document as version 2 wrote it: no language.
    const VERSION_TWO: &str = r#"{"schemaVersion":2,"accounts":[],"codexConfig":{"state":"untouched"},
        "settings":{
        "activeAccountId":null,"dockEdge":"right","displayId":null,"verticalOffset":0,
        "launchAtLogin":false,"alwaysOnTop":true,"avoidFullscreen":true,
        "activeRefreshSeconds":60,"inactiveRefreshSeconds":300,
        "reopenCodexAfterSwitch":true,"theme":"system","reduceMotion":false}}"#;

    /// A document as version 3 wrote it: no dock shape or hidden flag.
    const VERSION_THREE: &str = r#"{"schemaVersion":3,"accounts":[],"codexConfig":{"state":"untouched"},
        "settings":{
        "activeAccountId":null,"dockEdge":"right","displayId":null,"verticalOffset":0,
        "launchAtLogin":false,"alwaysOnTop":true,"avoidFullscreen":true,
        "activeRefreshSeconds":60,"inactiveRefreshSeconds":300,
        "reopenCodexAfterSwitch":true,"theme":"system","reduceMotion":false,"language":"zh"}}"#;

    #[test]
    fn a_version_three_document_steps_up_showing_the_bar_it_always_showed() {
        let parsed: MetadataDocument = serde_json::from_str(VERSION_THREE).expect("parses");

        let migrated = migrate(parsed).expect("a previous version is accepted");

        assert_eq!(migrated.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(migrated.settings.dock_shape, crate::storage::DockShape::Bar);
        assert!(!migrated.settings.dock_hidden);
        assert_eq!(
            migrated.settings.language,
            crate::storage::Language::Zh,
            "a choice already made is kept"
        );
    }

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
