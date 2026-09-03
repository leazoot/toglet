//! The four account onboarding paths, driven end to end against the fake app server.
//!
//! No real account, no network, no platform credential store. What these cannot cover is a
//! human completing OAuth in a browser and a second real account being added; those two are
//! recorded as outstanding manual verification.

mod support;

use std::time::Duration;

use support::{fake_binary, scenario_home};
use toglet_lib::accounts::onboarding::{
    self, LoginOutcome, PendingLogin, VerifiedCredentials, adopt, forget, reauthenticate, verify,
};
use toglet_lib::accounts::{AccountKind, AccountStatus, fingerprint::DuplicateCheck};
use toglet_lib::credentials::{CredentialRef, MemorySecretStore, Secret, SecretStore};
use toglet_lib::diagnostics::{ErrorCode, Phase, UserAction};
use toglet_lib::storage::MetadataDocument;
use toglet_lib::switching::adopt_current_session;

const NOW: &str = "2026-08-31T00:00:00Z";

fn auth_json(account_id: &str, mode: &str) -> Secret {
    Secret::new(
        format!(
            r#"{{"OPENAI_API_KEY":null,"auth_mode":"{mode}","last_refresh":"{NOW}",
               "tokens":{{"access_token":"eyJhbGciOiJIUzI1NiJ9.aaaaaaaaaaaaaaaaaaaaaa",
               "account_id":"{account_id}","id_token":"eyJhbGciOiJIUzI1NiJ9.bbbbbbbbbbbbbbbbbb",
               "refresh_token":"rt-cccccccccccccccccccccccc"}}}}"#
        )
        .into_bytes(),
    )
}

/// Verifies credentials against the fake server running `scenario`.
fn verify_with(
    scenario: &str,
    secret: Secret,
) -> toglet_lib::diagnostics::Result<VerifiedCredentials> {
    verify(
        &fake_binary(Phase::Login),
        scenario_home(scenario, Phase::Login),
        secret,
        Phase::Login,
    )
}

fn start_login(scenario: &str) -> PendingLogin {
    PendingLogin::start(
        &fake_binary(Phase::Login),
        scenario_home(scenario, Phase::Login),
        Phase::Login,
    )
    .expect("the sign-in starts")
}

#[test]
fn importing_the_current_account_stores_it_and_reports_who_it_is() {
    let store = MemorySecretStore::new();
    let mut document = MetadataDocument::default();
    let verified = verify_with("normal", auth_json("id-1", "chatgpt")).expect("verified");

    assert_eq!(
        verified.masked_email().as_deref(),
        Some("tes***@example.com")
    );
    assert_eq!(verified.kind(), &AccountKind::Chatgpt);

    let outcome = adopt(
        &store,
        &mut document,
        &verified,
        Some("Work"),
        "acct-1",
        NOW,
    )
    .expect("adopted");

    assert_eq!(outcome, DuplicateCheck::New);
    assert_eq!(document.accounts.len(), 1);
    assert_eq!(document.accounts[0].status, AccountStatus::Ready);
    // The full address never reaches the stored profile.
    assert_eq!(
        document.accounts[0].masked_email.as_deref(),
        Some("tes***@example.com")
    );
}

#[test]
fn stored_credentials_can_be_decrypted_and_used_again() {
    let store = MemorySecretStore::new();
    let mut document = MetadataDocument::default();
    let original = auth_json("id-1", "chatgpt");
    let expected = original.expose().to_vec();
    let verified = verify_with("normal", original).expect("verified");
    adopt(
        &store,
        &mut document,
        &verified,
        Some("Work"),
        "acct-1",
        NOW,
    )
    .expect("adopted");

    let reference = CredentialRef::new(&document.accounts[0].credential_ref).expect("valid");
    let loaded = store.load(&reference).expect("stored credentials load");

    assert_eq!(
        loaded.expose(),
        expected,
        "the round trip must be byte exact"
    );
    // And they still verify, which is the property that matters: a stored snapshot has to be
    // usable, not merely present.
    let reverified = verify_with("normal", loaded).expect("verified again");
    assert_eq!(reverified.fingerprint(), verified.fingerprint());
}

