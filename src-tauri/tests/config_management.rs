//! Managing `cli_auth_credentials_store` end to end, against the fake app server.
//!
//! No real Codex, no real configuration file. What the *real* server does to `config.toml` -
//! comments preserved, key added once, byte-identical on a repeat - was measured directly;
//! what these tests pin down is Toglet's policy: when it refuses, what it backs up, and that
//! it never reports an ineffective write as success.

mod support;

use std::path::{Path, PathBuf};

use support::{fake_binary, scenario_home};
use toglet_lib::app_server::{AppServerClient, AppServerSession};
use toglet_lib::codex_config::{
    CredentialStoreOutcome, RestoreOutcome, enable_file_credential_store, is_toglet_backup,
    restore_credential_store,
};
use toglet_lib::codex_home::{IsolatedHome, ServerHome};
use toglet_lib::diagnostics::{ErrorCode, Phase};
use toglet_lib::storage::{CodexConfigState, MetadataDocument, MetadataStore};

const NOW: i64 = 1_788_164_992;
const PHASE: Phase = Phase::Write;

const EXISTING_CONFIG: &[u8] = b"# kept by the user\nmodel = \"gpt-5.6-sol\"\n";

/// A session that believes it is running against the user's real Codex home.
///
/// The directory is still a throwaway one - the returned [`IsolatedHome`] deletes it when the
/// test ends - but the session sees [`ServerHome::Default`], which is the variant the config
/// manager insists on. Without this the manager refuses to run at all, which is the point of
/// the guard.
fn default_home_session(scenario: &str) -> (IsolatedHome, AppServerSession) {
    let home = scenario_home(scenario, PHASE);
    std::fs::write(home.path().join("config.toml"), EXISTING_CONFIG)
        .expect("the configuration is written");

    let server_home = ServerHome::Default {
        path: home.path().to_path_buf(),
        phase: PHASE,
    };
    let client =
        AppServerClient::start(&fake_binary(PHASE), server_home).expect("the fake server starts");
    let session = AppServerSession::open(client).expect("the handshake succeeds");
    (home, session)
}

fn backups(directory: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(directory)
        .expect("readable")
        .filter_map(|entry| {
            let path = entry.expect("entry").path();
            let name = path.file_name()?.to_string_lossy().into_owned();
            is_toglet_backup(&name).then_some(path)
        })
        .collect()
}

#[test]
fn enabling_file_mode_backs_up_first_and_verifies_afterwards() {
    let (home, mut session) = default_home_session("config_absent");

    let outcome = enable_file_credential_store(&mut session, NOW).expect("the setting is applied");

    let CredentialStoreOutcome::Enabled(record) = outcome else {
        panic!("the setting was absent, so it had to be written");
    };
    assert_eq!(
        record.previous_value, None,
        "the key was absent, and absent is not the empty string"
    );
    let backup = record
        .backup
        .expect("a configuration existed, so it was copied");
    assert_eq!(
        std::fs::read(&backup).expect("readable"),
        EXISTING_CONFIG,
        "the backup must hold the state from before the change"
    );

    session.close().expect("the server exits cleanly");
    drop(home);
}

#[test]
fn a_repeat_writes_nothing_and_leaves_no_second_backup() {
    let (home, mut session) = default_home_session("config_already_file");

    let outcome = enable_file_credential_store(&mut session, NOW).expect("the setting is applied");

    assert_eq!(
        outcome,
        CredentialStoreOutcome::AlreadyEnabled,
        "repeating the operation must be a no-op"
    );
    assert!(
        backups(home.path()).is_empty(),
        "a no-op must not leave a copy of the user's configuration behind"
    );

    session.close().expect("the server exits cleanly");
    drop(home);
}

#[test]
fn an_organisation_enforced_configuration_stops_before_anything_is_touched() {
    let (home, mut session) = default_home_session("config_org_enforced");

    let error = enable_file_credential_store(&mut session, NOW)
        .expect_err("an enforced configuration must stop the operation");

    assert_eq!(error.code(), ErrorCode::ConfigLayerReadonly);
    assert!(
        backups(home.path()).is_empty(),
        "stopping must happen before the configuration is copied"
    );
    assert_eq!(
        std::fs::read(home.path().join("config.toml")).expect("readable"),
        EXISTING_CONFIG
    );

    session.close().expect("the server exits cleanly");
    drop(home);
}

