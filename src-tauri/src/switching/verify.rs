//! Asks Codex who the default home is signed in as.
//! Nothing may report a switch done or write `activeAccountId` until this names the target.
//! Compared by e-mail: corrupt credentials and a signed-out home both return `account: null`.

use std::path::Path;

use crate::accounts::AccountIdentity;
use crate::app_server::{AppServerClient, AppServerSession, CodexBinary};
use crate::codex_home::ServerHome;
use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};

/// Reads the default home's identity through a short-lived app server, closed on every path.
pub fn read_default_identity(
    binary: &CodexBinary,
    default_home: &Path,
    phase: Phase,
) -> Result<Option<AccountIdentity>> {
    let home = ServerHome::Default {
        path: default_home.to_path_buf(),
        phase,
    };
    let mut session = AppServerSession::open(AppServerClient::start(binary, home)?)?;
    let identity = session.read_account();
    let closed = session.close();

    // A failed shutdown must not override a correct read, but is not swallowed either.
    let identity = identity?;
    closed?;
    Ok(identity)
}

/// Whether the home is signed in as the switch target. `None` on either side never matches.
pub fn is_target(actual: Option<&AccountIdentity>, expected: &AccountIdentity) -> bool {
    match (actual.and_then(AccountIdentity::email), expected.email()) {
        (Some(actual), Some(expected)) => actual.eq_ignore_ascii_case(expected),
        _ => false,
    }
}

/// Whether two readings name the same account, where nobody matches nobody (after a rollback).
pub fn is_same(actual: Option<&AccountIdentity>, expected: Option<&AccountIdentity>) -> bool {
    match (actual, expected) {
        (None, None) => true,
        (actual, Some(expected)) => is_target(actual, expected),
        (Some(_), None) => false,
    }
}

pub fn mismatch(phase: Phase) -> TogletError {
    TogletError::new(
        ErrorCode::SwitchVerificationMismatch,
        phase,
        false,
        UserAction::RestoreFromBackup,
    )
    .with_detail("the default home is not signed in as the switch target")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chatgpt(email: &str) -> AccountIdentity {
        AccountIdentity::Chatgpt {
            email: email.to_owned(),
            plan_type: None,
        }
    }

    #[test]
    fn the_same_account_verifies() {
        let expected = chatgpt("someone@example.com");

        assert!(is_target(Some(&chatgpt("someone@example.com")), &expected));
    }

    #[test]
    fn a_different_account_does_not_verify() {
        let expected = chatgpt("someone@example.com");

        assert!(!is_target(Some(&chatgpt("other@example.com")), &expected));
    }

    #[test]
    fn a_home_that_names_nobody_does_not_verify() {
        // `account: null` is what a corrupted credential file looks like as well as a
        // signed-out home. Treating it as agreement would confirm a switch that never happened.
        assert!(!is_target(None, &chatgpt("someone@example.com")));
    }

    #[test]
    fn an_api_key_home_does_not_verify_against_a_chatgpt_target() {
        assert!(!is_target(
            Some(&AccountIdentity::ApiKey),
            &chatgpt("someone@example.com")
        ));
    }

    #[test]
    fn a_chatgpt_home_does_not_verify_against_an_api_key_target() {
        assert!(!is_target(
            Some(&chatgpt("someone@example.com")),
            &AccountIdentity::ApiKey
        ));
    }

    #[test]
    fn a_home_that_held_nobody_and_holds_nobody_again_is_correctly_restored() {
        assert!(is_same(None, None));
    }

    #[test]
    fn a_restored_home_that_now_holds_somebody_else_is_not_the_previous_state() {
        assert!(!is_same(Some(&chatgpt("someone@example.com")), None));
        assert!(!is_same(None, Some(&chatgpt("someone@example.com"))));
    }

    #[test]
    fn addresses_differing_only_in_case_are_the_same_account() {
        let expected = chatgpt("Someone@Example.com");

        assert!(is_target(Some(&chatgpt("someone@example.com")), &expected));
    }
}