#[test]
fn adding_the_same_account_twice_offers_the_existing_one_instead_of_a_second_profile() {
    let store = MemorySecretStore::new();
    let mut document = MetadataDocument::default();
    let first = verify_with("normal", auth_json("id-1", "chatgpt")).expect("verified");
    adopt(&store, &mut document, &first, Some("Work"), "acct-1", NOW).expect("adopted");

    let again = verify_with("normal", auth_json("id-1", "chatgpt")).expect("verified");
    let outcome = adopt(
        &store,
        &mut document,
        &again,
        Some("Work again"),
        "acct-2",
        NOW,
    )
    .expect("checked");

    assert_eq!(
        outcome,
        DuplicateCheck::AlreadyPresent {
            existing_id: "acct-1".to_owned()
        }
    );
    assert_eq!(document.accounts.len(), 1);
    // Nothing was stored for the rejected attempt.
    assert!(
        !store
            .contains(&CredentialRef::new("cred-acct-2").expect("valid"))
            .expect("checkable")
    );
}

#[test]
fn two_different_accounts_both_get_a_profile() {
    let store = MemorySecretStore::new();
    let mut document = MetadataDocument::default();

    let first = verify_with("normal", auth_json("id-1", "chatgpt")).expect("verified");
    adopt(&store, &mut document, &first, Some("Work"), "acct-1", NOW).expect("adopted");
    let second = verify_with("second_account", auth_json("id-2", "chatgpt")).expect("verified");
    adopt(
        &store,
        &mut document,
        &second,
        Some("Personal"),
        "acct-2",
        NOW,
    )
    .expect("adopted");

    assert_eq!(document.accounts.len(), 2);
    assert_ne!(
        document.accounts[0].account_fingerprint,
        document.accounts[1].account_fingerprint
    );
}

#[test]
fn an_api_key_account_is_recorded_as_unsupported_with_a_reason_and_no_quota() {
    let verified = verify_with("api_key_account", auth_json("id-1", "apikey")).expect("verified");

    assert_eq!(verified.kind(), &AccountKind::ApiKey);
    assert_eq!(verified.kind().status(), AccountStatus::Unsupported);
    assert!(verified.kind().unsupported_reason().is_some());

    let store = MemorySecretStore::new();
    let mut document = MetadataDocument::default();
    adopt(&store, &mut document, &verified, Some("API"), "acct-1", NOW).expect("adopted");

    // Unknown quota is absent, not zero, and the account cannot be switched to.
    assert_eq!(document.accounts[0].plan_type, None);
    assert_eq!(document.accounts[0].status, AccountStatus::Unsupported);
    assert!(!document.accounts[0].status.may_start_switch());
}

#[test]
fn credentials_that_identify_nobody_are_refused_rather_than_stored() {
    // Matched rather than `expect_err`: `VerifiedCredentials` owns decrypted material, and a
    // `Debug` on such a type is a hazard worth not having.
    let error = match verify_with("signed_out", auth_json("id-1", "chatgpt")) {
        Ok(_) => panic!("credentials that identify nobody must not verify"),
        Err(error) => error,
    };

    // A corrupted file looks identical to a signed-out one here, so both are failures.
    assert_eq!(error.code(), ErrorCode::AuthExpired);
}

#[test]
fn a_credential_file_that_is_not_json_is_refused_before_a_subprocess_starts() {
    let error = match verify(
        &fake_binary(Phase::Login),
        scenario_home("normal", Phase::Login),
        Secret::new(b"not json".to_vec()),
        Phase::Login,
    ) {
        Ok(_) => panic!("garbage must not verify"),
        Err(error) => error,
    };

    assert_eq!(error.code(), ErrorCode::AuthFileConflict);
}

#[test]
fn re_authentication_replaces_the_snapshot_only_for_the_same_account() {
    let store = MemorySecretStore::new();
    let mut document = MetadataDocument::default();
    let original = verify_with("normal", auth_json("id-1", "chatgpt")).expect("verified");
    adopt(
        &store,
        &mut document,
        &original,
        Some("Work"),
        "acct-1",
        NOW,
    )
    .expect("adopted");
    let reference = CredentialRef::new("cred-acct-1").expect("valid");
    let before = store.load(&reference).expect("stored").expose().to_vec();

    // Somebody else's credentials must not be accepted for this profile.
    let stranger = verify_with("second_account", auth_json("id-2", "chatgpt")).expect("verified");
    let error = reauthenticate(&store, &mut document, "acct-1", &stranger, NOW)
        .expect_err("a different account must be refused");

    assert_eq!(error.code(), ErrorCode::SwitchVerificationMismatch);
    assert_eq!(
        store.load(&reference).expect("stored").expose(),
        before,
        "the old snapshot must survive a refused re-authentication"
    );
}