#[test]
fn a_setting_owned_by_a_managed_layer_is_not_written_through() {
    // The value is already `file`, but it comes from an MDM layer. Toglet reports it as
    // already enabled rather than trying to claim ownership of it.
    let (home, mut session) = default_home_session("config_managed_layer");

    let outcome = enable_file_credential_store(&mut session, NOW).expect("the state is readable");

    assert_eq!(outcome, CredentialStoreOutcome::AlreadyEnabled);
    assert!(backups(home.path()).is_empty());

    session.close().expect("the server exits cleanly");
    drop(home);
}

#[test]
fn a_concurrent_edit_is_reported_as_a_conflict_not_as_an_incompatible_runtime() {
    let (home, mut session) = default_home_session("config_version_conflict");

    let error = enable_file_credential_store(&mut session, NOW)
        .expect_err("a stale version must be refused");

    assert_eq!(
        error.code(),
        ErrorCode::ConfigConflict,
        "another tool editing the file must not be reported as a broken runtime"
    );
    assert!(
        error.retryable(),
        "re-reading and retrying is exactly what resolves a conflict"
    );

    session.close().expect("the server exits cleanly");
    drop(home);
}

#[test]
fn a_read_only_layer_is_reported_as_such() {
    let (home, mut session) = default_home_session("config_layer_readonly");

    let error = enable_file_credential_store(&mut session, NOW)
        .expect_err("a read-only layer must be refused");

    assert_eq!(error.code(), ErrorCode::ConfigLayerReadonly);
    assert!(
        !error.retryable(),
        "retrying cannot make a read-only layer writable"
    );

    session.close().expect("the server exits cleanly");
    drop(home);
}

#[test]
fn a_runtime_that_does_not_know_the_key_is_reported_as_incompatible() {
    let (home, mut session) = default_home_session("config_unknown_key");

    let error = enable_file_credential_store(&mut session, NOW)
        .expect_err("an unknown key must stop the operation");

    assert_eq!(error.code(), ErrorCode::RuntimeIncompatible);

    session.close().expect("the server exits cleanly");
    drop(home);
}

#[test]
fn an_unmapped_server_error_is_not_dressed_up_as_something_understood() {
    let (home, mut session) = default_home_session("config_unmapped_error");

    let error = enable_file_credential_store(&mut session, NOW)
        .expect_err("an unrecognised refusal is still a refusal");

    assert_eq!(
        error.code(),
        ErrorCode::Internal,
        "a code Toglet has never seen must not be guessed into a specific meaning"
    );

    session.close().expect("the server exits cleanly");
    drop(home);
}

#[test]
fn a_write_that_a_higher_layer_overrides_is_a_failure_not_a_success() {
    let (home, mut session) = default_home_session("config_overridden");

    let error = enable_file_credential_store(&mut session, NOW)
        .expect_err("a value that has no effect must not be reported as applied");

    assert_eq!(error.code(), ErrorCode::ConfigLayerReadonly);

    session.close().expect("the server exits cleanly");
    drop(home);
}

#[test]
fn a_write_that_reports_success_without_taking_effect_is_caught_by_the_read_back() {
    // The server answers `status: ok` and the value stays what it was. Without the verifying
    // read this is the classic false success.
    let (home, mut session) = default_home_session("config_write_ineffective");

    let error = enable_file_credential_store(&mut session, NOW)
        .expect_err("a write that changed nothing must not be reported as applied");

    assert_eq!(error.code(), ErrorCode::ConfigConflict);

    session.close().expect("the server exits cleanly");
    drop(home);
}

#[test]
fn managing_the_setting_against_a_throwaway_home_is_refused() {
    // An isolated home would accept the write and change nothing the user can see.
    let home = scenario_home("config_absent", PHASE);
    let client = AppServerClient::start(&fake_binary(PHASE), home).expect("the fake server starts");
    let mut session = AppServerSession::open(client).expect("the handshake succeeds");

    let error = enable_file_credential_store(&mut session, NOW)
        .expect_err("a throwaway home must not be mistaken for the user's own");

    assert_eq!(error.code(), ErrorCode::Internal);
    session.close().expect("the server exits cleanly");
}

#[test]
fn a_previous_value_is_carried_out_so_it_can_be_restored() {
    let (home, mut session) = default_home_session("config_other_value");

    let outcome = enable_file_credential_store(&mut session, NOW).expect("the setting is applied");

    let CredentialStoreOutcome::Enabled(record) = outcome else {
        panic!("the value differed, so it had to be written");
    };
    assert_eq!(
        record.previous_value.as_deref(),
        Some("keychain"),
        "a restore cannot put back a value that was never carried out"
    );

    session.close().expect("the server exits cleanly");
    drop(home);
}

