//! Asking Codex itself who the default home is now signed in as.
//!
//! Everything turns on this check: nothing may report a switch as done, and nothing may write
//! `activeAccountId`, until a fresh app server started against the **default** home says the
//! identity is the target's.
//!
//! Comparison is by e-mail address. A corrupted credential file and a signed-out home both
//! answer `account: null`, so "no answer" can never be read as agreement.

use std::path::Path;

use crate::accounts::AccountIdentity;
use crate::app_server::{AppServerClient, AppServerSession, CodexBinary};
use crate::codex_home::ServerHome;
use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};

/// Reads the identity of the default Codex home through a short-lived app server.
///
/// The server is closed on every path, including the one where reading fails: a verification
/// that leaves a process behind would trade one guarantee for another.
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

    // The read is the answer; a shutdown that misbehaved must not turn a correct verification
    // into a failure, but it must not be swallowed either.
    let identity = identity?;
    closed?;
    Ok(identity)
}

/// Whether the home is signed in as the account the switch aimed at.
///
/// Both sides must name somebody. `None` on either side is a disagreement, never a match.
pub fn is_target(actual: Option<&AccountIdentity>, expected: &AccountIdentity) -> bool {
    match (actual.and_then(AccountIdentity::email), expected.email()) {
        (Some(actual), Some(expected)) => actual.eq_ignore_ascii_case(expected),
        _ => false,
    }
}

/// Whether two readings name the same account, where "nobody" is a legitimate answer.
///
/// Used after a rollback: if the home held nobody before the switch, holding nobody again is a
/// correct restoration, while [`is_target`] would call it a failure.
pub fn is_same(actual: Option<&AccountIdentity>, expected: Option<&AccountIdentity>) -> bool {
    match (actual, expected) {
        (None, None) => true,
        (actual, Some(expected)) => is_target(actual, expected),
        (Some(_), None) => false,
    }
}

/// The error a mismatch produces.
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