#[test]
fn re_authentication_with_the_same_account_updates_the_snapshot() {
    let store = MemorySecretStore::new();
    let mut document = MetadataDocument::default();
    let original = verify_with("normal", auth_json("id-1", "chatgpt")).expect("verified");
    adopt(
        &store,
        &mut document,
        &original,
        Some("Work"),
        "acct-1",
        NOW,
    )
    .expect("adopted");

    // Same account id, different token material - what a refreshed sign-in produces.
    let mut refreshed_bytes = auth_json("id-1", "chatgpt").expose().to_vec();
    refreshed_bytes = String::from_utf8(refreshed_bytes)
        .expect("utf8")
        .replace("rt-cccccccccccccccccccccccc", "rt-dddddddddddddddddddddddd")
        .into_bytes();
    let refreshed = verify_with("normal", Secret::new(refreshed_bytes)).expect("verified");

    reauthenticate(
        &store,
        &mut document,
        "acct-1",
        &refreshed,
        "2026-09-01T00:00:00Z",
    )
    .expect("same account is accepted");

    let stored = store
        .load(&CredentialRef::new("cred-acct-1").expect("valid"))
        .expect("stored");
    assert!(
        String::from_utf8_lossy(stored.expose()).contains("rt-dddd"),
        "the refreshed snapshot was not stored"
    );
    assert_eq!(
        document.accounts[0].last_validated_at.as_deref(),
        Some("2026-09-01T00:00:00Z")
    );
}

#[test]
fn removing_an_account_deletes_its_credentials_too() {
    let store = MemorySecretStore::new();
    let mut document = MetadataDocument::default();
    let verified = verify_with("normal", auth_json("id-1", "chatgpt")).expect("verified");
    adopt(
        &store,
        &mut document,
        &verified,
        Some("Work"),
        "acct-1",
        NOW,
    )
    .expect("adopted");
    let reference = CredentialRef::new("cred-acct-1").expect("valid");

    forget(&store, &mut document, "acct-1").expect("removed");

    assert!(document.accounts.is_empty());
    assert!(
        !store.contains(&reference).expect("checkable"),
        "a removed account must not leave recoverable credentials behind"
    );
}

#[test]
fn a_completed_sign_in_is_reported_as_completed() {
    let mut login = start_login("login_success");

    assert!(login.auth_url().starts_with("https://"));
    let outcome = login.wait(Duration::from_secs(5));

    assert_eq!(outcome, LoginOutcome::Completed);
    login.finish().expect("the server exits cleanly");
}

#[test]
fn a_cancelled_sign_in_is_not_reported_as_a_failure() {
    let mut login = start_login("login_cancel");

    login.cancel().expect("cancellation is accepted");
    let outcome = login.wait(Duration::from_secs(5));

    // The server reports `success: false` for both. Only the local record of having asked
    // tells them apart.
    assert_eq!(outcome, LoginOutcome::Canceled);
    login.finish().expect("the server exits cleanly");
}

#[test]
fn a_failed_sign_in_that_was_not_cancelled_is_reported_as_a_failure() {
    let mut login = start_login("login_failure");

    let outcome = login.wait(Duration::from_secs(5));

    assert_eq!(outcome, LoginOutcome::Failed);
    login.finish().expect("the server exits cleanly");
}

#[test]
fn a_sign_in_nobody_finishes_times_out_rather_than_waiting_forever() {
    let mut login = start_login("login_pending");

    let outcome = login.wait(Duration::from_millis(500));

    assert_eq!(outcome, LoginOutcome::TimedOut);
    login.finish().expect("the server exits cleanly");
}

#[test]
fn the_account_the_default_home_already_holds_can_be_recorded_as_the_active_one() {
    // The gap the real-machine acceptance found: without this, `activeAccountId` stays null
    // after an import, every session in the default home reads as a stranger's, and the
    // pre-checks refuse the first switch.
    let verified = verify_with("normal", auth_json("id-1", "chatgpt")).expect("verified");
    // "normal" is also what this home answers, so it is signed in as the same account.
    let default_home = scenario_home("normal", Phase::Verify);
    let mut document = MetadataDocument::default();
    let store = MemorySecretStore::new();
    adopt(
        &store,
        &mut document,
        &verified,
        Some("Work"),
        "acct-1",
        NOW,
    )
    .expect("adopted");

    let token = adopt_current_session(&fake_binary(Phase::Verify), default_home.path(), &verified)
        .expect("the default home is signed in as this account");
    document
        .settings
        .set_active_account_id(Some("acct-1".to_owned()), &token);

    assert_eq!(document.settings.active_account_id(), Some("acct-1"));
}