#[test]
fn a_configuration_that_cannot_be_parsed_stops_the_operation_and_names_the_file() {
    // Measured on the real server: a broken `config.toml` answers `-32603` with no `data`, so
    // nothing but the method distinguishes it from a genuine protocol complaint. Reporting it
    // as an incompatible runtime would send the user to update Codex over their own typo.
    let (home, mut session) = default_home_session("config_broken");

    let error = enable_file_credential_store(&mut session, NOW)
        .expect_err("an unparseable configuration must stop the operation");

    assert_eq!(error.code(), ErrorCode::ConfigSyntaxError);
    assert!(
        backups(home.path()).is_empty(),
        "nothing may be copied or written when the file cannot be read"
    );
    assert_eq!(
        std::fs::read(home.path().join("config.toml")).expect("readable"),
        EXISTING_CONFIG
    );

    session.close().expect("the server exits cleanly");
    drop(home);
}

/// The credential-store value the session currently reports, for asserting what a restore left.
fn current_value(session: &mut AppServerSession) -> Option<String> {
    session
        .read_credential_store_setting()
        .expect("the setting is readable")
        .value
}

#[test]
fn restoring_removes_a_key_that_did_not_exist_before() {
    let (home, mut session) = default_home_session("config_already_file");

    let outcome =
        restore_credential_store(&mut session, None, NOW).expect("the setting is put back");

    assert!(matches!(outcome, RestoreOutcome::Restored { .. }));
    assert_eq!(
        current_value(&mut session),
        None,
        "restoring an added key removes it rather than writing an empty value"
    );

    session.close().expect("the server exits cleanly");
    drop(home);
}

#[test]
fn restoring_puts_back_a_value_the_user_had_chosen() {
    let (home, mut session) = default_home_session("config_already_file");

    restore_credential_store(&mut session, Some("keyring"), NOW).expect("the setting is put back");

    assert_eq!(current_value(&mut session).as_deref(), Some("keyring"));

    session.close().expect("the server exits cleanly");
    drop(home);
}

#[test]
fn a_setting_changed_after_toglet_wrote_it_is_not_overwritten() {
    // The value now says `keychain`, which is not what Toglet wrote. Somebody chose that, and
    // putting the remembered value back would throw their choice away silently.
    let (home, mut session) = default_home_session("config_other_value");

    let error = restore_credential_store(&mut session, None, NOW)
        .expect_err("a value Toglet did not write must not be undone");

    assert_eq!(error.code(), ErrorCode::ConfigConflict);
    assert!(
        !error.retryable(),
        "retrying cannot resolve a deliberate change by someone else"
    );
    assert_eq!(
        current_value(&mut session).as_deref(),
        Some("keychain"),
        "the refusal must leave the setting exactly as it found it"
    );
    assert!(
        backups(home.path()).is_empty(),
        "refusing must happen before the configuration is copied"
    );

    session.close().expect("the server exits cleanly");
    drop(home);
}

#[test]
fn a_configuration_already_in_its_original_state_is_left_alone() {
    let (home, mut session) = default_home_session("config_absent");

    let outcome =
        restore_credential_store(&mut session, None, NOW).expect("nothing to do succeeds");

    assert_eq!(outcome, RestoreOutcome::AlreadyRestored);
    assert!(
        backups(home.path()).is_empty(),
        "a restore that changes nothing must not pile up copies"
    );
    assert_eq!(
        std::fs::read(home.path().join("config.toml")).expect("readable"),
        EXISTING_CONFIG
    );

    session.close().expect("the server exits cleanly");
    drop(home);
}

#[test]
fn an_organisation_enforced_configuration_stops_a_restore_too() {
    let (home, mut session) = default_home_session("config_org_enforced");

    let error = restore_credential_store(&mut session, None, NOW)
        .expect_err("an enforced configuration must stop the operation");

    assert_eq!(error.code(), ErrorCode::ConfigLayerReadonly);
    assert!(backups(home.path()).is_empty());

    session.close().expect("the server exits cleanly");
    drop(home);
}

#[test]
fn a_restore_through_a_layer_toglet_does_not_own_is_refused() {
    // The value is `file`, but an MDM layer supplies it. Toglet did not put it there and has no
    // business removing it.
    let (home, mut session) = default_home_session("config_managed_layer");

    let error = restore_credential_store(&mut session, None, NOW)
        .expect_err("a layer Toglet does not own must not be written through");

    assert_eq!(error.code(), ErrorCode::ConfigLayerReadonly);
    assert!(backups(home.path()).is_empty());

    session.close().expect("the server exits cleanly");
    drop(home);
}

