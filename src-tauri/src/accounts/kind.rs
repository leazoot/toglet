//! Recognising account types Toglet cannot manage.
//!
//! An unmanageable account is reported honestly and kept out of the switch list. It is never
//! given a fabricated quota: an account outside the five-hour/weekly system has **unknown**
//! quota, which is `null`, not `0`.

use super::status::AccountStatus;

/// What kind of sign-in a Codex home holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountKind {
    /// The only kind Toglet manages.
    Chatgpt,
    /// An API key sign-in. Recognised, explained, not managed.
    ApiKey,
    /// `chatgptAuthTokens`: tokens supplied by a host application and held in memory. The app
    /// server schema marks it "FOR OPENAI INTERNAL USE ONLY". Toglet cannot store or replace
    /// something it never sees on disk.
    HostManagedTokens,
    /// A mode this build does not know. Reported as-is rather than forced into one of the
    /// above - guessing here would mean claiming to manage something unknown.
    Unknown(String),
}

/// Why an account cannot be managed.
///
/// A stable code, not a sentence. The Rust layer never produces user-facing prose; the
/// frontend maps the code to localised copy. Each variant names both halves of the answer the
/// user needs: what the account is, and therefore why the five-hour and weekly quota view does
/// not apply to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsupportedReason {
    /// Signed in with an API key, which does not use the five-hour and weekly quota system.
    ApiKeyHasNoQuotaWindows,
    /// Credentials are held by a host application, so there is nothing on disk to switch.
    TokensHeldByHostApplication,
    /// An authentication mode this build does not recognise.
    UnrecognisedAuthMode,
}

impl UnsupportedReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ApiKeyHasNoQuotaWindows => "api_key_has_no_quota_windows",
            Self::TokensHeldByHostApplication => "tokens_held_by_host_application",
            Self::UnrecognisedAuthMode => "unrecognised_auth_mode",
        }
    }
}

impl AccountKind {
    /// Reads the `auth_mode` field of an `auth.json`.
    ///
    /// Absent means nobody is signed in, which is not a "kind" at all - the caller handles that
    /// before asking.
    pub fn from_auth_mode(mode: &str) -> Self {
        match mode {
            "chatgpt" => Self::Chatgpt,
            "apikey" => Self::ApiKey,
            "chatgptAuthTokens" => Self::HostManagedTokens,
            other => Self::Unknown(other.to_owned()),
        }
    }

    pub fn is_manageable(&self) -> bool {
        matches!(self, Self::Chatgpt)
    }

    /// `None` for a manageable account.
    pub fn unsupported_reason(&self) -> Option<UnsupportedReason> {
        match self {
            Self::Chatgpt => None,
            Self::ApiKey => Some(UnsupportedReason::ApiKeyHasNoQuotaWindows),
            Self::HostManagedTokens => Some(UnsupportedReason::TokensHeldByHostApplication),
            Self::Unknown(_) => Some(UnsupportedReason::UnrecognisedAuthMode),
        }
    }

    /// The status such an account is recorded with.
    pub fn status(&self) -> AccountStatus {
        if self.is_manageable() {
            AccountStatus::Ready
        } else {
            AccountStatus::Unsupported
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_chatgpt_sign_in_is_manageable() {
        assert!(AccountKind::from_auth_mode("chatgpt").is_manageable());
        for other in ["apikey", "chatgptAuthTokens", "somethingNew"] {
            assert!(
                !AccountKind::from_auth_mode(other).is_manageable(),
                "{other} must not be manageable"
            );
        }
    }

    #[test]
    fn an_unmanageable_account_is_recorded_as_unsupported_not_as_an_error() {
        // `error` would suggest something went wrong. Nothing did: the account simply is not
        // one Toglet manages, and the panel says so.
        assert_eq!(
            AccountKind::from_auth_mode("apikey").status(),
            AccountStatus::Unsupported
        );
        assert!(!AccountStatus::Unsupported.may_start_switch());
    }

    #[test]
    fn every_unmanageable_kind_carries_a_distinct_reason() {
        let reasons: Vec<&str> = ["apikey", "chatgptAuthTokens", "whatever"]
            .into_iter()
            .map(|mode| {
                AccountKind::from_auth_mode(mode)
                    .unsupported_reason()
                    .expect("unmanageable kinds carry a reason")
                    .as_str()
            })
            .collect();

        let unique: std::collections::BTreeSet<_> = reasons.iter().collect();
        assert_eq!(
            unique.len(),
            reasons.len(),
            "reasons must be distinguishable"
        );
    }

    #[test]
    fn a_manageable_account_has_no_reason_to_explain() {
        assert_eq!(
            AccountKind::from_auth_mode("chatgpt").unsupported_reason(),
            None
        );
        assert_eq!(
            AccountKind::from_auth_mode("chatgpt").status(),
            AccountStatus::Ready
        );
    }

    #[test]
    fn an_unknown_mode_keeps_its_name_rather_than_being_folded_into_a_known_one() {
        assert_eq!(
            AccountKind::from_auth_mode("futureMode"),
            AccountKind::Unknown("futureMode".to_owned())
        );
    }
}