#[test]
fn an_account_the_default_home_is_not_signed_in_as_cannot_be_recorded_as_active() {
    let verified = verify_with("normal", auth_json("id-1", "chatgpt")).expect("verified");
    // This home reports `other@example.com`, so the candidate is not the one in use.
    let default_home = scenario_home("second_account", Phase::Verify);

    let error = adopt_current_session(&fake_binary(Phase::Verify), default_home.path(), &verified)
        .expect_err("a different account must not be adopted");

    assert_eq!(error.code(), ErrorCode::SwitchVerificationMismatch);
    // Nothing was replaced, so telling the user to restore a backup would be wrong.
    assert_eq!(error.action(), UserAction::ResolveExternalChange);
}

#[test]
fn a_signed_out_default_home_cannot_have_an_account_recorded_as_active() {
    // `account: null` is what a corrupted credential file looks like too, so it can never be
    // read as agreement.
    let verified = verify_with("normal", auth_json("id-1", "chatgpt")).expect("verified");
    let default_home = scenario_home("signed_out", Phase::Verify);

    let error = adopt_current_session(&fake_binary(Phase::Verify), default_home.path(), &verified)
        .expect_err("a home signed in as nobody must not be adopted");

    assert_eq!(error.code(), ErrorCode::SwitchVerificationMismatch);
}

#[test]
fn a_sign_in_leaves_no_isolated_home_behind() {
    let home = scenario_home("login_pending", Phase::Login);
    let path = home.path().to_path_buf();
    let login = PendingLogin::start(&fake_binary(Phase::Login), home, Phase::Login)
        .expect("the sign-in starts");

    // Dropped without `finish`, which is what an abandoned dialog looks like.
    drop(login);

    assert!(!path.exists(), "an abandoned sign-in left its home behind");
    // A sign-in deadline shorter than a minute would fire while somebody is still typing.
    assert!(onboarding::LOGIN_TIMEOUT > Duration::from_secs(60));
}

/// Base64url of `{"sub":"user-1","email":"leanne@example.com","name":"Leanne Q"}`.
const NAMED_ID_TOKEN: &str = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ1c2VyLTEiLCJlbWFpbCI6ImxlYW5uZUBleGFtcGxlLmNvbSIsIm5hbWUiOiJMZWFubmUgUSJ9.sig";

fn auth_json_with_id_token(account_id: &str, id_token: &str) -> Secret {
    Secret::new(
        format!(
            r#"{{"OPENAI_API_KEY":null,"auth_mode":"chatgpt","last_refresh":"{NOW}",
               "tokens":{{"access_token":"eyJhbGciOiJIUzI1NiJ9.aaaaaaaaaaaaaaaaaaaaaa",
               "account_id":"{account_id}","id_token":"{id_token}",
               "refresh_token":"rt-cccccccccccccccccccccccc"}}}}"#
        )
        .into_bytes(),
    )
}

#[test]
fn an_account_added_without_a_name_is_named_after_itself() {
    // The name the account carries at ChatGPT, read from the id token.
    let store = MemorySecretStore::new();
    let mut document = MetadataDocument::default();
    let verified =
        verify_with("normal", auth_json_with_id_token("id-1", NAMED_ID_TOKEN)).expect("verified");

    adopt(&store, &mut document, &verified, None, "acct-1", NOW).expect("adopted");

    assert_eq!(document.accounts[0].display_name, "Leanne Q");
}

#[test]
fn an_account_whose_token_carries_no_name_is_named_after_its_address() {
    // The fake server reports `tester@example.com`; the local part is the fallback.
    let store = MemorySecretStore::new();
    let mut document = MetadataDocument::default();
    let verified = verify_with("normal", auth_json("id-1", "chatgpt")).expect("verified");

    adopt(&store, &mut document, &verified, None, "acct-1", NOW).expect("adopted");

    assert_eq!(document.accounts[0].display_name, "tester");
}

#[test]
fn a_typed_name_still_wins_over_the_default() {
    let store = MemorySecretStore::new();
    let mut document = MetadataDocument::default();
    let verified =
        verify_with("normal", auth_json_with_id_token("id-1", NAMED_ID_TOKEN)).expect("verified");

    adopt(
        &store,
        &mut document,
        &verified,
        Some("Work"),
        "acct-1",
        NOW,
    )
    .expect("adopted");

    assert_eq!(document.accounts[0].display_name, "Work");
}