#[test]
fn a_restore_that_reports_success_without_taking_effect_is_caught_by_the_read_back() {
    let (home, mut session) = default_home_session("config_restore_ineffective");

    let error = restore_credential_store(&mut session, None, NOW)
        .expect_err("a restore that changed nothing must not be reported as done");

    assert_eq!(error.code(), ErrorCode::ConfigConflict);

    session.close().expect("the server exits cleanly");
    drop(home);
}

#[test]
fn a_backup_is_taken_before_a_restore_changes_anything() {
    let (home, mut session) = default_home_session("config_already_file");

    let RestoreOutcome::Restored { backup } =
        restore_credential_store(&mut session, None, NOW).expect("the setting is put back")
    else {
        panic!("the value differed, so it had to be written");
    };

    let backup = backup.expect("a configuration existed, so it was copied");
    assert_eq!(
        std::fs::read(&backup).expect("readable"),
        EXISTING_CONFIG,
        "the copy must hold the state from before the restore"
    );

    session.close().expect("the server exits cleanly");
    drop(home);
}

#[test]
fn restoring_against_a_throwaway_home_is_refused() {
    let home = scenario_home("config_already_file", PHASE);
    let client = AppServerClient::start(&fake_binary(PHASE), home).expect("the fake server starts");
    let mut session = AppServerSession::open(client).expect("the handshake succeeds");

    let error = restore_credential_store(&mut session, None, NOW)
        .expect_err("a throwaway home must not be mistaken for the user's own");

    assert_eq!(error.code(), ErrorCode::Internal);
    session.close().expect("the server exits cleanly");
}

#[test]
fn neither_managing_nor_restoring_touches_the_authentication_file() {
    // The credentials Codex is using are not Toglet's to remove. Changing a
    // configuration key must leave them exactly where they are.
    let (home, mut session) = default_home_session("config_absent");
    let auth = home.path().join("auth.json");
    let contents = br#"{"tokens":{"access_token":"not-a-real-token"}}"#;
    std::fs::write(&auth, contents).expect("the authentication file is written");

    enable_file_credential_store(&mut session, NOW).expect("the setting is applied");
    restore_credential_store(&mut session, None, NOW).expect("the setting is put back");

    assert_eq!(
        std::fs::read(&auth).expect("readable"),
        contents,
        "the authentication file must survive both operations untouched"
    );

    session.close().expect("the server exits cleanly");
    drop(home);
}

#[test]
fn what_managing_recorded_survives_a_restart_and_drives_the_restore() {
    // The record has to outlive the process: a user enables management today and stops it after
    // a reboot. This runs the value through the real metadata document rather than holding it in
    // memory, so the schema that carries it is exercised too.
    let (home, mut session) = default_home_session("config_other_value");
    let store = MetadataStore::new(home.path());

    let CredentialStoreOutcome::Enabled(record) =
        enable_file_credential_store(&mut session, NOW).expect("the setting is applied")
    else {
        panic!("the value differed, so it had to be written");
    };
    store
        .save(&MetadataDocument {
            codex_config: CodexConfigState::Managed {
                previous_value: record.previous_value,
            },
            ..MetadataDocument::default()
        })
        .expect("the document is written");

    let (reloaded, _) = store.load();
    let CodexConfigState::Managed { previous_value } = reloaded.codex_config else {
        panic!("the document must remember that Toglet changed the configuration");
    };
    restore_credential_store(&mut session, previous_value.as_deref(), NOW)
        .expect("the setting is put back");

    assert_eq!(
        current_value(&mut session).as_deref(),
        Some("keychain"),
        "the value the user had before management started must come back"
    );

    session.close().expect("the server exits cleanly");
    drop(home);
}

#[test]
fn a_failure_over_a_broken_configuration_never_carries_the_file_path() {
    let (home, mut session) = default_home_session("config_broken");

    let error = enable_file_credential_store(&mut session, NOW).expect_err("it fails");

    // The real server's message embeds the absolute path of the configuration file.
    let rendered = format!("{error:?}");
    for fragment in ["config.toml", "/fake/", "unclosed table"] {
        assert!(
            !rendered.contains(fragment),
            "`{fragment}` from the server's message must not survive into a Toglet error"
        );
    }

    session.close().expect("the server exits cleanly");
    drop(home);
}
